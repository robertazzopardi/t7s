use core::time::Duration;
use std::{path::PathBuf, time::SystemTime};

use k9tui::{theme, widgets::table::TableData};
use ratatui::style::Style;

use crate::{
    agent::AgentProfile,
    defaults::{
        GLYPH_BLOCKED, GLYPH_DONE, GLYPH_EXITED, GLYPH_IDLE, GLYPH_WORKING,
    },
    pty::{Pty, SessionId},
    status::{Status, StatusTracker},
};

const COLUMNS: [&str; 6] = ["#", "ST", "NAME", "AGENT", "DIR", "AGE"];
const ST_COLUMN: usize = 1;

/// A running (or exited) agent. `pty` stays `Some` after the child exits so
/// its last screen remains visible; it is `None` after a kill.
pub struct Session {
    pub id: SessionId,
    pub name: String,
    pub agent: &'static AgentProfile,
    pub dir: PathBuf,
    pub pty: Option<Pty>,
    pub status: StatusTracker,
    pub created: SystemTime,
    /// Git repo containing `dir`, computed once at create.
    pub repo: Option<PathBuf>,
}

impl Session {
    #[must_use]
    pub const fn is_exited(&self) -> bool {
        matches!(self.status.status(), Status::Exited(_))
    }

    /// Table row; `number` is the 1-based `#` (also the `Ctrl+N` number).
    #[must_use]
    pub fn row(&self, number: usize, now: SystemTime) -> SessionRow {
        let status = self.status.status();
        SessionRow {
            index: number.to_string(),
            state: glyph(status),
            blocked: status == Status::Blocked,
            name: self.name.clone(),
            agent: self.agent.name.to_string(),
            dir: self.dir.display().to_string(),
            age: age(now.duration_since(self.created).unwrap_or_default()),
        }
    }
}

#[must_use]
pub fn glyph(status: Status) -> String {
    match status {
        Status::Working => GLYPH_WORKING.into(),
        Status::Blocked => GLYPH_BLOCKED.into(),
        Status::Done => GLYPH_DONE.into(),
        Status::Idle => GLYPH_IDLE.into(),
        Status::Exited(code) => format!("{GLYPH_EXITED} {code}"),
    }
}

fn age(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}

/// One row in the sessions table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionRow {
    pub index: String,
    pub state: String,
    pub name: String,
    pub agent: String,
    pub dir: String,
    pub age: String,
    /// Blocked glyphs are drawn in the warning colour.
    pub blocked: bool,
}

impl TableData for SessionRow {
    fn title() -> &'static str {
        "Sessions"
    }

    fn ref_array(&self) -> Vec<String> {
        vec![
            self.index.clone(),
            self.state.clone(),
            self.name.clone(),
            self.agent.clone(),
            self.dir.clone(),
            self.age.clone(),
        ]
    }

    fn num_columns(&self) -> usize {
        COLUMNS.len()
    }

    fn cols() -> Vec<&'static str> {
        COLUMNS.to_vec()
    }

    fn cell_style(&self, column: usize) -> Option<Style> {
        (self.blocked && column == ST_COLUMN).then(theme::warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn age_units() {
        assert_eq!(age(Duration::from_secs(5)), "5s");
        assert_eq!(age(Duration::from_mins(2)), "2m");
        assert_eq!(age(Duration::from_hours(2)), "2h");
        assert_eq!(age(Duration::from_hours(48)), "2d");
    }

    #[test]
    fn exited_glyph_has_code() {
        assert_eq!(glyph(Status::Exited(3)), "✗ 3");
    }
}
