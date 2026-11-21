//! Excluded sessions (`exclude_sessions`, ADR-0010): sessions another tool
//! makes and keeps up itself, which rexurrect leaves to that tool. Save
//! doesn't save them, and restore neither creates nor replaces them.

/// The label patterns of the excluded sessions. A pattern matches a whole
/// label, and each `*` in it matches any run of characters, none included:
/// `"T3 Code"` matches only that label, `"T3 Code · *"` every label it
/// starts.
#[derive(Debug, Clone, Default)]
pub struct ExcludedSessions {
    patterns: Vec<String>,
}

impl ExcludedSessions {
    /// `None` if a pattern has no text other than `*`: `"*"` would exclude
    /// every session, so that each save on a restored server instance
    /// replaced the latest snapshot with an empty one (ADR-0009), and `""`
    /// names none.
    pub fn new(patterns: Vec<String>) -> Option<ExcludedSessions> {
        patterns
            .iter()
            .all(|pattern| pattern.chars().any(|c| c != '*'))
            .then_some(ExcludedSessions { patterns })
    }

    #[cfg(test)]
    pub fn patterns(&self) -> &[String] {
        &self.patterns
    }

    /// Whether a session labelled `label` is excluded.
    pub fn contains(&self, label: &str) -> bool {
        self.patterns.iter().any(|pattern| matches(pattern, label))
    }
}

/// Whether `pattern`, with `*` matching any run of characters, matches the
/// whole of `text`.
fn matches(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    // `split` always yields at least one part.
    let first = parts.next().unwrap_or_default();
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        // No `*`: the pattern is the whole label.
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn excluded(patterns: &[&str]) -> ExcludedSessions {
        new(patterns).expect("patterns with text")
    }

    fn new(patterns: &[&str]) -> Option<ExcludedSessions> {
        ExcludedSessions::new(patterns.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn a_pattern_without_a_star_matches_only_that_label() {
        let sessions = excluded(&["T3 Code"]);
        assert!(sessions.contains("T3 Code"));
        assert!(!sessions.contains("T3 Code · studio"));
        assert!(!sessions.contains("My T3 Code"));
        assert!(!sessions.contains("t3 code"));
    }

    #[test]
    fn a_star_matches_any_run_of_characters_or_none() {
        let sessions = excluded(&["T3 Code · *"]);
        assert!(sessions.contains("T3 Code · studio"));
        assert!(sessions.contains("T3 Code · "));
        assert!(!sessions.contains("T3 Code"));

        let sessions = excluded(&["*-scratch", "a*b*c"]);
        assert!(sessions.contains("work-scratch"));
        assert!(sessions.contains("-scratch"));
        assert!(!sessions.contains("work-scratch-2"));
        assert!(sessions.contains("abc"));
        assert!(sessions.contains("a-b-c"));
        assert!(!sessions.contains("a-c-b"));
        // The last part can't reuse characters a middle part matched.
        assert!(!sessions.contains("ab"));
        assert!(excluded(&["x*"]).contains("x"));
        assert!(!excluded(&["xy*yx"]).contains("xyx"));
    }

    #[test]
    fn a_pattern_needs_text_other_than_a_star() {
        for patterns in [&["*"][..], &["**"], &[""], &["T3 Code", "*"]] {
            assert!(new(patterns).is_none(), "{patterns:?}");
        }
        assert!(new(&["*-scratch", "T3 Code"]).is_some());
    }

    #[test]
    fn no_patterns_exclude_nothing() {
        assert!(!ExcludedSessions::default().contains("T3 Code"));
        assert!(!excluded(&[]).contains(""));
    }
}
