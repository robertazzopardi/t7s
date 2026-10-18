//! k9s-style `:` command resolution. Pure: no app state, easy to test.

/// A view/action a bare command word maps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verb {
    Connections,
    Schemas,
    Tables,
    Columns,
    Sql,
    Log,
    Activity,
    Help,
    Quit,
}

/// Every accepted command word, in suggestion order.
const NAMES: &[(&str, Verb)] = &[
    ("connections", Verb::Connections),
    ("conn", Verb::Connections),
    ("schema", Verb::Schemas), // bare `:schema` == `:schemas`; `:schema <name>` opens one
    ("schemas", Verb::Schemas),
    ("table", Verb::Tables), // bare `:table` == `:tables`; `:table <name>` opens one
    ("tables", Verb::Tables),
    ("columns", Verb::Columns),
    ("col", Verb::Columns),
    ("cols", Verb::Columns),
    ("sql", Verb::Sql),
    ("editor", Verb::Sql),
    ("log", Verb::Log),
    ("querylog", Verb::Log),
    ("activity", Verb::Activity),
    ("help", Verb::Help),
    ("quit", Verb::Quit),
    ("q", Verb::Quit),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Blank input: just close the bar.
    Empty,
    Verb(Verb),
    /// Purely numeric: jump to that 1-based row.
    Row(u64),
    /// `:table <name>`: best match first, then the other same-tier candidates.
    Table(Vec<String>),
    /// `:table <name>` matched nothing.
    NoTable(String),
    /// `:schema <name>`: best match first, then the other same-tier candidates.
    Schema(Vec<String>),
    /// `:schema <name>` matched nothing.
    NoSchema(String),
    /// Prefix matches several verbs (canonical names listed).
    Ambiguous(Vec<&'static str>),
    Unknown(String),
}

/// All names matching the query in the best non-empty tier: exact, then
/// prefix, then substring (case-insensitive); list order within a tier.
pub fn match_name_candidates<'a>(
    names: &[&'a str],
    query: &str,
) -> Vec<&'a str> {
    let needle = query.trim().to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let tiers: [&dyn Fn(&str) -> bool; 3] =
        [&|n| n == needle, &|n| n.starts_with(&needle), &|n| {
            n.contains(&needle)
        }];
    for tier in tiers {
        let hits: Vec<&str> = names
            .iter()
            .copied()
            .filter(|n| tier(&n.to_ascii_lowercase()))
            .collect();
        if !hits.is_empty() {
            return hits;
        }
    }
    Vec::new()
}

fn resolve_named_arg(
    arg: &str,
    names: &[&str],
    hit: fn(Vec<String>) -> Command,
    miss: fn(String) -> Command,
) -> Command {
    let arg = arg.trim();
    let hits = match_name_candidates(names, arg);
    if hits.is_empty() {
        miss(arg.into())
    } else {
        hit(hits.into_iter().map(String::from).collect())
    }
}

/// Resolve typed command text. `tables` / `schemas` are used only for the
/// argument of `table <name>` / `schema <name>` (no bare-name shortcuts).
pub fn resolve(input: &str, tables: &[&str], schemas: &[&str]) -> Command {
    let raw = input.trim();
    let lower = raw.to_ascii_lowercase();
    if lower.is_empty() {
        return Command::Empty;
    }
    if lower.bytes().all(|b| b.is_ascii_digit()) {
        return lower
            .parse()
            .map_or_else(|_| Command::Unknown(raw.into()), Command::Row);
    }
    if let Some((word, arg)) = raw.split_once(char::is_whitespace) {
        if word.eq_ignore_ascii_case("table") {
            return resolve_named_arg(
                arg,
                tables,
                Command::Table,
                Command::NoTable,
            );
        }
        if word.eq_ignore_ascii_case("schema") {
            return resolve_named_arg(
                arg,
                schemas,
                Command::Schema,
                Command::NoSchema,
            );
        }
    }
    if let Some((_, verb)) = NAMES.iter().find(|(n, _)| *n == lower) {
        return Command::Verb(*verb);
    }
    let mut prefixed: Vec<(&'static str, Verb)> = Vec::new();
    for (name, verb) in NAMES {
        if name.starts_with(&lower) && !prefixed.iter().any(|(_, v)| v == verb)
        {
            prefixed.push((name, *verb));
        }
    }
    match prefixed.as_slice() {
        [] => Command::Unknown(raw.into()),
        [(_, verb)] => Command::Verb(*verb),
        _ => Command::Ambiguous(prefixed.into_iter().map(|(n, _)| n).collect()),
    }
}

/// Full-text completion: verbs before the first space, then (after `table `
/// / `schema `) the best name prefix match.
pub fn suggest(
    input: &str,
    tables: &[&str],
    schemas: &[&str],
) -> Option<String> {
    if input.is_empty() || input.starts_with(char::is_whitespace) {
        return None;
    }
    let lower = input.to_ascii_lowercase();
    if let Some((word, arg)) = input.split_once(char::is_whitespace) {
        let names = if word.eq_ignore_ascii_case("table") {
            tables
        } else if word.eq_ignore_ascii_case("schema") {
            schemas
        } else {
            return None;
        };
        let arg = arg.trim_start();
        let arg_lower = arg.to_ascii_lowercase();
        let head_len = input.len().saturating_sub(arg.len());
        return names
            .iter()
            .find(|n| {
                n.len() > arg.len()
                    && n.to_ascii_lowercase().starts_with(&arg_lower)
            })
            .map(|n| format!("{}{n}", input.get(..head_len).unwrap_or("")));
    }
    // Jump straight to the argument for arg-taking verbs.
    if lower == "table" || lower == "schema" {
        return Some(format!("{lower} "));
    }
    NAMES
        .iter()
        .map(|(n, _)| *n)
        .find(|n| n.len() > lower.len() && n.starts_with(&lower))
        .map(|n| {
            if n == "table" || n == "schema" {
                format!("{n} ")
            } else {
                n.into()
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: [&str; 4] = ["accounts", "order_items", "orders", "users"];
    const S: [&str; 3] = ["information_schema", "pg_catalog", "public"];

    fn resolve(input: &str, tables: &[&str]) -> Command {
        super::resolve(input, tables, &[])
    }

    fn resolve_s(input: &str, schemas: &[&str]) -> Command {
        super::resolve(input, &[], schemas)
    }

    fn suggest(input: &str, tables: &[&str]) -> Option<String> {
        super::suggest(input, tables, &[])
    }

    fn suggest_s(input: &str, schemas: &[&str]) -> Option<String> {
        super::suggest(input, &[], schemas)
    }

    #[test]
    fn name_match_tiers() {
        let find = |q| match_name_candidates(&T, q).first().copied();
        assert_eq!(find("orders"), Some("orders"));
        assert_eq!(find("ord"), Some("order_items"), "first prefix hit");
        assert_eq!(find("tems"), Some("order_items"));
        assert_eq!(find("USERS"), Some("users"));
        assert_eq!(find("nope"), None);
        assert_eq!(find("usr"), None, "no hardcoded aliases");
        assert_eq!(match_name_candidates(&T, "ord"), ["order_items", "orders"]);
    }

    #[test]
    fn verbs_and_aliases() {
        for (q, v) in [
            ("tables", Verb::Tables),
            ("schema", Verb::Schemas),
            ("schemas", Verb::Schemas),
            ("conn", Verb::Connections),
            ("EDITOR", Verb::Sql),
            ("sql", Verb::Sql),
            ("querylog", Verb::Log),
            ("q", Verb::Quit),
            ("quit", Verb::Quit),
            ("cols", Verb::Columns),
        ] {
            assert_eq!(resolve(q, &[]), Command::Verb(v), "{q}");
        }
    }

    #[test]
    fn unique_prefix_resolves_ambiguous_does_not() {
        assert_eq!(resolve("sch", &[]), Command::Verb(Verb::Schemas));
        assert_eq!(resolve("act", &[]), Command::Verb(Verb::Activity));
        assert_eq!(resolve("l", &[]), Command::Verb(Verb::Log));
        assert_eq!(
            resolve("c", &[]),
            Command::Ambiguous(vec!["connections", "columns"])
        );
    }

    fn tbl(names: &[&str]) -> Command {
        Command::Table(names.iter().map(|n| (*n).into()).collect())
    }

    fn sch(names: &[&str]) -> Command {
        Command::Schema(names.iter().map(|n| (*n).into()).collect())
    }

    #[test]
    fn table_verb_takes_an_argument() {
        assert_eq!(resolve("table ord", &T), tbl(&["order_items", "orders"]));
        assert_eq!(resolve("TABLE  USERS ", &T), tbl(&["users"]));
        assert_eq!(resolve("table\tusers", &T), tbl(&["users"]));
        assert_eq!(resolve("table zzz", &T), Command::NoTable("zzz".into()));
        assert_eq!(resolve("table   ", &T), Command::Verb(Verb::Tables));
        assert_eq!(resolve("table", &T), Command::Verb(Verb::Tables));
        assert_eq!(resolve("tab", &T), Command::Verb(Verb::Tables));
        assert_eq!(resolve("t", &T), Command::Verb(Verb::Tables));
        // Only `table` takes an argument.
        assert_eq!(
            resolve("tables users", &T),
            Command::Unknown("tables users".into())
        );
        assert_eq!(
            resolve("tab users", &T),
            Command::Unknown("tab users".into())
        );
    }

    #[test]
    fn schema_verb_takes_an_argument() {
        assert_eq!(resolve_s("schema pub", &S), sch(&["public"]));
        assert_eq!(resolve_s("schema info", &S), sch(&["information_schema"]));
        assert_eq!(resolve_s("SCHEMA  PUBLIC ", &S), sch(&["public"]));
        assert_eq!(
            resolve_s("schema zzz", &S),
            Command::NoSchema("zzz".into())
        );
        assert_eq!(resolve_s("schema   ", &S), Command::Verb(Verb::Schemas));
        assert_eq!(resolve_s("schema", &S), Command::Verb(Verb::Schemas));
        assert_eq!(
            resolve_s("schemas public", &S),
            Command::Unknown("schemas public".into())
        );
        assert_eq!(
            resolve_s("sch public", &S),
            Command::Unknown("sch public".into())
        );
    }

    #[test]
    fn bare_table_names_no_longer_resolve() {
        assert_eq!(resolve("users", &T), Command::Unknown("users".into()));
        assert_eq!(resolve("ord", &T), Command::Unknown("ord".into()));
        assert_eq!(
            resolve("order items", &T),
            Command::Unknown("order items".into())
        );
        // Verb prefixes are never shadowed by table names.
        assert_eq!(
            resolve("c", &["cats"]),
            Command::Ambiguous(vec!["connections", "columns"])
        );
        assert_eq!(resolve("act", &["act"]), Command::Verb(Verb::Activity));
    }

    #[test]
    fn tables_named_like_verbs_are_reachable() {
        let t = ["log", "act", "tables", "table", "quit", "123"];
        assert_eq!(resolve("table log", &t), tbl(&["log"]));
        assert_eq!(resolve("table act", &t), tbl(&["act"]));
        assert_eq!(resolve("table tables", &t), tbl(&["tables"]));
        assert_eq!(resolve("table table", &t), tbl(&["table"]));
        assert_eq!(resolve("table quit", &t), tbl(&["quit"]));
        assert_eq!(resolve("table 123", &t), tbl(&["123"]));
        // The bare words still mean the verbs.
        assert_eq!(resolve("log", &t), Command::Verb(Verb::Log));
        assert_eq!(resolve("act", &t), Command::Verb(Verb::Activity));
        assert_eq!(resolve("tables", &t), Command::Verb(Verb::Tables));
        assert_eq!(resolve("quit", &t), Command::Verb(Verb::Quit));
        assert_eq!(resolve("123", &t), Command::Row(123));
    }

    #[test]
    fn row_blank_unknown() {
        assert_eq!(resolve("  ", &T), Command::Empty);
        assert_eq!(resolve("123", &T), Command::Row(123));
        assert_eq!(resolve("zzz", &T), Command::Unknown("zzz".into()));
        assert_eq!(
            resolve("99999999999999999999999", &T),
            Command::Unknown("99999999999999999999999".into())
        );
    }

    #[test]
    fn suggestions() {
        assert_eq!(suggest("", &T), None);
        assert_eq!(suggest("tab", &T).as_deref(), Some("table "));
        assert_eq!(suggest("table", &T).as_deref(), Some("table "));
        assert_eq!(suggest("us", &T), None, "tables only after `table `");
        assert_eq!(suggest("tables", &T), None, "already complete");
        assert_eq!(suggest("zz", &T), None);
        assert_eq!(suggest("sc", &T).as_deref(), Some("schema "));
        assert_eq!(suggest_s("schema", &S).as_deref(), Some("schema "));
    }

    #[test]
    fn argument_suggestions() {
        assert_eq!(suggest("table us", &T).as_deref(), Some("table users"));
        assert_eq!(
            suggest("table ord", &T).as_deref(),
            Some("table order_items")
        );
        assert_eq!(suggest("TABLE  US", &T).as_deref(), Some("TABLE  users"));
        assert_eq!(suggest("table ", &T).as_deref(), Some("table accounts"));
        assert_eq!(suggest("table users", &T), None, "already complete");
        assert_eq!(suggest("table zz", &T), None);
        assert_eq!(suggest("table tems", &T), None, "prefix only");
        assert_eq!(suggest("tables us", &T), None, "only `table` takes args");
        assert_eq!(suggest("table us", &[]), None);
        assert_eq!(
            suggest_s("schema pu", &S).as_deref(),
            Some("schema public")
        );
        assert_eq!(
            suggest_s("schema ", &S).as_deref(),
            Some("schema information_schema")
        );
        assert_eq!(suggest_s("schemas pu", &S), None);
    }

    #[test]
    fn whitespace_and_case_are_normalised() {
        assert_eq!(resolve("  tables  ", &T), Command::Verb(Verb::Tables));
        assert_eq!(resolve("\ttables\n", &T), Command::Verb(Verb::Tables));
        assert_eq!(resolve("\t \n", &T), Command::Empty);
        assert_eq!(resolve("QuIt", &T), Command::Verb(Verb::Quit));
        assert_eq!(resolve("  table  USERS ", &T), tbl(&["users"]));
    }

    #[test]
    fn numeric_edges() {
        assert_eq!(resolve("0", &T), Command::Row(0));
        assert_eq!(resolve("007", &T), Command::Row(7));
        assert_eq!(resolve(" 42 ", &T), Command::Row(42));
        assert_eq!(resolve("18446744073709551615", &T), Command::Row(u64::MAX));
        for bad in ["18446744073709551616", "-1", "+1", "1.5", "1_0", "１２"]
        {
            assert_eq!(resolve(bad, &T), Command::Unknown(bad.into()), "{bad}");
        }
        // A table literally named like a number does not beat the row jump...
        assert_eq!(resolve("2024", &["2024"]), Command::Row(2024));
        // ...but is reachable through `table`.
        assert_eq!(resolve("table 2024", &["2024"]), tbl(&["2024"]));
    }

    #[test]
    fn multibyte_input_never_panics() {
        let tables = ["Straße", "表格", "naïve", "📦boxes", "users"];
        for q in [
            "🦀", "é", "表", "表格", "ß", "📦", "ï", "\u{200d}", "a\u{301}",
        ] {
            for input in [q.to_string(), format!("table {q}")] {
                let _ = resolve(&input, &tables);
                let _ = suggest(&input, &tables);
            }
            let _ = match_name_candidates(&tables, q);
        }
        assert_eq!(resolve("table 表", &tables), tbl(&["表格"]));
        assert_eq!(resolve("table 📦", &tables), tbl(&["📦boxes"]));
        assert_eq!(resolve("table naïve", &tables), tbl(&["naïve"]));
        // ASCII-only folding: non-ASCII case differences do not match.
        assert_eq!(
            resolve("table NAÏVE", &tables),
            Command::NoTable("NAÏVE".into())
        );
        assert_eq!(resolve("表", &tables), Command::Unknown("表".into()));
        assert_eq!(suggest("table 表", &tables).as_deref(), Some("table 表格"));
        assert_eq!(
            suggest("table 📦", &tables).as_deref(),
            Some("table 📦boxes")
        );
        assert_eq!(suggest("table 🦀", &tables), None);
        assert_eq!(suggest("🦀", &tables), None);
    }

    #[test]
    fn table_names_with_spaces_quotes_and_uppercase() {
        let tables = ["Order Items", "O'Brien", "say \"hi\"", "UPPER", "a.b"];
        assert_eq!(
            resolve("table order items", &tables),
            tbl(&["Order Items"])
        );
        assert_eq!(resolve("table   order   ", &tables), tbl(&["Order Items"]));
        assert_eq!(resolve("table o'br", &tables), tbl(&["O'Brien"]));
        assert_eq!(resolve("table say \"hi\"", &tables), tbl(&["say \"hi\""]));
        assert_eq!(resolve("table upper", &tables), tbl(&["UPPER"]));
        assert_eq!(resolve("table a.b", &tables), tbl(&["a.b"]));
        assert_eq!(
            suggest("table order i", &tables).as_deref(),
            Some("table Order Items")
        );
        assert_eq!(
            suggest("table up", &tables).as_deref(),
            Some("table UPPER")
        );
    }

    #[test]
    fn suggest_edge_cases() {
        assert_eq!(suggest(" ", &T), None);
        assert_eq!(suggest(" tab", &T), None, "leading space: no ghost");
        assert_eq!(suggest("sch ", &T), None, "trailing space: no ghost");
        assert_eq!(suggest("TAB", &T).as_deref(), Some("table "));
        assert_eq!(suggest("x", &[]), None);
        assert_eq!(suggest("c", &["cats"]).as_deref(), Some("connections"));
    }
}
