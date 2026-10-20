//! Reading `arousal 40, climax 10, orgasm yes` - from the meter tracker's answer, or from a stray
//! `[meters: ...]` tag at the end of a reply (which is stripped and otherwise ignored).

use std::sync::LazyLock;

use regex::Regex;

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[\[(]\s*meters?\s*[:=]([^\])]*)[\])]").unwrap());
static AROUSAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)arousal\s*[:=]?\s*(\d{1,3})").unwrap());
static CLIMAX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(?:climax|orgasm)\s*[:=]?\s*(\d{1,3})").unwrap());
static ORGASM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:orgasm|climaxed|came)\s*[:=]?\s*(yes|true|no|false)").unwrap());

/// What the model reported; a number it left out stays as it was.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MeterReading {
    pub arousal: Option<u8>,
    pub climax: Option<u8>,
    /// The reply narrates the character reaching orgasm, whatever the climax number says.
    pub orgasm: bool,
}

/// The tag in `tail` (the trailing tags peeled off a reply), if it has at least one readable number.
/// Replies are no longer asked to carry it - the story is read after the fact (`chat::meters`) - but a
/// model that writes one anyway still has it stripped.
pub fn parse_tail(tail: &str) -> Option<MeterReading> {
    parse_values(TAG.captures(tail)?.get(1)?.as_str())
}

/// "arousal 40, climax 10, orgasm yes" in any of the shapes models write it.
pub fn parse_values(body: &str) -> Option<MeterReading> {
    let number = |re: &Regex| re.captures(body).and_then(|c| c[1].parse::<u32>().ok()).map(|n| n.min(100) as u8);
    let orgasm = ORGASM.captures(body).is_some_and(|c| matches!(c[1].to_lowercase().as_str(), "yes" | "true"));
    let reading = MeterReading { arousal: number(&AROUSAL), climax: number(&CLIMAX), orgasm };
    (reading != MeterReading::default()).then_some(reading)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_numbers_in_any_shape() {
        for tail in [
            "[meters: arousal 40, climax 10]",
            "[Meters: arousal=40 climax=10]",
            "(meter: Arousal: 40; Climax: 10)",
            "[time-skip: 5 minutes] [meters: arousal 40, orgasm 10]",
            "[meters: arousal 40, climax 10, orgasm no]",
        ] {
            assert_eq!(
                parse_tail(tail),
                Some(MeterReading { arousal: Some(40), climax: Some(10), orgasm: false }),
                "{tail}"
            );
        }
    }

    #[test]
    fn a_missing_number_is_left_alone_and_big_ones_are_capped() {
        assert_eq!(parse_tail("[meters: arousal 250]"), Some(MeterReading { arousal: Some(100), climax: None, orgasm: false }));
        assert_eq!(parse_tail("[meters: climax 7]"), Some(MeterReading { arousal: None, climax: Some(7), orgasm: false }));
    }

    #[test]
    fn an_orgasm_flag_is_read_with_or_without_numbers() {
        let yes = parse_tail("[meters: arousal 85, climax 70, orgasm yes]").unwrap();
        assert!(yes.orgasm);
        assert_eq!((yes.arousal, yes.climax), (Some(85), Some(70)));
        assert!(parse_tail("[meters: orgasm: true]").unwrap().orgasm);
        assert!(!parse_tail("[meters: arousal 5, orgasm false]").unwrap().orgasm);
    }

    #[test]
    fn no_tag_or_no_numbers_is_none() {
        assert_eq!(parse_tail("[time-skip: 5 minutes]"), None);
        assert_eq!(parse_tail("[meters: unknown]"), None);
    }
}
