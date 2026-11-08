//! Today's default hotkey bars, so moving their definitions can't change them.
use std::fmt::Write as _;

use k9tui::{config::defaults, widgets::hotkey::Hotkey};

use crate::{
    keymap::SPEC,
    ui::widgets::hotkeys::{
        GLOBAL_HOTKEYS, LIST_HOTKEYS, RESOURCE_HOTKEYS, VIEW_SWITCH_HOTKEYS,
        describe_hotkeys, log_hotkeys, process_hotkeys,
    },
};

fn section(out: &mut String, title: &str, hotkeys: &[Hotkey]) {
    writeln!(out, "[{title}]").unwrap();
    for hotkey in defaults(&SPEC).keymap.relabel(hotkeys) {
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
    section(&mut out, "list", &LIST_HOTKEYS);
    section(&mut out, "resources", &RESOURCE_HOTKEYS);
    section(&mut out, "views", &VIEW_SWITCH_HOTKEYS);
    section(&mut out, "global", &GLOBAL_HOTKEYS);
    section(&mut out, "logs", &log_hotkeys());
    section(&mut out, "processes", &process_hotkeys());
    section(&mut out, "describe", &describe_hotkeys());
    assert_eq!(out, include_str!("default_bars.golden"));
}
