//! Signal menu, safety guards, and human-readable outcomes.

use sysinfo::{SUPPORTED_SIGNALS, Signal};

/// Signals offered by the picker, as supported on the current platform.
#[must_use]
pub const fn menu() -> &'static [Signal] {
    SUPPORTED_SIGNALS
}

/// Conventional `SIGNAME` label for a signal.
#[must_use]
pub const fn label(sig: Signal) -> &'static str {
    match sig {
        Signal::Hangup => "SIGHUP",
        Signal::Interrupt => "SIGINT",
        Signal::Quit => "SIGQUIT",
        Signal::Illegal => "SIGILL",
        Signal::Trap => "SIGTRAP",
        Signal::Abort => "SIGABRT",
        Signal::IOT => "SIGIOT",
        Signal::Bus => "SIGBUS",
        Signal::FloatingPointException => "SIGFPE",
        Signal::Kill => "SIGKILL",
        Signal::User1 => "SIGUSR1",
        Signal::Segv => "SIGSEGV",
        Signal::User2 => "SIGUSR2",
        Signal::Pipe => "SIGPIPE",
        Signal::Alarm => "SIGALRM",
        Signal::Term => "SIGTERM",
        Signal::Child => "SIGCHLD",
        Signal::Continue => "SIGCONT",
        Signal::Stop => "SIGSTOP",
        Signal::TSTP => "SIGTSTP",
        Signal::TTIN => "SIGTTIN",
        Signal::TTOU => "SIGTTOU",
        Signal::Urgent => "SIGURG",
        Signal::XCPU => "SIGXCPU",
        Signal::XFSZ => "SIGXFSZ",
        Signal::VirtualAlarm => "SIGVTALRM",
        Signal::Profiling => "SIGPROF",
        Signal::Winch => "SIGWINCH",
        Signal::IO => "SIGIO",
        Signal::Poll => "SIGPOLL",
        Signal::Power => "SIGPWR",
        Signal::Sys => "SIGSYS",
    }
}

/// Why a signal was refused before it was even attempted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The target is p7s itself.
    SelfProcess,
    /// PID 1 is never signalled.
    Init,
    /// The PID now belongs to a different process (reuse).
    Recycled,
}

/// Result of a signal attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalReport {
    /// Pre-check passed and the caller may send the signal.
    Ready,
    Sent,
    /// The signal does not exist on this platform.
    Unsupported,
    /// No such process (it already exited).
    Gone,
    /// The kernel refused the signal (permissions).
    Denied,
    Refused(Refusal),
}

/// Decide whether signalling `pid` is safe, given the process's current start
/// time (`None` when it no longer exists) and the start time captured when the
/// user picked it.
#[must_use]
pub const fn precheck(
    pid: i32,
    self_pid: i32,
    found_start_time: Option<u64>,
    expected_start_time: u64,
) -> SignalReport {
    if pid == self_pid {
        return SignalReport::Refused(Refusal::SelfProcess);
    }
    if pid == 1 {
        return SignalReport::Refused(Refusal::Init);
    }
    match found_start_time {
        None => SignalReport::Gone,
        Some(t) if t != expected_start_time => {
            SignalReport::Refused(Refusal::Recycled)
        }
        Some(_) => SignalReport::Ready,
    }
}

impl SignalReport {
    /// Status-line text for a report.
    #[must_use]
    pub fn message(self, pid: i32, sig: Signal) -> String {
        let name = label(sig);
        match self {
            Self::Ready => format!("Sending {name} to {pid}"),
            Self::Sent => format!("Sent {name} to {pid}"),
            Self::Unsupported => {
                format!("{name} is not supported on this platform")
            }
            Self::Gone => format!("Process {pid} no longer exists"),
            Self::Denied => format!("Not permitted to signal {pid}"),
            Self::Refused(Refusal::SelfProcess) => {
                "Refusing to signal p7s itself".to_string()
            }
            Self::Refused(Refusal::Init) => {
                "Refusing to signal PID 1".to_string()
            }
            Self::Refused(Refusal::Recycled) => format!(
                "PID {pid} now belongs to a different process; not signalling"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_common_signals() {
        assert_eq!(label(Signal::Term), "SIGTERM");
        assert_eq!(label(Signal::Kill), "SIGKILL");
        assert_eq!(label(Signal::Interrupt), "SIGINT");
        assert_eq!(label(Signal::Hangup), "SIGHUP");
    }

    #[test]
    fn menu_is_non_empty_and_labelled() {
        // SIGKILL is supported on every platform sysinfo supports.
        assert!(menu().contains(&Signal::Kill));
        for sig in menu() {
            assert!(label(*sig).starts_with("SIG"));
        }
    }

    #[test]
    fn precheck_refuses_self_and_init() {
        assert_eq!(
            precheck(7, 7, Some(1), 1),
            SignalReport::Refused(Refusal::SelfProcess)
        );
        assert_eq!(
            precheck(1, 7, Some(1), 1),
            SignalReport::Refused(Refusal::Init)
        );
    }

    #[test]
    fn precheck_detects_gone_and_recycled() {
        assert_eq!(precheck(42, 7, None, 5), SignalReport::Gone);
        assert_eq!(
            precheck(42, 7, Some(6), 5),
            SignalReport::Refused(Refusal::Recycled)
        );
        assert_eq!(precheck(42, 7, Some(5), 5), SignalReport::Ready);
    }

    #[test]
    fn messages_mention_pid_and_signal() {
        assert!(SignalReport::Sent.message(42, Signal::Term).contains("42"));
        assert!(
            SignalReport::Sent
                .message(42, Signal::Term)
                .contains("SIGTERM")
        );
        assert!(
            SignalReport::Refused(Refusal::Init)
                .message(1, Signal::Kill)
                .contains("PID 1")
        );
    }
}
