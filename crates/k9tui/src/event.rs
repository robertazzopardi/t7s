//! Terminal input polling for run loops that also service background work.

use core::time::Duration;
use std::{io, time::Instant};

use crossterm::event::poll;

/// How often `poll_input` re-checks `wake` while waiting for input.
const WAKE_CHECK: Duration = Duration::from_millis(10);

/// Waits up to `timeout` for a crossterm input event.
///
/// Returns `Ok(true)` when one is ready to `read`. Returns `Ok(false)` early
/// once `wake()` is true (a background result is queued), so the caller
/// redraws it at once
/// instead of sitting out the rest of the timeout.
///
/// # Errors
///
/// Returns any error from [`crossterm::event::poll`].
pub fn poll_input(
    timeout: Duration,
    mut wake: impl FnMut() -> bool,
) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    while !wake() {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(false);
        }
        if poll(left.min(WAKE_CHECK))? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_immediately_when_woken() {
        let start = Instant::now();
        assert!(!poll_input(Duration::from_secs(5), || true).unwrap());
        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
