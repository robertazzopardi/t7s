//! Today's default hotkey bars, so moving their definitions can't change them.
use std::fmt::Write as _;

use k9tui::config::defaults;

use crate::keymap::SPEC;

fn section(out: &mut String, title: &str) {
    writeln!(out, "[{title}]").unwrap();
    for hotkey in defaults(&SPEC).bar(title) {
        writeln!(
            out,
            "{} {}",
            hotkey.key_label(),
            hotkey.description.display_suffix()
        )
        .unwrap();
    }
}

#[test]
fn default_bars_are_unchanged() {
    let mut out = String::new();
    section(&mut out, "list");
    section(&mut out, "resources");
    section(&mut out, "views");
    section(&mut out, "global");
    section(&mut out, "logs");
    section(&mut out, "processes");
    section(&mut out, "describe");
    insta::assert_snapshot!(out);
}
