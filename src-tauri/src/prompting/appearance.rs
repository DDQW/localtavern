//! Turns quick-builder picks into the appearance paragraph stored on a character or persona. The
//! text names the subject by macro (`{{char}}` / `{{user}}`) so it survives renames, and carries no
//! age: that depends on the in-story date and is added per prompt (see `world_state::format`).

use crate::models::{Appearance, Sex};

/// Whose appearance is being written; picks the macro that names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppearanceSubject {
    Character,
    Persona,
}

impl AppearanceSubject {
    pub fn macro_name(self) -> &'static str {
        match self {
            Self::Character => "{{char}}",
            Self::Persona => "{{user}}",
        }
    }
}

pub fn render(a: &Appearance, subject: AppearanceSubject) -> String {
    let (noun, they, their) = match a.sex {
        Some(Sex::Female) => ("woman", "She", "Her"),
        Some(Sex::Male) => ("man", "He", "His"),
        None => ("person", "They", "Their"),
    };
    let has = if a.sex.is_some() { "has" } else { "have" };
    let mut sentences = Vec::new();

    // "{{char}} is a woman, 168 cm (5'6") tall and 58 kg (128 lb), with a slim, toned build."
    let mut intro = format!("{} is a {noun}", subject.macro_name());
    let height = a.height_cm.filter(|&h| h > 0).map(|h| format!("{h} cm ({}) tall", feet_inches(h)));
    let weight = a.weight_kg.filter(|&w| w > 0).map(|w| format!("{w} kg ({} lb)", pounds(w)));
    if let Some(size) = join_and(&[height, weight].into_iter().flatten().collect::<Vec<_>>()) {
        intro.push_str(", ");
        intro.push_str(&size);
    }
    let build = [text(&a.build), text(&a.muscle_tone)].into_iter().flatten().collect::<Vec<_>>();
    if !build.is_empty() {
        intro.push_str(&format!(", with a {} build", build.join(", ")));
    }
    sentences.push(intro + ".");

    // "She has long auburn hair (wavy), green eyes and fair skin."
    let hair = {
        let words = [text(&a.hair_length), text(&a.hair_color)].into_iter().flatten().collect::<Vec<_>>();
        match (words.is_empty(), text(&a.hair_style)) {
            (true, None) => None,
            (true, Some(style)) => Some(format!("{style} hair")),
            (false, None) => Some(format!("{} hair", words.join(" "))),
            (false, Some(style)) => Some(format!("{} hair ({style})", words.join(" "))),
        }
    };
    let eyes = text(&a.eye_color).map(|c| format!("{c} eyes"));
    let skin = text(&a.skin_tone).map(|s| format!("{s} skin"));
    if let Some(features) = join_and(&[hair, eyes, skin].into_iter().flatten().collect::<Vec<_>>()) {
        sentences.push(format!("{they} {has} {features}."));
    }

    match a.sex {
        Some(Sex::Female) => {
            let f = &a.female;
            let breasts = match (text(&f.cup_size), text(&f.bust)) {
                (Some(cup), Some(bust)) => Some(format!("{cup}-cup {bust} breasts")),
                (Some(cup), None) => Some(format!("{cup}-cup breasts")),
                (None, Some(bust)) => Some(format!("{bust} breasts")),
                (None, None) => None,
            };
            let parts = [
                breasts,
                text(&f.waist).map(|w| format!("a {w} waist")),
                text(&f.hips).map(|h| format!("{h} hips")),
                text(&f.butt).map(|b| format!("a {b} butt")),
            ];
            if let Some(figure) = join_and(&parts.into_iter().flatten().collect::<Vec<_>>()) {
                sentences.push(format!("{their} figure: {figure}."));
            }
        }
        Some(Sex::Male) => {
            let m = &a.male;
            let parts = [text(&m.shoulders).map(|s| format!("{s} shoulders")), text(&m.chest).map(|c| format!("a {c} chest"))];
            if let Some(figure) = join_and(&parts.into_iter().flatten().collect::<Vec<_>>()) {
                sentences.push(format!("{their} physique: {figure}."));
            }
            if let Some(facial) = text(&m.facial_hair) {
                sentences.push(format!("Facial hair: {facial}."));
            }
        }
        None => {}
    }

    if let Some(hair) = text(&a.body_hair) {
        sentences.push(format!("Body hair: {hair}."));
    }
    let marks = a.marks.iter().map(|m| m.trim()).filter(|m| !m.is_empty()).map(String::from).collect::<Vec<_>>();
    if !marks.is_empty() {
        sentences.push(format!("Distinguishing features: {}.", marks.join(", ")));
    }
    if let Some(notes) = text(&a.notes) {
        sentences.push(sentence(notes));
    }

    let adult = &a.adult;
    let intimate = [
        text(&adult.nipples).map(|n| format!("nipples: {n}")),
        text(&adult.pubic_hair).map(|p| format!("pubic hair: {p}")),
        text(&adult.genitals).map(|g| format!("genitals: {g}")),
        text(&adult.circumcision).filter(|_| a.sex == Some(Sex::Male)).map(String::from),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let mut paragraphs = vec![sentences.join(" ")];
    if !intimate.is_empty() || text(&adult.notes).is_some() {
        let mut block = format!("{}'s intimate details", subject.macro_name());
        if !intimate.is_empty() {
            block.push_str(&format!(": {}.", intimate.join("; ")));
        } else {
            block.push(':');
        }
        if let Some(notes) = text(&adult.notes) {
            block.push(' ');
            block.push_str(&sentence(notes));
        }
        paragraphs.push(block);
    }
    paragraphs.join("\n\n")
}

/// A trimmed, non-empty trait value.
fn text(value: &Option<String>) -> Option<&str> {
    value.as_deref().map(str::trim).filter(|v| !v.is_empty())
}

/// Ends free text with a full stop unless it already has closing punctuation.
fn sentence(s: &str) -> String {
    if s.ends_with(['.', '!', '?']) {
        s.to_string()
    } else {
        format!("{s}.")
    }
}

/// "a", "a and b", "a, b and c".
fn join_and(items: &[String]) -> Option<String> {
    match items {
        [] => None,
        [one] => Some(one.clone()),
        [init @ .., last] => Some(format!("{} and {last}", init.join(", "))),
    }
}

fn feet_inches(cm: u16) -> String {
    let inches = (cm as f64 / 2.54).round() as u32;
    format!("{}'{}\"", inches / 12, inches % 12)
}

fn pounds(kg: u16) -> u32 {
    (kg as f64 * 2.204_62).round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AdultTraits, FemaleTraits, MaleTraits};

    fn some(s: &str) -> Option<String> {
        Some(s.into())
    }

    fn woman() -> Appearance {
        Appearance {
            sex: Some(Sex::Female),
            height_cm: Some(168),
            weight_kg: Some(58),
            build: some("slim"),
            muscle_tone: some("toned"),
            skin_tone: some("fair"),
            hair_color: some("auburn"),
            hair_length: some("long"),
            hair_style: some("wavy"),
            eye_color: some("green"),
            marks: vec!["freckles".into(), " ".into()],
            female: FemaleTraits { cup_size: some("C"), waist: some("narrow"), hips: some("wide"), ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn renders_a_woman_with_figure() {
        let text = render(&woman(), AppearanceSubject::Character);
        assert_eq!(
            text,
            "{{char}} is a woman, 168 cm (5'6\") tall and 58 kg (128 lb), with a slim, toned build. \
She has long auburn hair (wavy), green eyes and fair skin. \
Her figure: C-cup breasts, a narrow waist and wide hips. \
Distinguishing features: freckles."
        );
    }

    #[test]
    fn renders_a_man_for_a_persona_and_ignores_female_traits() {
        let a = Appearance {
            sex: Some(Sex::Male),
            hair_color: some("black"),
            eye_color: some("grey"),
            female: FemaleTraits { cup_size: some("D"), ..Default::default() },
            male: MaleTraits { shoulders: some("broad"), facial_hair: some("stubble"), ..Default::default() },
            ..Default::default()
        };
        let text = render(&a, AppearanceSubject::Persona);
        assert!(text.starts_with("{{user}} is a man."), "{text}");
        assert!(text.contains("He has black hair and grey eyes."), "{text}");
        assert!(text.contains("His physique: broad shoulders."), "{text}");
        assert!(text.contains("Facial hair: stubble."), "{text}");
        assert!(!text.contains("cup"), "{text}");
    }

    #[test]
    fn empty_picks_render_a_bare_intro() {
        assert_eq!(render(&Appearance::default(), AppearanceSubject::Character), "{{char}} is a person.");
    }

    #[test]
    fn blank_values_are_skipped() {
        let a = Appearance { sex: Some(Sex::Female), hair_color: some("  "), eye_color: some("blue"), ..Default::default() };
        assert_eq!(render(&a, AppearanceSubject::Character), "{{char}} is a woman. She has blue eyes.");
    }

    #[test]
    fn intimate_details_only_when_filled() {
        let mut a = woman();
        assert!(!render(&a, AppearanceSubject::Character).contains("intimate"));
        a.adult = AdultTraits { pubic_hair: some("trimmed"), circumcision: some("circumcised"), ..Default::default() };
        let text = render(&a, AppearanceSubject::Character);
        assert!(text.ends_with("\n\n{{char}}'s intimate details: pubic hair: trimmed."), "{text}");
    }

    #[test]
    fn circumcision_is_kept_for_men() {
        let a = Appearance {
            sex: Some(Sex::Male),
            adult: AdultTraits { circumcision: some("uncircumcised"), notes: some("Has a birthmark on the thigh"), ..Default::default() },
            ..Default::default()
        };
        assert!(render(&a, AppearanceSubject::Persona)
            .ends_with("{{user}}'s intimate details: uncircumcised. Has a birthmark on the thigh."));
    }

    #[test]
    fn converts_units() {
        assert_eq!(feet_inches(183), "6'0\"");
        assert_eq!(feet_inches(155), "5'1\"");
        assert_eq!(pounds(80), 176);
    }
}
