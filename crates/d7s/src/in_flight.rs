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

pub fn connection_key(id: &str) -> String {
    format!("connection:{id}")
}

/// Health ping; separate from `connection_key` so a slow launch ping does not
/// refuse the user's own connect.
pub fn ping_key(id: &str) -> String {
    format!("ping:{id}")
}

/// Explorer keys carry their connection, so results from a connection the user
/// has since left are recognised (see [`key_connection`]) and the same table on
/// another connection is not reported busy.
fn scoped(connection_name: &str, rest: &str) -> String {
    format!("{connection_name}\0{rest}")
}

/// Connection a scoped key belongs to; `None` for unscoped keys.
pub fn key_connection(key: &str) -> Option<&str> {
    key.split_once('\0').map(|(c, _)| c)
}

pub fn database_key(connection_name: &str) -> String {
    scoped(connection_name, "databases")
}

pub fn select_database_key(
    connection_name: &str,
    database_name: &str,
) -> String {
    scoped(connection_name, &format!("select_database:{database_name}"))
}

pub fn describe_key() -> String {
    "describe".into()
}

pub fn connection_test_key(connection_name: &str) -> String {
    format!("connection_test:{connection_name}")
}

pub fn schemas_key(connection_name: &str) -> String {
    scoped(connection_name, "schemas")
}

pub fn schema_key(connection_name: &str, schema: &str) -> String {
    scoped(connection_name, &format!("schema:{schema}"))
}

pub fn table_key(connection_name: &str, schema: &str, table: &str) -> String {
    scoped(connection_name, &format!("table:{schema}.{table}"))
}

pub fn sql_key(connection_name: &str) -> String {
    scoped(connection_name, "sql")
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
    fn explorer_keys_are_scoped_by_connection() {
        let mut f = InFlight::default();
        assert!(f.try_begin(table_key("a", "public", "users")));
        assert!(f.try_begin(table_key("b", "public", "users")));
        assert_eq!(key_connection(&table_key("a", "s", "t")), Some("a"));
        assert_eq!(key_connection(&sql_key("b")), Some("b"));
        assert_eq!(key_connection(&connection_key("a")), None);
        assert_ne!(ping_key("a"), connection_key("a"));
    }

    #[test]
    fn end_on_missing_key_is_ok() {
        let mut f = InFlight::default();
        f.end("connection:missing");
    }
}
