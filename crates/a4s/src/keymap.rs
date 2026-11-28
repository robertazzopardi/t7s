use k9tui::config::AppSpec;

/// `a4s` definitions live in `config.toml`.
pub const SPEC: AppSpec = AppSpec {
    name: "a4s",
    config: include_str!("config.toml"),
};

#[cfg(test)]
mod tests {
    use k9tui::config::resolve;

    use super::*;

    #[test]
    fn builtin_files_validate() {
        resolve(None, &SPEC, "test").unwrap();
    }
}
