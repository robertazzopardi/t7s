//! Collapse consecutive duplicate / near-duplicate log lines into one row
//! with a `×N` suffix.

use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
};

use k9tui::theme;
use ratatui::text::{Line, Span};

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum DedupMode {
    Off,
    /// Identical adjacent lines.
    #[default]
    Exact,
    /// Adjacent lines equal after masking numbers/ids.
    Similar,
}

impl DedupMode {
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Exact,
            Self::Exact => Self::Similar,
            Self::Similar => Self::Off,
        }
    }
}

impl Display for DedupMode {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Off => "off",
            Self::Exact => "exact",
            Self::Similar => "similar",
        })
    }
}

/// Replace every alphanumeric token containing a digit with `#`, so
/// timestamps, counters, IPs, ports and hex/uuid chunks compare equal.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if token.chars().any(|c| c.is_ascii_digit()) {
            out.push('#');
        } else {
            out.push_str(token);
        }
        token.clear();
    };
    for c in text.chars() {
        if c.is_alphanumeric() {
            token.push(c);
        } else {
            flush(&mut token, &mut out);
            out.push(c);
        }
    }
    flush(&mut token, &mut out);
    out
}

/// Fold adjacent matching lines; the last line of a run is shown.
// ponytail: recomputed per frame over <=5000 lines; cache keys at ingest if profiling shows cost.
#[must_use]
pub fn fold<'a>(
    lines: &'a [Line<'static>],
    mode: DedupMode,
) -> Cow<'a, [Line<'static>]> {
    if mode == DedupMode::Off {
        return Cow::Borrowed(lines);
    }
    let key = |l: &Line| {
        let text = l.to_string();
        if mode == DedupMode::Similar {
            normalize(&text)
        } else {
            text
        }
    };
    let mut out: Vec<(Line<'static>, usize)> = Vec::new();
    let mut last_key = String::new();
    for line in lines {
        let k = key(line);
        match out.last_mut() {
            Some((prev, n)) if k == last_key => {
                *prev = line.clone();
                *n += 1;
            }
            _ => {
                out.push((line.clone(), 1));
                last_key = k;
            }
        }
    }
    Cow::Owned(
        out.into_iter()
            .map(|(mut line, n)| {
                if n > 1 {
                    line.spans
                        .push(Span::styled(format!("  ×{n}"), theme::muted()));
                }
                line
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<Line<'static>> {
        v.iter().map(|s| Line::from((*s).to_string())).collect()
    }

    fn texts(c: &[Line<'static>]) -> Vec<String> {
        c.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn exact_folds_only_adjacent() {
        let l = lines(&["a", "a", "b", "a"]);
        assert_eq!(texts(&fold(&l, DedupMode::Exact)), ["a  ×2", "b", "a"]);
    }

    #[test]
    fn similar_masks_numbers_and_keeps_latest() {
        let l = lines(&[
            "2024-01-01T00:00:01Z req 12 took 4ms",
            "2024-01-01T00:00:02Z req 13 took 9ms",
            "other",
        ]);
        assert_eq!(
            texts(&fold(&l, DedupMode::Similar)),
            ["2024-01-01T00:00:02Z req 13 took 9ms  ×2", "other"]
        );
        assert_eq!(fold(&l, DedupMode::Exact).len(), 3);
    }

    #[test]
    fn off_is_passthrough() {
        let l = lines(&["a", "a"]);
        assert_eq!(fold(&l, DedupMode::Off).len(), 2);
    }
}
