//! Keeps a group reply to its own speaker. Every earlier group reply reaches the model labelled
//! "Name: ...", so a model easily slips into writing a transcript: "Alice: ..." followed by
//! "Bob: ..." (and the user's next line) in one reply. Saved under Alice, that reply then teaches
//! the next prompt the same habit. The reply is cut where another speaker's line starts, and a
//! leading "Alice:" label (the app labels messages itself) is dropped.

use regex::Regex;

pub struct SpeakerCut {
    /// The speaker's own label at the very start: `Alice:`, `**Alice:**`.
    own: Regex,
    /// A line opening with someone else's label.
    others: Regex,
}

/// A name label, optionally in bold or italics: `Bob:`, `**Bob:**`, `**Bob**:`, `_Bob_:`.
fn label(names: &[&str]) -> String {
    let names: Vec<String> = names.iter().map(|n| regex::escape(n.trim())).collect();
    format!(r"[*_]{{0,2}}(?:{})[*_]{{0,2}}[ \t]*:[*_]{{0,2}}", names.join("|"))
}

impl SpeakerCut {
    /// `others` are everyone else in the scene: the other members and the user's character.
    pub fn new(speaker: &str, others: &[&str]) -> Option<Self> {
        let others: Vec<&str> = others.iter().copied().filter(|n| !n.trim().is_empty() && *n != speaker).collect();
        if speaker.trim().is_empty() || others.is_empty() {
            return None;
        }
        Some(Self {
            own: Regex::new(&format!(r"(?i)^\s*{}[ \t]*", label(&[speaker]))).ok()?,
            others: Regex::new(&format!(r"(?im)^[ \t]*{}", label(&others))).ok()?,
        })
    }

    /// Where the speaker's part ends: the start of the first line, after the reply's first, that
    /// opens with another speaker's label. A reply that opens with one is left alone (there would
    /// be nothing of the speaker's to keep).
    pub fn find(&self, text: &str) -> Option<usize> {
        let start = self.own.find(text).map_or(0, |m| m.end());
        let body = &text[start..];
        let first_line = body.len() - body.trim_start().len();
        self.others.find_iter(body).map(|m| m.start()).find(|at| *at > first_line).map(|at| start + at)
    }

    /// The reply as the speaker's alone: without a leading label of their own, and cut before
    /// another speaker's line.
    pub fn apply(&self, text: &str) -> String {
        let end = self.find(text).unwrap_or(text.len());
        let kept = &text[..end];
        let start = self.own.find(kept).map_or(0, |m| m.end());
        kept[start..].trim().to_string()
    }

    /// A continuation of the speaker's reply, cut before another speaker's line (which may be its
    /// very first).
    pub fn apply_continuation(&self, added: &str) -> String {
        match self.others.find(added) {
            Some(m) => added[..m.start()].trim_end().to_string(),
            None => added.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut() -> SpeakerCut {
        SpeakerCut::new("Alice", &["Bob", "Cara", "Alex"]).unwrap()
    }

    #[test]
    fn a_transcript_is_cut_back_to_the_speakers_part() {
        let reply = "Alice: *She laughs.* \"Fine, I'll go.\"\n\nBob: \"Me too!\"\n\nAlex: \"Wait for me.\"";
        assert_eq!(cut().apply(reply), "*She laughs.* \"Fine, I'll go.\"");
        assert_eq!(cut().find(reply), Some(reply.find("Bob:").unwrap()));
    }

    #[test]
    fn labels_in_bold_or_italics_count_too() {
        assert_eq!(cut().apply("**Alice:** Hi.\n**Bob:** Hey.\n"), "Hi.");
        assert_eq!(cut().apply("Sure.\n\n**Cara**: Not me.\n"), "Sure.");
        assert_eq!(cut().apply("_Alice_: Hm.\n  bob : no"), "Hm.", "any case, leading spaces");
    }

    #[test]
    fn plain_replies_and_names_in_the_story_are_untouched() {
        let reply = "*Alice glances at Bob.* \"Bob: always late.\"\nBob's coat is wet.";
        assert_eq!(cut().apply(reply), reply);
        assert_eq!(cut().find(reply), None);
        assert_eq!(cut().apply("Alice: Just me."), "Just me.");
        // A reply that opens with someone else's line keeps everything rather than nothing.
        assert_eq!(cut().apply("Bob: hi"), "Bob: hi");
    }

    #[test]
    fn a_continuation_is_cut_even_at_its_start() {
        assert_eq!(cut().apply_continuation(" and left.\nBob: Bye!"), " and left.");
        assert_eq!(cut().apply_continuation("\nCara: Wait!"), "");
        assert_eq!(cut().apply_continuation(" more words"), " more words");
    }

    #[test]
    fn nobody_else_means_nothing_to_cut() {
        assert!(SpeakerCut::new("Alice", &[]).is_none());
        assert!(SpeakerCut::new("Alice", &["Alice", " "]).is_none());
    }
}
