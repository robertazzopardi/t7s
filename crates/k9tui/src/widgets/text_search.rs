use alloc::borrow::Cow;

use ratatui::text::{Line, Span};

use crate::theme;

/// Lines matching `query` (case-insensitive substring), with the match
/// highlighted. Empty query returns `lines` unchanged, with no copy.
///
/// Matching is ASCII-only: `to_ascii_lowercase` keeps byte offsets aligned
/// between the original and folded text, which a full Unicode
/// `to_lowercase` cannot guarantee (case folding can change byte length).
#[must_use]
pub fn filter_and_highlight<'a, 'b>(
    lines: &'b [Line<'a>],
    query: &str,
) -> Cow<'b, [Line<'a>]> {
    if query.is_empty() {
        return Cow::Borrowed(lines);
    }
    let needle = query.to_ascii_lowercase();
    Cow::Owned(
        lines
            .iter()
            .filter_map(|line| highlight_line(line, &needle))
            .collect(),
    )
}

/// Returns `None` if no span in `line` contains `needle`.
fn highlight_line<'a>(line: &Line<'a>, needle: &str) -> Option<Line<'a>> {
    let mut spans = Vec::new();
    let mut matched_any = false;
    for span in &line.spans {
        let text = span.content.as_ref();
        let lower = text.to_ascii_lowercase();
        let mut rest = text;
        let mut lower_rest = lower.as_str();
        let mut offset = 0;
        while let Some(pos) = lower_rest.find(needle) {
            matched_any = true;
            let byte_pos = offset + pos;
            let before = &text[offset..byte_pos];
            if !before.is_empty() {
                spans.push(Span::styled(before.to_string(), span.style));
            }
            let matched = &text[byte_pos..byte_pos + needle.len()];
            spans.push(Span::styled(matched.to_string(), theme::accent()));
            offset = byte_pos + needle.len();
            rest = &text[offset..];
            lower_rest = &lower[offset..];
        }
        if !rest.is_empty() {
            spans.push(Span::styled(rest.to_string(), span.style));
        }
    }
    matched_any.then(|| Line::from(spans))
}
