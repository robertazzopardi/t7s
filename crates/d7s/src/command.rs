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

/// Names the command bar matches and completes against.
#[derive(Debug, Default, Clone, Copy)]
pub struct Scope<'a> {
    /// Tables of the current schema.
    pub tables: &'a [&'a str],
    /// Schemas of the current database.
    pub schemas: &'a [&'a str],
    /// Databases of the current connection.
    pub databases: &'a [&'a str],
    /// Saved connections as `(name, is_sqlite)`.
    pub connections: &'a [(&'a str, bool)],
    /// Current connection, database and schema (`""` when none).
    pub conn: &'a str,
    pub db: &'a str,
    pub schema: &'a str,
}

/// `:table conn.db.schema.table` (SQLite: `conn.table`). `conn` is the
/// resolved connection name; the other parts are matched as each level loads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TablePath {
    pub conn: String,
    /// `None` for SQLite (no database/schema levels).
    pub db: Option<String>,
    pub schema: Option<String>,
    pub table: String,
}

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
    /// `:table conn.db.schema.table`: jump across connections/databases.
    TablePath(TablePath),
    /// The path's first segment matched no saved connection.
    NoConnection(String),
    /// Dotted path with the wrong number of parts for its connection type.
    BadTablePath(String),
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

/// Split `conn.rest`: the longest saved connection name followed by `.` wins
/// (names may contain dots); with `fuzzy`, else match the first segment.
fn split_conn<'n, 'a>(
    arg: &'a str,
    connections: &[(&'n str, bool)],
    fuzzy: bool,
) -> Option<(&'n str, bool, &'a str)> {
    let exact = connections
        .iter()
        .filter_map(|&(name, sqlite)| {
            let head = arg.get(..name.len())?;
            let rest = arg.get(name.len()..)?.strip_prefix('.')?;
            head.eq_ignore_ascii_case(name)
                .then_some((name, sqlite, rest))
        })
        .max_by_key(|(name, ..)| name.len());
    if exact.is_some() || !fuzzy {
        return exact;
    }
    let (head, rest) = arg.split_once('.')?;
    let names: Vec<&str> = connections.iter().map(|c| c.0).collect();
    let hit = *match_name_candidates(&names, head).first()?;
    let &(name, sqlite) = connections.iter().find(|c| c.0 == hit)?;
    Some((name, sqlite, rest))
}

/// Connected to a server (non-SQLite) connection: relative paths allowed.
fn in_server(scope: &Scope) -> bool {
    !scope.conn.is_empty()
        && scope
            .connections
            .iter()
            .any(|&(name, sqlite)| name == scope.conn && !sqlite)
}

/// Path parts after `conn`, or `None` when the count doesn't fit its type.
fn table_path(conn: &str, sqlite: bool, parts: &[&str]) -> Option<TablePath> {
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let (db, schema, table) = match (sqlite, parts) {
        (true, [table]) => (None, None, table),
        (false, [db, schema, table]) => (Some(db), Some(schema), table),
        _ => return None,
    };
    Some(TablePath {
        conn: conn.into(),
        db: db.map(|d| (*d).into()),
        schema: schema.map(|s| (*s).into()),
        table: (*table).into(),
    })
}

fn path_parts(s: &str) -> Vec<&str> {
    s.split('.').map(str::trim).collect()
}

/// `table <arg>`: a current-schema table name, or a dotted path. A table
/// literally named with a dot wins when typed exactly. Inside a server
/// connection, `schema.table` and `db.schema.table` are relative to it,
/// unless the path starts with an exact connection name and fits it.
fn resolve_table_arg(arg: &str, scope: &Scope) -> Command {
    let arg = arg.trim();
    if !arg.contains('.')
        || scope.tables.iter().any(|t| t.eq_ignore_ascii_case(arg))
    {
        return resolve_named_arg(
            arg,
            scope.tables,
            Command::Table,
            Command::NoTable,
        );
    }
    if let Some(path) = split_conn(arg, scope.connections, false).and_then(
        |(conn, sqlite, rest)| table_path(conn, sqlite, &path_parts(rest)),
    ) {
        return Command::TablePath(path);
    }
    let parts = path_parts(arg);
    if in_server(scope) && (2..=3).contains(&parts.len()) {
        if parts.iter().any(|p| p.is_empty()) {
            return Command::BadTablePath(arg.into());
        }
        let (db, rest) = match parts.as_slice() {
            [db, rest @ ..] if parts.len() == 3 => (Some(*db), rest),
            rest => ((!scope.db.is_empty()).then_some(scope.db), rest),
        };
        let [schema, table] = rest else {
            return Command::BadTablePath(arg.into());
        };
        return Command::TablePath(TablePath {
            conn: scope.conn.into(),
            db: db.map(String::from),
            schema: Some((*schema).into()),
            table: (*table).into(),
        });
    }
    match split_conn(arg, scope.connections, true) {
        None => {
            let head = arg.split('.').next().unwrap_or(arg);
            Command::NoConnection(head.into())
        }
        Some((conn, sqlite, rest)) => {
            table_path(conn, sqlite, &path_parts(rest)).map_or_else(
                || Command::BadTablePath(arg.into()),
                Command::TablePath,
            )
        }
    }
}

/// First of `names` extending `typed` (case-insensitive). `more`: a deeper
/// level follows, so also complete an exact name and append the `.`.
fn complete_name(names: &[&str], typed: &str, more: bool) -> Option<String> {
    let typed_lower = typed.to_ascii_lowercase();
    names
        .iter()
        .find(|n| {
            let l = n.to_ascii_lowercase();
            l.starts_with(&typed_lower)
                && (n.len() > typed.len() || (more && l == typed_lower))
        })
        .map(|n| format!("{n}{}", if more { "." } else { "" }))
}

/// Complete the last of `parts`, typed against `levels`. Only the current
/// context's names are loaded, so earlier parts must equal `context`.
fn complete_level(
    levels: &[&[&str]],
    context: &[&str],
    parts: &[&str],
) -> Option<String> {
    let (typed, parents) = parts.split_last()?;
    let names = levels.get(parents.len())?;
    if !parents
        .iter()
        .zip(context)
        .all(|(p, c)| p.eq_ignore_ascii_case(c))
    {
        return None;
    }
    complete_name(names, typed, parents.len() + 1 < levels.len())
}

/// Completion for `table <arg>`: current tables, then connection names, then
/// each path level whose parents are the current context (absolute first,
/// then relative `schema.table` / `db.schema.table` inside a server).
fn suggest_table_arg(arg: &str, scope: &Scope) -> Option<String> {
    if let Some(t) = complete_name(scope.tables, arg, false) {
        return Some(t);
    }
    let typed = arg.rsplit('.').next()?;
    let done = arg.get(..arg.len() - typed.len())?;
    let server = [scope.databases, scope.schemas, scope.tables];
    let absolute = split_conn(arg, scope.connections, false)
        .filter(|(conn, ..)| conn.eq_ignore_ascii_case(scope.conn))
        .and_then(|(_, sqlite, rest)| {
            let parts: Vec<&str> = rest.split('.').collect();
            if sqlite {
                complete_level(&[scope.tables], &[], &parts)
            } else {
                complete_level(&server, &[scope.db, scope.schema], &parts)
            }
        });
    let completed = absolute
        .or_else(|| {
            let names: Vec<&str> =
                scope.connections.iter().map(|c| c.0).collect();
            (!arg.contains('.'))
                .then(|| complete_name(&names, arg, true))
                .flatten()
        })
        .or_else(|| {
            let parts: Vec<&str> = arg.split('.').collect();
            in_server(scope).then(|| {
                complete_level(
                    &[scope.schemas, scope.tables],
                    &[scope.schema],
                    &parts,
                )
                .or_else(|| {
                    complete_level(&server, &[scope.db, scope.schema], &parts)
                })
            })?
        })?;
    // `completed` replaces only the last typed part.
    Some(format!("{done}{completed}"))
}

/// Resolve typed command text. `scope` names are used only for the argument
/// of `table <name>` / `schema <name>` (no bare-name shortcuts).
pub fn resolve(input: &str, scope: &Scope) -> Command {
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
            return resolve_table_arg(arg, scope);
        }
        if word.eq_ignore_ascii_case("schema") {
            return resolve_named_arg(
                arg,
                scope.schemas,
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
pub fn suggest(input: &str, scope: &Scope) -> Option<String> {
    if input.is_empty() || input.starts_with(char::is_whitespace) {
        return None;
    }
    let lower = input.to_ascii_lowercase();
    if let Some((word, arg)) = input.split_once(char::is_whitespace) {
        let arg = arg.trim_start();
        let arg_lower = arg.to_ascii_lowercase();
        let head = input.get(..input.len().saturating_sub(arg.len()))?;
        let completed = if word.eq_ignore_ascii_case("table") {
            suggest_table_arg(arg, scope)?
        } else if word.eq_ignore_ascii_case("schema") {
            (*scope.schemas.iter().find(|n| {
                n.len() > arg.len()
                    && n.to_ascii_lowercase().starts_with(&arg_lower)
            })?)
            .to_string()
        } else {
            return None;
        };
        return Some(format!("{head}{completed}"));
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
        super::resolve(
            input,
            &Scope {
                tables,
                ..Scope::default()
            },
        )
    }

    fn resolve_s(input: &str, schemas: &[&str]) -> Command {
        super::resolve(
            input,
            &Scope {
                schemas,
                ..Scope::default()
            },
        )
    }

    fn suggest(input: &str, tables: &[&str]) -> Option<String> {
        super::suggest(
            input,
            &Scope {
                tables,
                ..Scope::default()
            },
        )
    }

    fn suggest_s(input: &str, schemas: &[&str]) -> Option<String> {
        super::suggest(
            input,
            &Scope {
                schemas,
                ..Scope::default()
            },
        )
    }

    const CONNS: [(&str, bool); 4] = [
        ("pg", false),
        ("pg.prod", false),
        ("lite", true),
        ("mysql-local", false),
    ];

    /// Connected to `pg`, inside database `app`, schema `public`.
    fn in_pg() -> Scope<'static> {
        Scope {
            tables: &T,
            schemas: &S,
            databases: &["analytics", "app"],
            connections: &CONNS,
            conn: "pg",
            db: "app",
            schema: "public",
        }
    }

    fn path(conn: &str, db: Option<&str>, s: Option<&str>, t: &str) -> Command {
        Command::TablePath(TablePath {
            conn: conn.into(),
            db: db.map(String::from),
            schema: s.map(String::from),
            table: t.into(),
        })
    }

    #[test]
    fn qualified_table_paths() {
        let sc = in_pg();
        let r = |q: &str| super::resolve(q, &sc);
        assert_eq!(
            r("table pg.app.public.users"),
            path("pg", Some("app"), Some("public"), "users")
        );
        // Longest exact connection name wins; names may contain dots.
        assert_eq!(
            r("table pg.prod.db.s.t"),
            path("pg.prod", Some("db"), Some("s"), "t")
        );
        // Fuzzy first segment; parts after it are kept raw.
        assert_eq!(
            r("table MYSQL.Shop.Shop.Orders"),
            path("mysql-local", Some("Shop"), Some("Shop"), "Orders")
        );
        assert_eq!(r("table lite.notes"), path("lite", None, None, "notes"));
        assert_eq!(r("table nope.a.b.c"), Command::NoConnection("nope".into()));
        for bad in ["pg.a.b.c.d", "pg..b.c", "public.", "a..t"] {
            assert_eq!(
                r(&format!("table {bad}")),
                Command::BadTablePath(bad.into()),
                "{bad}"
            );
        }
        // Inside a server connection: relative to the current db / conn.
        assert_eq!(
            r("table public.users"),
            path("pg", Some("app"), Some("public"), "users")
        );
        assert_eq!(
            r("table analytics.events.clicks"),
            path("pg", Some("analytics"), Some("events"), "clicks")
        );
        // An exact connection prefix that fits its depth stays absolute.
        assert_eq!(r("table lite.notes"), path("lite", None, None, "notes"));
        // A misfit connection prefix reads as relative.
        assert_eq!(
            r("table pg.app.users"),
            path("pg", Some("pg"), Some("app"), "users")
        );

        // Not connected (or in SQLite): no relative forms.
        let off = Scope {
            connections: &CONNS,
            ..Scope::default()
        };
        let r = |q: &str| super::resolve(q, &off);
        assert_eq!(
            r("table public.users"),
            Command::NoConnection("public".into())
        );
        for bad in ["pg.app.users", "lite.main.notes"] {
            assert_eq!(
                r(&format!("table {bad}")),
                Command::BadTablePath(bad.into()),
                "{bad}"
            );
        }
        let in_lite = Scope {
            connections: &CONNS,
            conn: "lite",
            ..Scope::default()
        };
        assert_eq!(
            super::resolve("table main.notes", &in_lite),
            Command::NoConnection("main".into())
        );
        // A table literally named with a dot still opens directly.
        let sc = Scope {
            tables: &["a.b"],
            connections: &CONNS,
            ..Scope::default()
        };
        assert_eq!(super::resolve("table A.B", &sc), tbl(&["a.b"]));
    }

    #[test]
    fn qualified_path_suggestions() {
        let sc = in_pg();
        let s = |q: &str| super::suggest(q, &sc);
        assert_eq!(s("table us").as_deref(), Some("table users"), "tables 1st");
        assert_eq!(s("table my").as_deref(), Some("table mysql-local."));
        assert_eq!(s("table pg").as_deref(), Some("table pg."));
        assert_eq!(s("table pg.").as_deref(), Some("table pg.analytics."));
        assert_eq!(s("table pg.ap").as_deref(), Some("table pg.app."));
        assert_eq!(s("table pg.app").as_deref(), Some("table pg.app."));
        assert_eq!(
            s("table pg.app.pu").as_deref(),
            Some("table pg.app.public.")
        );
        assert_eq!(
            s("table pg.app.public.ord").as_deref(),
            Some("table pg.app.public.order_items")
        );
        assert_eq!(s("table pg.app.public.users"), None, "complete");
        assert_eq!(s("table pg.app.public.users.x"), None, "too deep");
        // Levels outside the current context are not loaded: no guesses.
        assert_eq!(s("table pg.analytics.pu"), None);
        assert_eq!(s("table pg.app.other.us"), None);
        assert_eq!(s("table lite.no"), None);
        assert_eq!(s("table nope.x"), None);
        // Relative levels: schemas, then databases, after tables and conns.
        assert_eq!(s("table pu").as_deref(), Some("table public."));
        assert_eq!(s("table an").as_deref(), Some("table analytics."));
        assert_eq!(s("table public.us").as_deref(), Some("table public.users"));
        assert_eq!(s("table app.pu").as_deref(), Some("table app.public."));
        assert_eq!(
            s("table app.public.ord").as_deref(),
            Some("table app.public.order_items")
        );
        assert_eq!(s("table analytics.pu"), None, "db not loaded");
        assert_eq!(s("table info.us"), None, "schema not current");
        let lite = Scope {
            tables: &["notes"],
            connections: &CONNS,
            conn: "lite",
            schema: "sqlite_schema",
            ..Scope::default()
        };
        assert_eq!(
            super::suggest("table lite.n", &lite).as_deref(),
            Some("table lite.notes")
        );
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
