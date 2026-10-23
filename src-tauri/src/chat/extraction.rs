//! Best-effort background enrichment after a reply: pulling durable facts about the user out of
//! Assistant chats. It asks the model a single question about one exchange and applies the answer.
//! It runs detached from the reply (see `ChatEngine::spawn_enrichment`), and a failure here never
//! reaches the chat - the visible reply already succeeded.

use crate::db::Db;
use crate::error::{Error, Result};
use crate::llm::{LlmGenerationConfig, LlmMessage, LlmRequest, LlmRole};
use crate::models::{StoryCharacter, UserMemory};
use crate::providers::LlmProviderRegistry;
use crate::repo::{story_characters, user_memories};
use chrono::NaiveDateTime;
use uuid::Uuid;

/// Where to send the question and what to store the answer in.
pub struct Extractor<'a> {
    pub db: &'a Db,
    pub providers: &'a LlmProviderRegistry,
    pub provider_id: &'a str,
    pub model: &'a str,
}

/// One exchange: the user's message and the reply to it.
pub struct Exchange<'a> {
    pub user_text: &'a str,
    pub reply_text: &'a str,
}

/// One roleplay exchange, for spotting people the story has introduced.
pub struct StoryExchange<'a> {
    pub conversation_id: Uuid,
    pub user_text: &'a str,
    pub reply_text: &'a str,
    /// The in-story time of the exchange.
    pub story_time: Option<NaiveDateTime>,
    pub message_id: Uuid,
    /// The main cast and the user's persona: never listed as side characters.
    pub main_names: Vec<String>,
}

/// What the model reported about one person. Missing fields stay empty.
#[derive(Debug, Default)]
pub struct FoundCharacter {
    pub name: String,
    pub age: Option<i32>,
    pub role: String,
    pub appearance: String,
    pub adult_details: String,
}

/// The JSON array in a model answer (it may wrap it in a code fence or add chatter).
pub fn parse_found(response: &str) -> Vec<FoundCharacter> {
    let (Some(start), Some(end)) = (response.find('['), response.rfind(']')) else { return Vec::new() };
    if end < start {
        return Vec::new();
    }
    serde_json::from_str::<Vec<serde_json::Value>>(&response[start..=end])
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| {
            // Tolerate numbers as strings and null fields.
            let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
            let age = v.get("age").and_then(|a| a.as_i64().or_else(|| a.as_str().and_then(|s| s.trim().parse().ok())));
            let name = text("name");
            (!name.is_empty()).then(|| FoundCharacter {
                name,
                age: age.and_then(|a| i32::try_from(a).ok()),
                role: text("role"),
                appearance: text("appearance"),
                adult_details: text("adult_details"),
            })
        })
        .collect()
}

/// Same name, or one is just the first name of the other.
fn same_person(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_lowercase(), b.trim().to_lowercase());
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let (wa, wb) = (a.split_whitespace().collect::<Vec<_>>(), b.split_whitespace().collect::<Vec<_>>());
    a == b || (wa[0] == wb[0] && (wa.len() == 1 || wb.len() == 1))
}

/// Applies findings to the stored cast. Returns the characters that were added or changed.
/// Main-cast names are skipped; a known person gets the newer non-empty facts, but their age and
/// introduction stay as first recorded.
pub fn merge_found(known: &mut Vec<StoryCharacter>, found: Vec<FoundCharacter>, ex: &StoryExchange<'_>) -> Vec<StoryCharacter> {
    let mut changed: Vec<StoryCharacter> = Vec::new();
    for f in found {
        if ex.main_names.iter().any(|n| same_person(n, &f.name)) {
            continue;
        }
        let index = match known.iter().position(|k| same_person(&k.name, &f.name)) {
            Some(i) => i,
            None => {
                let mut c = StoryCharacter::new(ex.conversation_id, &f.name);
                c.introduced_at = ex.story_time;
                c.introduced_message_id = Some(ex.message_id);
                known.push(c);
                known.len() - 1
            }
        };
        let c = &mut known[index];
        c.age = c.age.or(f.age);
        if !f.role.is_empty() {
            c.role = f.role;
        }
        if !f.appearance.is_empty() {
            c.appearance = f.appearance;
        }
        if !f.adult_details.is_empty() {
            c.adult_details = f.adult_details;
        }
        c.enforce_rules();
        let c = known[index].clone();
        changed.retain(|x| x.id != c.id);
        changed.push(c);
    }
    changed
}

/// "- fact" / "* fact" / "fact" lines, without blanks.
fn fact_lines(response: &str) -> impl Iterator<Item = &str> {
    response.lines().map(|line| line.trim().trim_start_matches(['-', '*', ' '])).filter(|line| !line.is_empty())
}

impl Extractor<'_> {
    pub async fn ask(&self, prompt: String) -> Result<String> {
        let provider = self.providers.get(self.provider_id).map_err(|e| Error::Other(e.message))?;
        let request = LlmRequest {
            model: self.model.to_string(),
            system_instruction: None,
            history: vec![LlmMessage::new(LlmRole::User, prompt)],
            generation_config: LlmGenerationConfig::without_reasoning(),
            enable_web_search: None,
        };
        let response = provider.get_completion(request).await.map_err(|e| Error::Other(e.message))?;
        Ok(response.delta_text)
    }

    /// Assistant chats: new durable facts about the user, kept for future Assistant chats.
    pub async fn memories(&self, exchange: &Exchange<'_>) -> Result<()> {
        if exchange.user_text.trim().is_empty() {
            return Ok(());
        }
        let existing = self.db.with(user_memories::get_all)?;
        let existing_list = if existing.is_empty() {
            "(none yet)".to_string()
        } else {
            existing.iter().map(|m| format!("- {}", m.content)).collect::<Vec<_>>().join("\n")
        };

        let prompt = format!(
            "You are extracting durable facts about the USER from one exchange in an ongoing assistant conversation, for long-term memory recall in future chats.\n\n\
Facts already remembered (do not repeat these):\n{existing_list}\n\n\
Exchange:\nUser: {user}\nAssistant: {reply}\n\n\
List any NEW, durable, specific facts about the user worth remembering long-term (identity, preferences, ongoing projects, constraints) - one per line, no bullets, no commentary, no quotes. Skip trivia, small talk, or anything already remembered. If there is nothing new and durable, output exactly: NONE",
            user = exchange.user_text,
            reply = exchange.reply_text,
        );
        let response = self.ask(prompt).await?;

        self.db.with(|c| {
            for fact in fact_lines(&response).filter(|f| !f.eq_ignore_ascii_case("NONE")) {
                user_memories::add(c, &UserMemory::new(fact))?;
            }
            Ok(())
        })
    }

    /// Roleplay: people the story has just introduced (or told more about) that are not on a
    /// character card. Returns the whole stored cast when something changed.
    pub async fn story_cast(&self, ex: &StoryExchange<'_>) -> Result<Option<Vec<StoryCharacter>>> {
        let mut known = self.db.with(|c| story_characters::for_conversation(c, ex.conversation_id))?;
        let known_list = if known.is_empty() {
            "(none yet)".to_string()
        } else {
            known
                .iter()
                .map(|k| {
                    format!(
                        "- {} (age {}; {}) look: {} | intimate: {}",
                        k.name,
                        k.age.map(|a| a.to_string()).unwrap_or_else(|| "unknown".into()),
                        k.role,
                        k.appearance,
                        if k.adult_details.is_empty() { "(none)" } else { &k.adult_details },
                    )
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let prompt = format!(
            "You keep the cast list of a fictional roleplay story (fiction for adults). From the exchange below, find side \
characters who appear in it and are NOT one of these main characters: {main}.\n\n\
Side characters already recorded:\n{known_list}\n\n\
Exchange:\nUser: {user}\nStory: {reply}\n\n\
Reply with ONLY a JSON array (no commentary) containing every side character who is NEW, or about whom the exchange \
established NEW facts. Each item has these keys:\n\
\"name\": their name as used in the story;\n\
\"age\": a whole number if stated or clearly implied, otherwise null;\n\
\"role\": who they are in the story, in a few words;\n\
\"appearance\": a proper description of what they look like - build, height, hair, eyes, skin, clothing, distinguishing marks, \
voice, manner - using only what the story says or clearly implies (for a character already recorded, give the full updated text);\n\
\"adult_details\": only when the story is explicit about them: their intimate physical details (body, intimate anatomy), stated \
matter-of-factly; otherwise an empty string.\n\
Skip crowds, unnamed extras and anyone merely mentioned in passing. If there is nobody, reply exactly: []",
            main = if ex.main_names.is_empty() { "(none)".to_string() } else { ex.main_names.join(", ") },
            user = ex.user_text,
            reply = ex.reply_text,
        );
        let found = parse_found(&self.ask(prompt).await?);
        let changed = merge_found(&mut known, found, ex);
        if changed.is_empty() {
            return Ok(None);
        }
        self.db.with(|c| changed.iter().try_for_each(|k| story_characters::upsert(c, k)))?;
        Ok(Some(known))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exchange(main: &[&str]) -> StoryExchange<'static> {
        StoryExchange {
            conversation_id: Uuid::new_v4(),
            user_text: "",
            reply_text: "",
            story_time: None,
            message_id: Uuid::new_v4(),
            main_names: main.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn found(name: &str, age: Option<i32>, look: &str, adult: &str) -> FoundCharacter {
        FoundCharacter { name: name.into(), age, role: "r".into(), appearance: look.into(), adult_details: adult.into() }
    }

    #[test]
    fn parses_fenced_json_and_loose_ages() {
        let list = parse_found("Sure!\n```json\n[{\"name\":\"Tom\",\"age\":\"34\",\"role\":\"bartender\",\"appearance\":\"red beard\",\"adult_details\":null}]\n```");
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].name.as_str(), list[0].age, list[0].adult_details.as_str()), ("Tom", Some(34), ""));
        assert!(parse_found("[]").is_empty());
        assert!(parse_found("nothing").is_empty());
    }

    #[test]
    fn merge_skips_the_main_cast_and_keeps_first_age() {
        let ex = exchange(&["Mia Weber"]);
        let mut known = Vec::new();
        let changed = merge_found(
            &mut known,
            vec![found("Mia", Some(20), "x", ""), found("Tom", Some(34), "red beard", "d"), found("Kid", Some(15), "small", "d")],
            &ex,
        );
        assert_eq!(changed.len(), 2); // Mia is skipped as main cast, Tom and Kid are added
        assert_eq!(known.len(), 2);
        assert_eq!(known[0].adult_details, "d");
        assert_eq!(known[1].adult_details, "d"); // No longer filtered by age

        let changed = merge_found(&mut known, vec![found("tom", Some(50), "red beard, scar", "")], &ex);
        assert_eq!(changed.len(), 1);
        assert_eq!((known[0].age, known[0].appearance.as_str(), known[0].adult_details.as_str()), (Some(34), "red beard, scar", "d"));
        assert_eq!(known.len(), 2);
    }
}
