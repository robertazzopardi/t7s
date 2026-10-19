use std::collections::HashSet;

#[derive(Debug, Default)]
pub struct InFlight {
    keys: HashSet<String>,
}

impl InFlight {
    pub fn try_begin(&mut self, key: impl Into<String>) -> bool {
        self.keys.insert(key.into())
    }

    pub fn end(&mut self, key: &str) {
        self.keys.remove(key);
    }

    #[cfg(test)]
    pub fn contains(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[allow(dead_code)] // Tasks 10–12
pub fn connection_key(id: &str) -> String {
    format!("connection:{id}")
}

pub fn database_key(name: &str) -> String {
    format!("database:{name}")
}

pub fn select_database_key(connection_name: &str, database_name: &str) -> String {
    format!("select_database:{connection_name}:{database_name}")
}

pub fn describe_key() -> String {
    "describe".into()
}

pub fn connection_test_key(connection_name: &str) -> String {
    format!("connection_test:{connection_name}")
}

pub fn schemas_key(connection_name: &str) -> String {
    format!("schemas:{connection_name}")
}

pub fn schema_key(schema: &str) -> String {
    format!("schema:{schema}")
}

pub fn table_key(schema: &str, table: &str) -> String {
    format!("table:{schema}.{table}")
}

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
