//! Prompts and answer parsers for the editor's "Draft with AI" helpers (scenario, greetings,
//! lorebook entries, weekly schedule). Answers use plain line formats rather than JSON: small local
//! models keep to them far more reliably, and the parsers below are lenient about fences, bullets
//! and numbering.

use serde::Deserialize;

use crate::models::{Character, DayKind, DayPlan, Schedule, ScheduleBlock, WorldInfoEntry};

/// What the lorebook editor sends when asking for entries.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoreDraftRequest {
    pub lorebook_name: String,
    pub topic: String,
    pub count: usize,
    /// Keys already in the lorebook; drafts repeating them are dropped.
    #[serde(default)]
    pub existing_keys: Vec<String>,
    /// A character whose card the entries should fit.
    #[serde(default)]
    pub character: Option<Character>,
}

/// The parts of a card the model needs, keeping `{{char}}` / `{{user}}` as written.
pub fn card_context(card: &Character) -> String {
    let mut parts = vec![format!("Name: {}", card.name.trim())];
    let mut add = |label: &str, text: &str| {
        if !text.trim().is_empty() {
            parts.push(format!("{label}: {}", text.trim()));
        }
    };
    add("Description", &card.description);
    add("Personality", &card.personality);
    add("Scenario", &card.scenario);
    if let Some(first) = card.first_messages.iter().find(|m| !m.trim().is_empty()) {
        add("Current first message", first);
    }
    parts.join("\n")
}

fn idea_part(idea: &str) -> String {
    match idea.trim() {
        "" => String::new(),
        idea => format!("\n\nThe author's idea for it:\n{idea}"),
    }
}

pub fn scenario_prompt(card: &Character, idea: &str) -> String {
    let task = if card.scenario.trim().is_empty() { "Write" } else { "Rewrite and improve" };
    format!(
        "{task} the Scenario field of this roleplay character card: the situation and setting {{{{char}}}} and \
{{{{user}}}} are in when the roleplay starts - where they are, how they know each other, what is going on. \
2-4 sentences, present tense, no dialogue. Refer to the two of them only as {{{{char}}}} and {{{{user}}}}. \
Reply with the scenario text only.\n\nCharacter card:\n{}{}",
        card_context(card),
        idea_part(idea)
    )
}

pub fn greetings_prompt(card: &Character, idea: &str, count: usize) -> String {
    format!(
        "Write {count} different opening messages for this roleplay character card - the first message of a new \
chat, written by {{{{char}}}}. Roleplay style: narration and *actions in asterisks*, spoken words in \"quotes\", \
1-3 paragraphs each. Set the scene from the scenario and give {{{{user}}}} something to react to, but never write \
{{{{user}}}}'s actions, words or thoughts. Use {{{{char}}}} and {{{{user}}}} instead of names. Make each one start \
differently. Separate the messages with a line containing only ---. No titles or numbering.\n\n\
Character card:\n{}{}",
        card_context(card),
        idea_part(idea)
    )
}

pub fn lore_prompt(request: &LoreDraftRequest) -> String {
    let mut prompt = format!(
        "Create {} World Info entries for the roleplay lorebook \"{}\" about: {}\n\n\
Each entry describes one thing (a place, person, faction, item, custom or event) in 2-5 factual sentences \
written as reference notes, not story. Its keys are 1-4 short words or names that would appear in a chat \
message whenever the entry is relevant. Use exactly this format, with a line containing only --- between entries:\n\
KEYS: first key, second key\nCONTENT: The reference text.\n---",
        request.count.clamp(1, 10),
        request.lorebook_name.trim(),
        request.topic.trim()
    );
    let existing: Vec<&str> = request.existing_keys.iter().map(|k| k.trim()).filter(|k| !k.is_empty()).collect();
    if !existing.is_empty() {
        prompt.push_str(&format!("\n\nThe lorebook already covers these keys; don't repeat them: {}", existing.join(", ")));
    }
    if let Some(card) = &request.character {
        prompt.push_str(&format!("\n\nThe entries belong to this character's world:\n{}", card_context(card)));
    }
    prompt
}

pub fn schedule_prompt(card: &Character, idea: &str) -> String {
    format!(
        "Write a typical weekly schedule for this roleplay character, as it fits their job, school, age and habits. \
Use exactly one line per day, Monday to Sunday, plus one line for public holidays, in this format:\n\
MON: work | 07:00-07:45 commute; 08:00-16:00 work @ the bakery; 18:00-19:30 gym\n\
The word after the colon is the kind of day: work, school, shift or off. Times are 24-hour HH:MM; a block may \
run past midnight (22:00-06:00). Add \"@ place\" when the place matters. Leave out sleep. Lines start with MON, \
TUE, WED, THU, FRI, SAT, SUN and HOLIDAY. Reply with the eight lines only.\n\nCharacter card:\n{}{}",
        card_context(card),
        idea_part(idea)
    )
}

// ---- parsing ----

/// Drops code-fence lines and surrounding whitespace.
fn unfence(text: &str) -> String {
    text.lines().filter(|l| !l.trim_start().starts_with("```")).collect::<Vec<_>>().join("\n").trim().to_string()
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.len() >= 3 && (t.chars().all(|c| c == '-') || t.chars().all(|c| c == '*') || t.chars().all(|c| c == '='))
}

/// Splits at separator lines, dropping empty pieces.
fn pieces(text: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    for line in unfence(text).lines() {
        if is_separator(line) {
            out.push(String::new());
        } else {
            let current = out.last_mut().unwrap();
            current.push_str(line);
            current.push('\n');
        }
    }
    out.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// Strips a leading "Greeting 2:", "2." or "**Option 1**" label line or prefix.
fn strip_label(text: &str) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    let label = |l: &str| {
        let t = l.trim().trim_matches('*').trim().trim_end_matches(':').to_ascii_lowercase();
        let words: Vec<&str> = t.split_whitespace().collect();
        matches!(words.as_slice(), [w, n] if ["greeting", "message", "option", "version"].contains(w) && n.trim_end_matches(['.', ')']).parse::<u32>().is_ok())
    };
    if lines.first().is_some_and(|l| label(l)) {
        lines.remove(0);
    }
    let joined = lines.join("\n");
    let trimmed = joined.trim_start();
    // "1. text" / "2) text"
    let digits = trimmed.chars().take_while(|c| c.is_ascii_digit()).count();
    if digits > 0 && digits <= 2 {
        if let Some(rest) = trimmed[digits..].strip_prefix(['.', ')']) {
            return rest.trim().to_string();
        }
    }
    trimmed.trim().to_string()
}

pub fn parse_text(answer: &str) -> String {
    unfence(answer)
}

pub fn parse_greetings(answer: &str, count: usize) -> Vec<String> {
    pieces(answer).iter().map(|p| strip_label(p)).filter(|g| !g.is_empty()).take(count.max(1)).collect()
}

pub fn parse_lore_entries(answer: &str, existing_keys: &[String]) -> Vec<WorldInfoEntry> {
    let known = |k: &str| existing_keys.iter().any(|e| e.trim().eq_ignore_ascii_case(k));
    let mut entries = Vec::new();
    for piece in pieces(answer) {
        let mut keys = Vec::new();
        let mut content = String::new();
        let mut in_content = false;
        for line in piece.lines() {
            let bare = line.trim().trim_start_matches(['-', '*', ' ']).replace("**", "");
            let upper = bare.to_ascii_uppercase();
            if let Some(rest) = upper.strip_prefix("KEYS:").or_else(|| upper.strip_prefix("KEYWORDS:")) {
                let rest = &bare[bare.len() - rest.len()..];
                keys = rest.split(',').map(|k| k.trim().trim_matches(['"', '\'', '.']).to_string()).filter(|k| !k.is_empty()).collect();
                in_content = false;
            } else if let Some(rest) = upper.strip_prefix("CONTENT:") {
                content = bare[bare.len() - rest.len()..].trim().to_string();
                in_content = true;
            } else if in_content && !line.trim().is_empty() {
                content.push('\n');
                content.push_str(line.trim());
            }
        }
        if keys.is_empty() || content.trim().is_empty() || keys.iter().all(|k| known(k)) {
            continue;
        }
        entries.push(WorldInfoEntry { keys, content: content.trim().to_string(), ..Default::default() });
    }
    entries
}

fn normalize_time(text: &str) -> Option<String> {
    let (h, m) = text.trim().split_once([':', '.'])?;
    let (h, m): (u32, u32) = (h.trim().parse().ok()?, m.trim().get(..2)?.parse().ok()?);
    (h <= 24 && m < 60).then(|| format!("{:02}:{m:02}", h % 24))
}

fn parse_block(text: &str) -> Option<ScheduleBlock> {
    let text = text.trim().trim_start_matches(['-', '*']).trim().replace(" - ", "-").replace(" \u{2013} ", "-");
    let (range, rest) = text.split_once(char::is_whitespace)?;
    let (start, end) = range.split_once(['-', '\u{2013}', '\u{2014}'])?;
    let (activity, place) = match rest.split_once(" @ ").or_else(|| rest.split_once('@')) {
        Some((a, p)) => (a.trim(), Some(p.trim().to_string()).filter(|p| !p.is_empty())),
        None => (rest.trim(), None),
    };
    if activity.is_empty() {
        return None;
    }
    Some(ScheduleBlock { start: normalize_time(start)?, end: normalize_time(end)?, activity: activity.to_string(), place })
}

fn parse_kind(word: &str) -> DayKind {
    match word.trim().to_ascii_lowercase().as_str() {
        "work" | "workday" | "job" => DayKind::Work,
        "school" | "schoolday" | "school day" | "class" | "university" | "uni" => DayKind::School,
        "shift" | "night shift" => DayKind::Shift,
        _ => DayKind::Off,
    }
}

/// Fills a schedule from the answer's day lines, keeping `base`'s home and other settings. `None`
/// when no day line could be read.
pub fn parse_schedule(answer: &str, base: Schedule) -> Option<Schedule> {
    const DAYS: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
    let mut schedule = Schedule { days: vec![DayPlan::default(); 7], holiday: DayPlan::default(), ..base };
    let mut found = false;
    for line in unfence(answer).lines() {
        let line = line.trim().trim_start_matches(['-', '*']).replace("**", "");
        let Some((day, rest)) = line.split_once(':') else { continue };
        let day = day.trim().to_ascii_uppercase();
        let slot = if day.starts_with("HOLIDAY") {
            &mut schedule.holiday
        } else if let Some(i) = DAYS.iter().position(|d| day.starts_with(d)) {
            &mut schedule.days[i]
        } else {
            continue;
        };
        let (kind, blocks) = rest.split_once('|').unwrap_or((rest, ""));
        *slot = DayPlan { kind: parse_kind(kind), blocks: blocks.split(';').filter_map(parse_block).collect() };
        found = true;
    }
    found.then_some(schedule)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> Character {
        Character {
            name: "Mira".into(),
            description: "A baker.\n\n{{char}} has flour on her apron.".into(),
            personality: "  ".into(),
            first_messages: vec!["".into(), "*waves*".into()],
            ..Default::default()
        }
    }

    #[test]
    fn card_context_skips_blank_fields_and_keeps_macros() {
        let text = card_context(&card());
        assert_eq!(
            text,
            "Name: Mira\nDescription: A baker.\n\n{{char}} has flour on her apron.\nCurrent first message: *waves*"
        );
    }

    #[test]
    fn prompts_mention_the_idea_and_the_format() {
        let p = scenario_prompt(&card(), "a snowed-in village");
        assert!(p.starts_with("Write the Scenario"), "{p}");
        assert!(p.contains("{{char}} and {{user}}"), "{p}");
        assert!(p.ends_with("The author's idea for it:\na snowed-in village"), "{p}");
        assert!(greetings_prompt(&card(), "", 3).starts_with("Write 3 different opening messages"));
        assert!(schedule_prompt(&card(), "").contains("MON: work | 07:00-07:45 commute"));
        let lore = lore_prompt(&LoreDraftRequest {
            lorebook_name: "Vale".into(),
            topic: "the guilds".into(),
            count: 50,
            existing_keys: vec!["bakers' guild".into(), " ".into()],
            character: Some(card()),
        });
        assert!(lore.starts_with("Create 10 World Info entries"), "{lore}");
        assert!(lore.contains("don't repeat them: bakers' guild\n") || lore.contains("don't repeat them: bakers' guild\n\n"), "{lore}");
        assert!(lore.contains("Name: Mira"), "{lore}");
    }

    #[test]
    fn greetings_are_split_and_unlabelled() {
        let answer = "```\nGreeting 1:\n*She looks up.* \"Oh, hi.\"\n---\n2. *A bell rings.*\n\nSecond paragraph.\n***\n\n---\n**Option 3**\nThird.\n```";
        assert_eq!(
            parse_greetings(answer, 5),
            ["*She looks up.* \"Oh, hi.\"", "*A bell rings.*\n\nSecond paragraph.", "Third."]
        );
        assert_eq!(parse_greetings(answer, 1).len(), 1);
    }

    #[test]
    fn lore_entries_parse_and_skip_known_or_incomplete_ones() {
        let answer = "\
**KEYS:** Bakers' Guild, guild hall
CONTENT: The oldest guild in Vale.
It meets at dawn.
---
KEYS: harbor
---
- KEYS: Night Market
- CONTENT: Opens at dusk.
---
KEYS: old mill
CONTENT: Known already.";
        let entries = parse_lore_entries(answer, &["OLD MILL".into()]);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].keys, ["Bakers' Guild", "guild hall"]);
        assert_eq!(entries[0].content, "The oldest guild in Vale.\nIt meets at dawn.");
        assert!(entries[0].enabled);
        assert_eq!(entries[1].keys, ["Night Market"]);
        assert_eq!(entries[1].content, "Opens at dusk.");
    }

    #[test]
    fn schedules_parse_day_lines_and_keep_the_home() {
        let answer = "\
MON: work | 7:00-07:45 commute; 08:00-16:00 work @ the bakery; 18:00\u{2013}19:30 gym
- **TUE**: school | 08:00-13:00 classes
Friday: shift | 22:00-06:00 night shift @ hospital
SAT: off
HOLIDAY: off | 12:00-14:00 lunch with family
nonsense line
WED: work | garbage; 09:00-17:00 office";
        let base = Schedule { home_country: Some("DE".into()), observes_holidays: false, ..Default::default() };
        let s = parse_schedule(answer, base).unwrap();
        assert_eq!(s.home_country.as_deref(), Some("DE"));
        assert!(!s.observes_holidays);
        assert_eq!(s.days[0].kind, DayKind::Work);
        assert_eq!(s.days[0].blocks.len(), 3);
        assert_eq!(s.days[0].blocks[0].start, "07:00");
        assert_eq!(s.days[0].blocks[1].place.as_deref(), Some("the bakery"));
        assert_eq!(s.days[0].blocks[2].end, "19:30");
        assert_eq!(s.days[1].kind, DayKind::School);
        assert_eq!(s.days[2].blocks.len(), 1, "unreadable blocks are skipped");
        assert_eq!(s.days[4].kind, DayKind::Shift);
        assert_eq!(s.days[4].blocks[0].end, "06:00");
        assert_eq!(s.days[6], DayPlan::default(), "missing days are days off");
        assert_eq!(s.holiday.blocks[0].activity, "lunch with family");
        assert!(parse_schedule("I can't do that.", Schedule::default()).is_none());
    }
}
