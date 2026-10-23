//! k9s-style `:` command resolution (pure).

use crate::proc::SortKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Empty,
    Sort(SortKey),
    Info,
    Quit,
    Unknown,
}

/// Accepted words in suggestion order; anything else resolves by unique prefix.
const NAMES: &[(&str, Command)] = &[
    ("cpu", Command::Sort(SortKey::Cpu)),
    ("c", Command::Sort(SortKey::Cpu)),
    ("mem", Command::Sort(SortKey::Mem)),
    ("memory", Command::Sort(SortKey::Mem)),
    ("m", Command::Sort(SortKey::Mem)),
    ("pid", Command::Sort(SortKey::Pid)),
    ("p", Command::Sort(SortKey::Pid)),
    ("name", Command::Sort(SortKey::Name)),
    ("n", Command::Sort(SortKey::Name)),
    ("info", Command::Info),
    ("quit", Command::Quit),
    ("q", Command::Quit),
];

pub fn resolve(input: &str) -> Command {
    let s = input.trim().to_ascii_lowercase();
    if s.is_empty() {
        return Command::Empty;
    }
    if let Some((_, c)) = NAMES.iter().find(|(n, _)| *n == s) {
        return *c;
    }
    let mut hits = NAMES.iter().filter(|(n, _)| n.starts_with(&s));
    match (hits.next(), hits.next()) {
        (Some((_, c)), None) => *c,
        _ => Command::Unknown,
    }
}

/// Full-word completion of the typed prefix.
pub fn suggest(input: &str) -> Option<&'static str> {
    let s = input.to_ascii_lowercase();
    if s.is_empty() {
        return None;
    }
    NAMES
        .iter()
        .map(|(n, _)| *n)
        .find(|n| n.len() > s.len() && n.starts_with(&s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_prefixes_unknown() {
        let s = Command::Sort;
        assert_eq!(resolve("c"), s(SortKey::Cpu));
        assert_eq!(resolve("MEM"), s(SortKey::Mem));
        assert_eq!(resolve("pid"), s(SortKey::Pid));
        assert_eq!(resolve("na"), s(SortKey::Name));
        assert_eq!(resolve("info"), Command::Info);
        assert_eq!(resolve("q"), Command::Quit);
        assert_eq!(resolve(""), Command::Empty);
        assert_eq!(resolve("zz"), Command::Unknown);
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("cp"), Some("cpu"));
        assert_eq!(suggest("cpu"), None);
        assert_eq!(suggest(""), None);
        assert_eq!(suggest("zz"), None);
    }

    #[test]
    fn edge_inputs_never_panic() {
        assert_eq!(resolve("   "), Command::Empty);
        assert_eq!(resolve("  cpu  "), Command::Sort(SortKey::Cpu));
        assert_eq!(resolve("Q"), Command::Quit);
        for junk in ["🦀", "图像", "c m", "cpux", "::", "1", "-1"] {
            assert_eq!(resolve(junk), Command::Unknown, "{junk}");
        }
        assert_eq!(suggest("🦀"), None);
    }
}
