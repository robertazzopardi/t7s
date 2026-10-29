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
