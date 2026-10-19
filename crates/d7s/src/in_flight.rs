use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct InFlight {
    keys: HashSet<String>,
}

impl InFlight {
    #[allow(dead_code)] // callers in Tasks 9–12
    pub fn try_begin(&mut self, key: impl Into<String>) -> bool {
        self.keys.insert(key.into())
    }

    pub fn end(&mut self, key: &str) {
        self.keys.remove(key);
    }

    #[allow(dead_code)] // callers in Tasks 9–12
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }
}

#[allow(dead_code)] // callers in Tasks 9–12
pub fn connection_key(id: &str) -> String {
    format!("connection:{id}")
}

#[allow(dead_code)]
pub fn database_key(name: &str) -> String {
    format!("database:{name}")
}

#[allow(dead_code)]
pub fn table_key(schema: &str, table: &str) -> String {
    format!("table:{schema}.{table}")
}

#[allow(dead_code)]
pub fn sql_key(connection_id: &str) -> String {
    format!("sql:{connection_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_begin_refuses_duplicate() {
        let mut f = InFlight::default();
        assert!(f.try_begin(connection_key("abc")));
        assert!(!f.try_begin(connection_key("abc")));
        assert!(f.try_begin(connection_key("def")));
    }

    #[test]
    fn end_clears_so_retry_works() {
        let mut f = InFlight::default();
        let k = connection_key("abc");
        assert!(f.try_begin(&k));
        f.end(&k);
        assert!(f.try_begin(&k));
    }

    #[test]
    fn end_on_missing_key_is_ok() {
        let mut f = InFlight::default();
        f.end("connection:missing");
    }
}
