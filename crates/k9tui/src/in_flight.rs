//! Keys for background work already running, so a repeat request is refused.

use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct InFlight {
    keys: HashSet<String>,
}

impl InFlight {
    /// `false` when `key` is already in flight.
    pub fn try_begin(&mut self, key: impl Into<String>) -> bool {
        self.keys.insert(key.into())
    }

    pub fn end(&mut self, key: &str) {
        self.keys.remove(key);
    }

    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_begin_refuses_duplicate() {
        let mut f = InFlight::default();
        assert!(f.try_begin("a"));
        assert!(!f.try_begin("a"));
        assert!(f.try_begin("b"));
    }

    #[test]
    fn end_clears_so_retry_works() {
        let mut f = InFlight::default();
        assert!(f.try_begin("a"));
        f.end("a");
        assert!(f.try_begin("a"));
    }

    #[test]
    fn end_on_missing_key_is_ok() {
        let mut f = InFlight::default();
        f.end("missing");
        assert!(f.is_empty());
    }
}
