//! Per-session agent status derived from screen text, hooks and exit.

use std::time::{Instant, SystemTime};

use crate::{agent::AgentProfile, defaults::IDLE_AFTER};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Working,
    Blocked,
    Done,
    Idle,
    Exited(u32),
}

pub struct StatusTracker {
    status: Status,
    last_output: Instant,
    hook_seen: bool,
    /// When the agent exited; the AGE column stops here.
    ended: Option<SystemTime>,
}

impl StatusTracker {
    #[must_use]
    pub const fn new(now: Instant) -> Self {
        Self {
            status: Status::Idle,
            last_output: now,
            hook_seen: false,
            ended: None,
        }
    }

    /// Feed the rendered screen text after new output arrived.
    pub fn observe(&mut self, screen: &str, p: &AgentProfile, now: Instant) {
        if matches!(self.status, Status::Exited(_)) {
            return;
        }
        self.last_output = now;
        if self.hook_seen {
            return;
        }
        let hit = |pats: &[&str]| pats.iter().any(|s| screen.contains(s));
        if hit(p.blocked) {
            self.status = Status::Blocked;
        } else if hit(p.working) {
            self.status = Status::Working;
        }
    }

    /// Quiet `Working` output for `IDLE_AFTER` becomes `Done`.
    pub fn tick(&mut self, now: Instant) {
        // Once a hook has reported, it is the authority on Working/Done.
        if !self.hook_seen
            && self.status == Status::Working
            && now.saturating_duration_since(self.last_output) >= IDLE_AFTER
        {
            self.status = Status::Done;
        }
    }

    /// Agent-reported status; from now on screen matching is ignored.
    pub const fn hook(&mut self, status: Status) {
        if !matches!(self.status, Status::Exited(_)) {
            self.status = status;
            self.hook_seen = true;
        }
    }

    pub fn exited(&mut self, code: u32, at: SystemTime) {
        self.status = Status::Exited(code);
        self.ended.get_or_insert(at);
    }

    #[must_use]
    pub const fn ended(&self) -> Option<SystemTime> {
        self.ended
    }

    #[must_use]
    pub const fn status(&self) -> Status {
        self.status
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::{
        agent::{AgentProfile, profile},
        defaults::IDLE_AFTER,
    };

    fn claude() -> &'static AgentProfile {
        profile("claude").unwrap()
    }

    #[test]
    fn blocked_beats_working() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.observe("esc to interrupt\nDo you want to proceed?", claude(), now);
        assert_eq!(t.status(), Status::Blocked);
    }

    #[test]
    fn working_pattern_sets_working() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.observe("thinking (esc to interrupt)", claude(), now);
        assert_eq!(t.status(), Status::Working);
    }

    #[test]
    fn quiet_working_becomes_done() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.observe("esc to interrupt", claude(), now);
        t.tick(now + IDLE_AFTER);
        assert_eq!(t.status(), Status::Done);
    }

    #[test]
    fn quiet_idle_stays_idle() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.tick(now + IDLE_AFTER);
        assert_eq!(t.status(), Status::Idle);
    }

    #[test]
    fn unmatched_output_keeps_status_and_refreshes_quiet_timer() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.observe("esc to interrupt", claude(), now);
        let later = now + IDLE_AFTER;
        t.observe("something else", claude(), later);
        t.tick(later);
        assert_eq!(t.status(), Status::Working);
    }

    #[test]
    fn hook_overrides_output() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.hook(Status::Done);
        t.observe("esc to interrupt", claude(), now);
        assert_eq!(t.status(), Status::Done);
    }

    #[test]
    fn tick_does_not_override_hook() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.hook(Status::Working);
        t.tick(now + IDLE_AFTER * 2);
        assert_eq!(t.status(), Status::Working);
    }

    #[test]
    fn exited_is_final() {
        let now = Instant::now();
        let mut t = StatusTracker::new(now);
        t.exited(3, SystemTime::UNIX_EPOCH);
        t.observe("esc to interrupt", claude(), now);
        t.hook(Status::Working);
        t.tick(now + IDLE_AFTER);
        assert_eq!(t.status(), Status::Exited(3));
    }
}
