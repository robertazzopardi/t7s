//! k9s-style ratatui widget kit.

extern crate alloc;

#[cfg(feature = "bg")]
pub mod bg;
pub mod config;
pub mod error;
pub mod event;
pub mod in_flight;
pub mod keymap;
pub mod terminal;
pub mod theme;
pub mod theme_source;
pub mod widgets;

pub use terminal::{TuiApp, launch};
