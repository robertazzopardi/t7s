pub use k9tui::in_flight::InFlight;

use crate::app_state::ResourceKind;

pub fn container_key(id: &str) -> String {
    format!("container:{id}")
}

pub fn image_key(id: &str) -> String {
    format!("image:{id}")
}

pub fn volume_key(name: &str) -> String {
    format!("volume:{name}")
}

pub fn network_key(id: &str) -> String {
    format!("network:{id}")
}

pub const DAEMON_KEY: &str = "daemon";

pub fn resource_key(kind: ResourceKind, id: &str) -> String {
    match kind {
        ResourceKind::Containers => container_key(id),
        ResourceKind::Images => image_key(id),
        ResourceKind::Volumes => volume_key(id),
        ResourceKind::Networks => network_key(id),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_begin_refuses_duplicate() {
        let mut f = InFlight::default();
        assert!(f.try_begin(container_key("abc")));
        assert!(!f.try_begin(container_key("abc")));
        assert!(f.try_begin(container_key("def")));
    }

    #[test]
    fn end_clears_so_retry_works() {
        let mut f = InFlight::default();
        let k = container_key("abc");
        assert!(f.try_begin(&k));
        f.end(&k);
        assert!(f.try_begin(&k));
    }

    #[test]
    fn end_on_missing_key_is_ok() {
        let mut f = InFlight::default();
        f.end("container:missing");
    }
}
