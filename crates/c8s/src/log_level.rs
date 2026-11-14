//! Hide log lines below a minimum severity (WARN or ERROR).

use alloc::borrow::Cow;
use core::fmt::{self, Display, Formatter};

use ratatui::text::Line;

#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum LevelFilter {
    #[default]
    All,
    /// WARN and above.
    Warn,
    /// ERROR and above.
    Error,
}

impl LevelFilter {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Warn,
            Self::Warn => Self::Error,
            Self::Error => Self::All,
        }
    }
}

impl Display for LevelFilter {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "all",
            Self::Warn => "warn+",
            Self::Error => "error+",
        })
    }
}

/// Severity of the first level word in `text`; `All` means a level below
/// WARN (info/debug/trace), `None` means no level word at all.
fn detect(text: &str) -> Option<LevelFilter> {
    text.split(|c: char| !c.is_ascii_alphabetic())
        .find_map(|word| match word.to_ascii_lowercase().as_str() {
            "trace" | "debug" | "dbg" | "info" | "notice" => {
                Some(LevelFilter::All)
            }
            "warn" | "warning" | "wrn" => Some(LevelFilter::Warn),
            "error" | "err" | "eror" | "fatal" | "crit" | "critical"
            | "panic" | "alert" | "emerg" => Some(LevelFilter::Error),
            _ => None,
        })
}

/// Keep lines at or above `min`. A line with no level word (stack trace,
/// wrapped message) takes the level of the line before it.
#[must_use]
pub fn filter<'a>(
    lines: &'a [Line<'static>],
    min: LevelFilter,
) -> Cow<'a, [Line<'static>]> {
    if min == LevelFilter::All {
        return Cow::Borrowed(lines);
    }
    let mut level = None;
    Cow::Owned(
        lines
            .iter()
            .filter(|line| {
                level = detect(&line.to_string()).or(level);
                level.is_some_and(|l| l >= min)
            })
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(v: &[&str], min: LevelFilter) -> Vec<String> {
        let lines: Vec<Line<'static>> =
            v.iter().map(|s| Line::from((*s).to_string())).collect();
        filter(&lines, min)
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    const INFO: &str = "2024-01-01T00:00:01Z INFO started, error count 0";
    const WARN: &str = "2024-01-01T00:00:02Z WARN slow request";
    const JSON_ERROR: &str = r#"{"level":"error","msg":"boom"}"#;
    const TRACE: &str = "    at main.rs:12";
    const DEBUG: &str = "[debug] tick";
    const FATAL: &str = "E0101 fatal: disk full";
    const LOG: &[&str] = &[INFO, WARN, JSON_ERROR, TRACE, DEBUG, FATAL];

    #[test]
    fn warn_keeps_warn_and_error_with_continuations() {
        assert_eq!(
            texts(LOG, LevelFilter::Warn),
            [WARN, JSON_ERROR, TRACE, FATAL]
        );
    }

    #[test]
    fn error_drops_warn() {
        assert_eq!(texts(LOG, LevelFilter::Error), [JSON_ERROR, TRACE, FATAL]);
    }

    #[test]
    fn all_is_passthrough_and_unlevelled_start_is_hidden() {
        assert_eq!(texts(LOG, LevelFilter::All).len(), LOG.len());
        assert_eq!(
            texts(&["no level here"], LevelFilter::Warn),
            [] as [String; 0]
        );
    }
}
