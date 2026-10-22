//! Context-aware SQL autocompletion against a schema catalog.
//!
//! This is deliberately a heuristic, token-based analysis rather than a full
//! parser: it only needs to understand the statement around the cursor well
//! enough to pick between tables, columns, keywords and functions.

use crate::model::TableKind;
use std::ops::Range;

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub tables: Vec<CatalogTable>,
}

#[derive(Clone, Debug)]
pub struct CatalogTable {
    pub schema: String,
    pub name: String,
    pub kind: TableKind,
    /// Column name and formatted type.
    pub columns: Vec<(String, String)>,
    /// Single-column unique keys (primary key or unique index), for
    /// inferring relations in databases without foreign keys.
    pub keys: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    Column,
    Table,
    View,
    Schema,
    Function,
    Keyword,
    /// Generated SQL, e.g. `ST_Transform(geom, 4326)` for a geometry column.
    Snippet,
    /// Slash command such as `/con_4326` (expanded with Tab).
    Command,
}

#[derive(Clone, Debug)]
pub struct Suggestion {
    pub label: String,
    pub insert: String,
    pub kind: ItemKind,
    pub detail: String,
    /// Overrides [`Completion::replace`] (e.g. to also replace `alias.`).
    pub replace: Option<Range<usize>>,
}

#[derive(Clone, Debug)]
pub struct Completion {
    /// Byte range in the input that `insert` replaces.
    pub replace: Range<usize>,
    /// What the user has typed so far (unquoted).
    pub prefix: String,
    pub items: Vec<Suggestion>,
}

const MAX_ITEMS: usize = 80;

const KEYWORDS: &[&str] = &[
    "select", "from", "where", "and", "or", "not", "in", "is null", "is not null", "like",
    "ilike", "between", "exists", "as", "distinct", "join", "left join", "right join",
    "inner join", "full join", "cross join", "on", "using", "group by", "order by", "having",
    "limit", "offset", "asc", "desc", "nulls first", "nulls last", "union", "union all",
    "intersect", "except", "insert into", "values", "update", "set", "delete from", "returning",
    "create table", "create index", "create view", "alter table", "drop table", "truncate",
    "with", "case", "when", "then", "else", "end", "true", "false", "null", "default",
    "primary key", "references", "begin", "commit", "rollback", "explain", "explain analyze",
];

/// PostGIS functions with their usual arguments (shown as detail).
const POSTGIS_FUNCTIONS: &[(&str, &str)] = &[
    ("ST_Transform", "geom, srid"),
    ("ST_SetSRID", "geom, srid"),
    ("ST_SRID", "geom"),
    ("ST_AsText", "geom"),
    ("ST_AsEWKT", "geom"),
    ("ST_AsGeoJSON", "geom"),
    ("ST_GeomFromText", "wkt, srid"),
    ("ST_GeomFromGeoJSON", "json"),
    ("ST_MakePoint", "x, y"),
    ("ST_Point", "x, y, srid"),
    ("ST_MakeEnvelope", "xmin, ymin, xmax, ymax, srid"),
    ("ST_X", "point"),
    ("ST_Y", "point"),
    ("ST_Area", "geom"),
    ("ST_Length", "geom"),
    ("ST_Perimeter", "geom"),
    ("ST_Distance", "a, b"),
    ("ST_DWithin", "a, b, distance"),
    ("ST_Intersects", "a, b"),
    ("ST_Contains", "a, b"),
    ("ST_Within", "a, b"),
    ("ST_Touches", "a, b"),
    ("ST_Covers", "a, b"),
    ("ST_Buffer", "geom, radius"),
    ("ST_Centroid", "geom"),
    ("ST_PointOnSurface", "geom"),
    ("ST_Envelope", "geom"),
    ("ST_Extent", "geom"),
    ("ST_Union", "geom"),
    ("ST_Collect", "geom"),
    ("ST_Intersection", "a, b"),
    ("ST_Difference", "a, b"),
    ("ST_Simplify", "geom, tolerance"),
    ("ST_SimplifyPreserveTopology", "geom, tolerance"),
    ("ST_IsValid", "geom"),
    ("ST_MakeValid", "geom"),
    ("ST_GeometryType", "geom"),
    ("ST_NPoints", "geom"),
    ("ST_Dump", "geom"),
    ("ST_ClosestPoint", "a, b"),
    ("ST_ShortestLine", "a, b"),
    ("ST_LineInterpolatePoint", "line, fraction"),
    ("ST_Azimuth", "a, b"),
    ("ST_Force2D", "geom"),
    ("ST_Multi", "geom"),
];

const FUNCTIONS: &[&str] = &[
    "count", "sum", "avg", "min", "max", "coalesce", "nullif", "greatest", "least", "lower",
    "upper", "length", "trim", "substring", "replace", "concat", "concat_ws", "split_part",
    "position", "left", "right", "now", "current_date", "current_timestamp", "date_trunc",
    "date_part", "extract", "age", "to_char", "to_date", "to_timestamp", "round", "floor",
    "ceil", "abs", "random", "array_agg", "string_agg", "json_agg", "jsonb_agg",
    "jsonb_build_object", "json_build_object", "jsonb_array_elements", "jsonb_each",
    "to_jsonb", "row_number", "rank", "dense_rank", "lag", "lead", "generate_series",
    "unnest", "gen_random_uuid", "cast", "format", "regexp_replace",
];

/// Keywords that end a FROM list or can't be a table alias.
const CLAUSE_WORDS: &[&str] = &[
    "where", "group", "order", "having", "limit", "offset", "on", "using", "join", "left",
    "right", "inner", "full", "cross", "outer", "natural", "lateral", "union", "intersect",
    "except", "set", "values", "returning", "select", "from", "window", "for", "as", "and",
    "or", "not", "into", "update", "delete", "insert", "with", "default", "fetch",
];

/// Words that commonly cause trouble unquoted as identifiers.
const RESERVED: &[&str] = &[
    "all", "and", "any", "as", "asc", "both", "case", "check", "column", "constraint",
    "create", "default", "desc", "distinct", "do", "else", "end", "false", "for", "foreign",
    "from", "grant", "group", "having", "in", "into", "is", "join", "limit", "not", "null",
    "offset", "on", "or", "order", "primary", "references", "select", "table", "then", "to",
    "true", "union", "unique", "user", "using", "when", "where", "with",
];

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Tok {
    /// Identifier or keyword; unquoted words are lowercased.
    Word { text: String, quoted: bool },
    Punct(char),
}

#[derive(Clone, Debug)]
pub(crate) struct Token {
    pub(crate) tok: Tok,
    start: usize,
    end: usize,
}

impl Token {
    pub(crate) fn word(&self) -> Option<&str> {
        match &self.tok {
            Tok::Word { text, .. } => Some(text),
            Tok::Punct(_) => None,
        }
    }

    pub(crate) fn is_word(&self, w: &str) -> bool {
        matches!(&self.tok, Tok::Word { text, quoted: false } if text == w)
    }

    pub(crate) fn is_punct(&self, c: char) -> bool {
        self.tok == Tok::Punct(c)
    }
}

/// Tokenize, skipping strings, comments and numbers.
pub(crate) fn tokenize(s: &str) -> Vec<Token> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    let ident_start = |c: u8| c.is_ascii_alphabetic() || c == b'_' || c >= 0x80;
    let ident_char = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80;

    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'-' && b.get(i + 1) == Some(&b'-') {
            match s[i..].find('\n') {
                Some(n) => i += n + 1,
                None => return out,
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            match s[i + 2..].find("*/") {
                Some(n) => i += n + 4,
                None => return out,
            }
        } else if c == b'\'' {
            match s[i + 1..].find('\'') {
                Some(n) => i += n + 2,
                None => return out,
            }
        } else if c == b'"' {
            match s[i + 1..].find('"') {
                Some(n) => {
                    out.push(Token {
                        tok: Tok::Word { text: s[i + 1..i + 1 + n].to_string(), quoted: true },
                        start: i,
                        end: i + n + 2,
                    });
                    i += n + 2;
                }
                None => {
                    // Unterminated quoted identifier: still completable.
                    out.push(Token {
                        tok: Tok::Word { text: s[i + 1..].to_string(), quoted: true },
                        start: i,
                        end: b.len(),
                    });
                    i = b.len();
                }
            }
        } else if c == b'$' && dollar_tag_len(&b[i..]).is_some() {
            let n = dollar_tag_len(&b[i..]).unwrap();
            let tag = &s[i..i + n];
            match s[i + n..].find(tag) {
                Some(m) => i += n + m + n,
                None => return out,
            }
        } else if c.is_ascii_digit() {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
        } else if ident_start(c) {
            let start = i;
            while i < b.len() && ident_char(b[i]) {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Word { text: s[start..i].to_lowercase(), quoted: false },
                start,
                end: i,
            });
        } else {
            let ch = s[i..].chars().next().unwrap();
            out.push(Token { tok: Tok::Punct(ch), start: i, end: i + ch.len_utf8() });
            i += ch.len_utf8();
        }
    }
    out
}

/// Unquoted words (lowercased) outside strings and comments.
pub(crate) fn words(sql: &str) -> Vec<String> {
    tokenize(sql)
        .into_iter()
        .filter_map(|t| match t.tok {
            Tok::Word { text, quoted: false } => Some(text),
            _ => None,
        })
        .collect()
}

fn dollar_tag_len(b: &[u8]) -> Option<usize> {
    let mut j = 1;
    if b.get(j).is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
    }
    (b.get(j) == Some(&b'$')).then_some(j + 1)
}

#[derive(Debug)]
struct TableRef {
    schema: Option<String>,
    table: String,
    alias: Option<String>,
}

/// Find `FROM a [AS] x, b`, `JOIN c d`, `UPDATE e`, `INTO f` references.
fn table_refs(tokens: &[Token]) -> Vec<TableRef> {
    let mut refs = Vec::new();
    let mut in_from = false;
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        let starts_ref = ["from", "join", "update", "into"].iter().any(|w| t.is_word(w))
            || (in_from && t.is_punct(','));
        if t.is_word("from") {
            in_from = true;
        } else if CLAUSE_WORDS.iter().any(|w| t.is_word(w)) && !t.is_word("as") {
            in_from = false;
        }
        if !starts_ref {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        let Some(first) = tokens.get(j).and_then(|t| t.word()).filter(|_| !is_clause(&tokens[j])) else {
            i += 1;
            continue;
        };
        let mut r = TableRef { schema: None, table: first.to_string(), alias: None };
        j += 1;
        if tokens.get(j).is_some_and(|t| t.is_punct('.')) {
            if let Some(second) = tokens.get(j + 1).and_then(|t| t.word()) {
                r.schema = Some(r.table.clone());
                r.table = second.to_string();
                j += 2;
            }
        }
        if tokens.get(j).is_some_and(|t| t.is_word("as")) {
            j += 1;
        }
        if let Some(alias) = tokens.get(j).filter(|t| !is_clause(t)).and_then(|t| t.word()) {
            r.alias = Some(alias.to_string());
            j += 1;
        }
        refs.push(r);
        i = j;
        if t.is_word("from") || t.is_punct(',') {
            in_from = true;
        }
    }
    refs
}

fn is_clause(t: &Token) -> bool {
    CLAUSE_WORDS.iter().any(|w| t.is_word(w))
}

/// Postgres catalogs and PostGIS's own helper schemas.
fn is_system_schema(s: &str) -> bool {
    matches!(s, "pg_catalog" | "information_schema" | "tiger" | "tiger_data" | "topology")
}

impl Catalog {
    /// `default`: the console's schema, tried before `public`.
    fn resolve(&self, r: &TableRef, default: &str) -> Option<&CatalogTable> {
        let candidates = self.tables.iter().filter(|t| t.name == r.table);
        match &r.schema {
            Some(s) => candidates.into_iter().find(|t| &t.schema == s),
            None => {
                let all: Vec<_> = candidates.collect();
                all.iter()
                    .find(|t| t.schema == default)
                    .or_else(|| all.iter().find(|t| t.schema == "public"))
                    .or_else(|| all.iter().find(|t| t.schema == "pg_catalog"))
                    .or_else(|| all.first())
                    .copied()
            }
        }
    }

    /// Spatial SQL is available: PostGIS (its catalog table or geometry
    /// columns) or ArcGIS `st_geometry` columns.
    pub fn has_postgis(&self) -> bool {
        self.tables.iter().any(|t| {
            t.name == "spatial_ref_sys"
                || t.columns.iter().any(|(_, ty)| crate::geo::column_srid(ty).is_some() || is_arcgis_geometry(ty))
        })
    }

    fn schemas(&self) -> Vec<&str> {
        let mut s: Vec<&str> = self.tables.iter().map(|t| t.schema.as_str()).collect();
        s.dedup();
        s.sort();
        s.dedup();
        s
    }
}

/// ArcGIS (sde) geometry type, e.g. `sde.st_geometry`.
pub fn is_arcgis_geometry(ty: &str) -> bool {
    ty.trim().to_ascii_lowercase().trim_matches('"').ends_with("st_geometry")
}

/// `/word` being typed at the cursor: (range from the slash, word after it).
fn slash_command_at(sql: &str, cursor: usize) -> Option<(Range<usize>, &str)> {
    let head = &sql[..cursor];
    let word_start = head.len() - head.chars().rev().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').map(char::len_utf8).sum::<usize>();
    let slash = word_start.checked_sub(1).filter(|&i| head.as_bytes()[i] == b'/')?;
    // Only at the start of a line or after whitespace (not `a/b` division).
    let before = head[..slash].chars().next_back();
    if before.is_some_and(|c| !c.is_whitespace()) {
        return None;
    }
    let word_end = cursor + sql[cursor..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').map(char::len_utf8).sum::<usize>();
    Some((slash..word_end, &sql[word_start..cursor]))
}

pub fn needs_quotes(ident: &str) -> bool {
    let mut chars = ident.chars();
    let valid_start = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c == '_');
    !valid_start
        || !ident.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '$')
        || RESERVED.contains(&ident)
}

fn ident(name: &str) -> String {
    if needs_quotes(name) { crate::sql::quote_ident(name) } else { name.to_string() }
}

#[derive(Clone, Copy, PartialEq)]
enum Context {
    /// After FROM / JOIN / INTO / UPDATE: a table is expected.
    Table,
    /// Right after a table reference: clause keywords make sense.
    AfterTable,
    /// Expressions: columns, functions, keywords.
    Expr,
}

/// Compute suggestions for the cursor at byte offset `cursor`.
/// `forced` (⌃Space) allows an empty prefix outside of `x.` positions.
/// `schema` is the console's schema, where unqualified names resolve first.
pub fn complete(sql: &str, cursor: usize, catalog: &Catalog, schema: &str, forced: bool) -> Option<Completion> {
    let cursor = cursor.min(sql.len());
    if !sql.is_char_boundary(cursor) || ends_in_literal(&sql[..cursor]) {
        return None;
    }
    if let Some((replace, prefix)) = slash_command_at(sql, cursor) {
        let prefix_lc = prefix.to_ascii_lowercase();
        let mut items: Vec<Suggestion> = crate::commands::active()
            .iter()
            .filter(|c| c.name.starts_with(&prefix_lc))
            .map(|c| Suggestion {
                label: format!("/{} {}", c.name, c.args()).trim_end().to_string(),
                insert: format!("/{} ", c.name),
                kind: ItemKind::Command,
                detail: c.help.clone(),
                replace: None,
            })
            .collect();
        // Last, and only once typed toward: it isn't a query.
        if !prefix_lc.is_empty() && crate::commands::NEW.starts_with(&prefix_lc) {
            items.push(Suggestion {
                label: format!("/{} [name]", crate::commands::NEW),
                insert: format!("/{} ", crate::commands::NEW),
                kind: ItemKind::Command,
                detail: "Create a slash command".into(),
                replace: None,
            });
        }
        return (!items.is_empty()).then(|| Completion { replace, prefix: format!("/{prefix}"), items });
    }
    // Arguments of a slash command aren't SQL: no suggestions, so Tab expands.
    if crate::commands::command_line_at(sql, cursor).is_some() {
        return None;
    }
    let tokens = tokenize(sql);

    // Current statement bounds (top-level `;`, strings already skipped).
    let stmt_start = tokens
        .iter()
        .filter(|t| t.is_punct(';') && t.end <= cursor)
        .map(|t| t.end)
        .last()
        .unwrap_or(0);
    let stmt_end = tokens
        .iter()
        .find(|t| t.is_punct(';') && t.start >= cursor)
        .map_or(sql.len(), |t| t.start);
    let stmt: Vec<Token> = tokens
        .into_iter()
        .filter(|t| t.start >= stmt_start && t.start < stmt_end)
        .collect();

    // The word under/before the cursor is the prefix being completed.
    let current = stmt.iter().position(|t| t.start < cursor && cursor <= t.end && t.word().is_some());
    let (prefix, replace, before) = match current {
        Some(i) => {
            let t = &stmt[i];
            let quoted = matches!(t.tok, Tok::Word { quoted: true, .. });
            let raw = &sql[t.start..cursor];
            let prefix = if quoted { raw.trim_start_matches('"').to_string() } else { raw.to_string() };
            (prefix, t.start..t.end, &stmt[..i])
        }
        None => {
            let n = stmt.iter().take_while(|t| t.end <= cursor).count();
            (String::new(), cursor..cursor, &stmt[..n])
        }
    };

    // `qualifier.` directly before the prefix?
    let qualifier = match before {
        [.., q, dot] if dot.is_punct('.') && dot.end == replace.start => q.word().map(str::to_string),
        _ => None,
    };
    // Where `alias.` starts, so snippets can replace it too.
    let qualifier_start = match before {
        [.., q, dot] if dot.is_punct('.') && dot.end == replace.start => Some(q.start),
        _ => None,
    };
    let postgis = catalog.has_postgis();
    let in_select = last_clause(before) == Some("select");
    if prefix.is_empty() && qualifier.is_none() && !forced {
        return None;
    }

    let refs = table_refs(&stmt);
    let resolved: Vec<(&TableRef, &CatalogTable)> =
        refs.iter().filter_map(|r| catalog.resolve(r, schema).map(|t| (r, t))).collect();

    let mut items = Vec::new();
    if let Some(q) = qualifier {
        let target = resolved
            .iter()
            .find(|(r, _)| r.alias.as_deref() == Some(q.as_str()))
            .or_else(|| resolved.iter().find(|(r, _)| r.table == q))
            .map(|(_, t)| *t)
            .or_else(|| catalog.resolve(&TableRef { schema: None, table: q.clone(), alias: None }, schema));
        if let Some(t) = target {
            push_columns(&mut items, t, false);
            if postgis {
                let qualified = qualifier_start.map(|start| (format!("{}.", sql_ident_text(&sql[start..replace.start - 1])), start..replace.end));
                push_geometry_snippets(&mut items, t, qualified, in_select);
            }
        }
        for t in catalog.tables.iter().filter(|t| t.schema == q) {
            push_table(&mut items, t, false, schema);
        }
    } else {
        match context(before) {
            Context::Table => {
                for t in &catalog.tables {
                    push_table(&mut items, t, true, schema);
                }
                for s in catalog.schemas() {
                    items.push(Suggestion {
                        label: s.to_string(),
                        insert: ident(s),
                        kind: ItemKind::Schema,
                        detail: "schema".into(), replace: None
                    });
                }
            }
            Context::AfterTable => push_keywords(&mut items, &prefix),
            Context::Expr => {
                for (_, t) in &resolved {
                    push_columns(&mut items, t, resolved.len() > 1);
                }
                if postgis {
                    for (r, t) in &resolved {
                        // With several tables, qualify generated SQL by alias/name.
                        let q = (resolved.len() > 1).then(|| format!("{}.", r.alias.as_deref().unwrap_or(&r.table)));
                        push_geometry_snippets(&mut items, t, q.map(|q| (q, replace.clone())), in_select);
                    }
                    push_postgis_functions(&mut items);
                }
                push_functions(&mut items, &prefix);
                push_keywords(&mut items, &prefix);
                for t in &catalog.tables {
                    push_table(&mut items, t, true, schema);
                }
            }
        }
    }

    let items = rank(items, &prefix);
    // Nothing to offer if the only match is exactly what was typed.
    if items.is_empty() || (items.len() == 1 && items[0].label.eq_ignore_ascii_case(&prefix)) {
        return None;
    }
    Some(Completion { replace, prefix, items })
}

/// Whether the text ends inside a string literal or comment.
fn ends_in_literal(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'-' if b.get(i + 1) == Some(&b'-') => match s[i..].find('\n') {
                Some(n) => i += n + 1,
                None => return true,
            },
            b'/' if b.get(i + 1) == Some(&b'*') => match s[i + 2..].find("*/") {
                Some(n) => i += n + 4,
                None => return true,
            },
            b'\'' => match s[i + 1..].find('\'') {
                Some(n) => i += n + 2,
                None => return true,
            },
            b'$' => match dollar_tag_len(&b[i..]) {
                Some(n) => match s[i + n..].find(&s[i..i + n]) {
                    Some(m) => i += n + m + n,
                    None => return true,
                },
                None => i += 1,
            },
            _ => i += 1,
        }
    }
    false
}

fn context(before: &[Token]) -> Context {
    let last_clause = before.iter().rposition(|t| {
        ["from", "join", "into", "update", "table", "truncate", "select", "where", "on", "by",
         "set", "having", "and", "or", "values", "returning", "when", "then", "else"]
            .iter()
            .any(|w| t.is_word(w))
    });
    let Some(k) = last_clause else { return Context::Expr };
    let kw = before[k].word().unwrap_or("");
    if !["from", "join", "into", "update", "table", "truncate"].contains(&kw) {
        return Context::Expr;
    }
    // FROM a, b| → table; FROM a | → clause keywords.
    match before.last() {
        Some(t) if t.is_punct(',') => Context::Table,
        Some(_) if before.len() - 1 == k => Context::Table,
        _ => Context::AfterTable,
    }
}

fn push_columns(items: &mut Vec<Suggestion>, t: &CatalogTable, show_table: bool) {
    for (name, ty) in &t.columns {
        items.push(Suggestion {
            label: name.clone(),
            insert: ident(name),
            kind: ItemKind::Column,
            detail: match crate::sql::short_type(ty) {
                ty if show_table => format!("{} · {ty}", t.name),
                ty => ty,
            }, replace: None
        });
    }
}

fn push_table(items: &mut Vec<Suggestion>, t: &CatalogTable, qualify: bool, default: &str) {
    let needs_schema = qualify && !matches!(t.schema.as_str(), "public" | "pg_catalog") && t.schema != default;
    let insert = if needs_schema { format!("{}.{}", ident(&t.schema), ident(&t.name)) } else { ident(&t.name) };
    let kind = match t.kind {
        TableKind::View | TableKind::MaterializedView => ItemKind::View,
        _ => ItemKind::Table,
    };
    items.push(Suggestion { label: t.name.clone(), insert, kind, detail: t.schema.clone(), replace: None });
}

fn push_postgis_functions(items: &mut Vec<Suggestion>) {
    for (name, args) in POSTGIS_FUNCTIONS {
        items.push(Suggestion {
            label: name.to_string(),
            insert: format!("{name}("),
            kind: ItemKind::Function,
            detail: format!("({args})"),
            replace: None,
        });
    }
}

/// The SQL text of an identifier as typed (keeps quotes).
fn sql_ident_text(s: &str) -> String {
    s.trim().to_string()
}

/// Generated expressions for each geometry column: reprojection to WGS84
/// (when stored in another SRID), GeoJSON, WKT and Web Mercator.
/// `qualified` is the `alias.` prefix and the range it should replace.
fn push_geometry_snippets(
    items: &mut Vec<Suggestion>,
    t: &CatalogTable,
    qualified: Option<(String, Range<usize>)>,
    in_select: bool,
) {
    let (q, range) = match qualified {
        Some((q, r)) => (q, Some(r)),
        None => (String::new(), None),
    };
    for (col, ty) in &t.columns {
        let c = format!("{q}{}", ident(col));
        let alias = if in_select { format!(" AS {}", ident(col)) } else { String::new() };
        if is_arcgis_geometry(ty) {
            // ArcGIS st_geometry: convert through WKT text into PostGIS.
            for (label, insert, detail) in [
                (format!("ST_SetSRID(st_astext({col})::text, 4326)"), format!("ST_SetSRID(st_astext({c})::text, 4326){alias}"), "ArcGIS → 4326 (text)"),
                (
                    format!("ST_Transform(ST_SetSRID(st_astext({col})::text::geometry, 3857), 4326)"),
                    format!("ST_Transform(ST_SetSRID(st_astext({c})::text::geometry, 3857), 4326){alias}"),
                    "ArcGIS Web Mercator → 4326",
                ),
                (format!("st_astext({col})"), format!("st_astext({c})"), "WKT"),
            ] {
                items.push(Suggestion { label, insert, kind: ItemKind::Snippet, detail: detail.into(), replace: range.clone() });
            }
            continue;
        }
        let Some(srid) = crate::geo::column_srid(ty) else { continue };
        let mut add = |label: String, insert: String, detail: String| {
            items.push(Suggestion { label, insert, kind: ItemKind::Snippet, detail, replace: range.clone() });
        };
        let wgs84 = if srid != 4326 && srid != 0 { format!("ST_Transform({c}, 4326)") } else { c.clone() };
        if srid != 4326 && srid != 0 {
            add(format!("ST_Transform({col}, 4326)"), format!("{wgs84}{alias}"), format!("SRID {srid} → 4326"));
        }
        add(format!("ST_AsGeoJSON({col})"), format!("ST_AsGeoJSON({wgs84})"), "GeoJSON (WGS84)".into());
        add(format!("ST_AsText({col})"), format!("ST_AsText({c})"), "WKT".into());
        if srid != 3857 && srid != 0 {
            add(format!("ST_Transform({col}, 3857)"), format!("ST_Transform({c}, 3857){alias}"), format!("SRID {srid} → 3857"));
        }
    }
}

fn last_clause(before: &[Token]) -> Option<&str> {
    before.iter().rev().find_map(|t| {
        t.word().filter(|w| ["select", "from", "where", "on", "by", "set", "having", "join", "values"].contains(w))
    })
}

fn push_functions(items: &mut Vec<Suggestion>, prefix: &str) {
    for f in FUNCTIONS {
        let name = match_case(f, prefix);
        let insert = if matches!(*f, "current_date" | "current_timestamp") { name.clone() } else { format!("{name}(") };
        items.push(Suggestion { label: f.to_string(), insert, kind: ItemKind::Function, detail: "function".into(), replace: None });
    }
}

fn push_keywords(items: &mut Vec<Suggestion>, prefix: &str) {
    for k in KEYWORDS {
        items.push(Suggestion {
            label: k.to_string(),
            insert: format!("{} ", match_case(k, prefix)),
            kind: ItemKind::Keyword,
            detail: String::new(), replace: None
        });
    }
}

/// Uppercase keywords when the user is typing in uppercase.
fn match_case(word: &str, prefix: &str) -> String {
    let typed_upper = prefix.chars().any(|c| c.is_ascii_alphabetic())
        && prefix.chars().all(|c| !c.is_ascii_lowercase());
    if typed_upper { word.to_uppercase() } else { word.to_string() }
}

/// Filter by prefix/substring/subsequence and order by match quality,
/// then kind (in the order items were added), then length.
fn rank(items: Vec<Suggestion>, prefix: &str) -> Vec<Suggestion> {
    let p = prefix.to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let mut scored: Vec<(u8, bool, usize, Suggestion)> = items
        .into_iter()
        .enumerate()
        .filter_map(|(order, s)| {
            let l = s.label.to_lowercase();
            // Snippets match on the column they're generated for: the
            // innermost argument of e.g. `ST_AsGeoJSON(geom)`.
            let key = if s.kind == ItemKind::Snippet {
                l.rsplit('(').next().and_then(|r| r.split([',', ')']).next()).unwrap_or(&l).trim().to_string()
            } else {
                l.clone()
            };
            let quality = if p.is_empty() || key.starts_with(&p) || l.starts_with(&p) {
                0
            } else if l.split(['_', ' ', '.']).any(|part| part.starts_with(&p)) {
                1
            } else if l.contains(&p) {
                2
            } else if is_subsequence(&p, &l) {
                3
            } else {
                return None;
            };
            let system = is_system_schema(&s.detail) && !p.starts_with("pg_");
            // Keep the list focused: system objects and loose fuzzy hits only
            // when the user is clearly asking for them.
            if (system && quality > 0) || (quality == 3 && p.chars().count() < 3) {
                return None;
            }
            Some((quality, system, order, s))
        })
        .collect();
    scored.sort_by(|a, b| (a.0, a.1, kind_rank(a.3.kind), a.2).cmp(&(b.0, b.1, kind_rank(b.3.kind), b.2)));
    scored
        .into_iter()
        .map(|(.., s)| s)
        .filter(|s| seen.insert((s.label.clone(), s.kind as u8)))
        .take(MAX_ITEMS)
        .collect()
}

fn kind_rank(k: ItemKind) -> u8 {
    match k {
        ItemKind::Column => 0,
        ItemKind::Snippet | ItemKind::Command => 1,
        ItemKind::Table | ItemKind::View => 1,
        ItemKind::Schema => 2,
        ItemKind::Keyword => 3,
        ItemKind::Function => 4,
    }
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    needle.chars().all(|c| it.any(|h| h == c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let t = |schema: &str, name: &str, kind, cols: &[&str]| CatalogTable {
            schema: schema.into(),
            name: name.into(),
            kind,
            columns: cols.iter().map(|c| (c.to_string(), "text".to_string())).collect(),
            keys: Vec::new(),
        };
        Catalog {
            tables: vec![
                t("public", "users", TableKind::Table, &["id", "email", "created_at"]),
                t("public", "orders", TableKind::Table, &["id", "user_id", "total"]),
                t("billing", "invoices", TableKind::Table, &["id", "amount"]),
                t("public", "active_users", TableKind::View, &["id"]),
                t("pg_catalog", "pg_stat_activity", TableKind::View, &["pid", "state"]),
                t("public", "Weird Name", TableKind::Table, &["Mixed"]),
            ],
        }
    }

    #[test]
    fn console_schema_is_preferred() {
        let mut c = catalog();
        c.tables.push(CatalogTable { schema: "billing".into(), name: "users".into(), kind: TableKind::Table, columns: vec![("account".into(), "text".into())], keys: Vec::new() });
        let cols = |schema: &str| -> Vec<String> {
            let sql = "select u. from users u";
            complete(sql, 9, &c, schema, false).unwrap().items.into_iter().map(|s| s.label).collect()
        };
        assert!(cols("public").contains(&"email".to_string()));
        assert!(cols("billing").contains(&"account".to_string()));
        assert!(!cols("billing").contains(&"email".to_string()));
        // Tables of the console schema are offered unqualified.
        let items = complete("select * from inv", 17, &c, "billing", false).unwrap().items;
        assert_eq!(items[0].insert, "invoices");
        let items = complete("select * from inv", 17, &c, "public", false).unwrap().items;
        assert_eq!(items[0].insert, "billing.invoices");
    }

    /// `|` marks the cursor.
    fn labels(input: &str) -> Vec<String> {
        let cursor = input.find('|').unwrap();
        let sql = input.replace('|', "");
        complete(&sql, cursor, &catalog(), "public", false)
            .map(|c| c.items.into_iter().map(|s| s.label).collect())
            .unwrap_or_default()
    }

    fn first_insert(input: &str) -> String {
        let cursor = input.find('|').unwrap();
        let sql = input.replace('|', "");
        complete(&sql, cursor, &catalog(), "public", false).unwrap().items[0].insert.clone()
    }

    #[test]
    fn tables_after_from() {
        let l = labels("select * from us|");
        assert_eq!(l[0], "users");
        assert!(l.contains(&"active_users".to_string()));
        assert_eq!(first_insert("select * from inv|"), "billing.invoices");
        assert_eq!(first_insert("select * from Weird|"), "\"Weird Name\"");
    }

    #[test]
    fn columns_from_referenced_tables() {
        let l = labels("select em| from users");
        assert_eq!(l[0], "email");
        let l = labels("select * from users u join orders o on o.user_id = u.id where to|");
        assert_eq!(l[0], "total");
    }

    #[test]
    fn alias_dot_completion() {
        let l = labels("select o.| from orders o");
        assert_eq!(l, ["id", "user_id", "total"]);
        let l = labels("select u.cr| from public.users as u");
        assert_eq!(l, ["created_at"]);
        let l = labels("select users.| from users");
        assert_eq!(l, ["id", "email", "created_at"]);
    }

    #[test]
    fn schema_dot_lists_tables() {
        assert_eq!(labels("select * from billing.|"), ["invoices"]);
    }

    #[test]
    fn keywords_after_table() {
        let l = labels("select * from users wh|");
        assert_eq!(l[0], "where");
        assert_eq!(first_insert("SELECT * FROM users WH|"), "WHERE ");
        assert_eq!(first_insert("sel|"), "select ");
    }

    #[test]
    fn statement_scoped() {
        // Columns come only from the statement under the cursor.
        let l = labels("select * from orders; select tot| from users");
        assert!(!l.contains(&"total".to_string()));
    }

    #[test]
    fn system_tables_rank_low_unless_asked() {
        let l = labels("select * from pg_st|");
        assert_eq!(l[0], "pg_stat_activity");
        let l = labels("select p| from pg_stat_activity");
        assert_eq!(l[0], "pid");
    }

    #[test]
    fn nothing_inside_strings_or_comments() {
        assert!(labels("select 'us|").is_empty());
        assert!(labels("-- us|").is_empty());
        assert!(labels("select * from users -- wh|").is_empty());
    }

    #[test]
    fn empty_prefix_needs_dot_or_force() {
        assert!(labels("select * from |").is_empty());
        let sql = "select * from ";
        let c = complete(sql, sql.len(), &catalog(), "public", true).unwrap();
        assert_eq!(c.items[0].label, "users");
    }

    #[test]
    fn replace_range_covers_whole_word() {
        let sql = "select * from usXXers";
        let c = complete(sql, 16, &catalog(), "public", false).unwrap();
        assert_eq!(c.replace, 14..21);
        assert_eq!(c.prefix, "us");
    }

    #[test]
    fn no_system_noise_for_short_prefixes() {
        let l = labels("select cr| from users");
        assert_eq!(l[0], "created_at");
        assert!(!l.iter().any(|x| x.starts_with("pg_")), "{l:?}");
    }

    fn spatial_catalog() -> Catalog {
        let mut c = catalog();
        c.tables.push(CatalogTable {
            schema: "public".into(),
            name: "parcels".into(),
            kind: TableKind::Table,
            columns: vec![("id".into(), "integer".into()), ("geom".into(), "geometry(Polygon,32648)".into())],
            keys: Vec::new(),
        });
        c.tables.push(CatalogTable {
            schema: "public".into(),
            name: "spatial_ref_sys".into(),
            kind: TableKind::Table,
            columns: vec![("srid".into(), "integer".into())],
            keys: Vec::new(),
        });
        c
    }

    fn complete_with(input: &str, catalog: &Catalog) -> Vec<Suggestion> {
        let cursor = input.find('|').unwrap();
        let sql = input.replace('|', "");
        complete(&sql, cursor, catalog, "public", false).map(|c| c.items).unwrap_or_default()
    }

    #[test]
    fn postgis_functions_only_with_postgis() {
        let with = complete_with("select st_tr| from parcels", &spatial_catalog());
        assert!(with.iter().any(|s| s.label == "ST_Transform" && s.insert == "ST_Transform("));
        let without = complete_with("select st_tr| from users", &catalog());
        assert!(!without.iter().any(|s| s.label == "ST_Transform"));
    }

    #[test]
    fn geometry_snippets_reproject_by_srid() {
        let items = complete_with("select id, ge| from parcels", &spatial_catalog());
        // The column first, then its generated expressions.
        assert_eq!(items[0].label, "geom");
        assert_eq!(items[1].label, "ST_Transform(geom, 4326)");
        let t = &items[1];
        assert_eq!(t.insert, "ST_Transform(geom, 4326) AS geom");
        assert_eq!(t.detail, "SRID 32648 → 4326");
        let gj = items.iter().find(|s| s.label == "ST_AsGeoJSON(geom)").unwrap();
        assert_eq!(gj.insert, "ST_AsGeoJSON(ST_Transform(geom, 4326))");
        // Outside the select list there's no alias.
        let w = complete_with("select * from parcels where st_| ", &spatial_catalog());
        assert!(w.iter().any(|s| s.insert == "ST_Transform(geom, 4326)"));
    }

    #[test]
    fn geometry_snippets_replace_the_alias() {
        let input = "select p.| from parcels p";
        let cursor = input.find('|').unwrap();
        let sql = input.replace('|', "");
        let c = complete(&sql, cursor, &spatial_catalog(), "public", false).unwrap();
        let t = c.items.iter().find(|s| s.label == "ST_Transform(geom, 4326)").unwrap();
        assert_eq!(t.insert, "ST_Transform(p.geom, 4326) AS geom");
        assert_eq!(t.replace, Some(7..9)); // replaces "p."
        // Plain columns still complete after the dot.
        assert!(c.items.iter().any(|s| s.label == "geom" && s.replace.is_none()));
    }

    #[test]
    fn slash_commands() {
        let input = "select 1;\n/con|";
        let cursor = input.find('|').unwrap();
        let sql = input.replace('|', "");
        let c = complete(&sql, cursor, &catalog(), "public", false).unwrap();
        assert_eq!(c.replace, 10..14);
        assert_eq!(c.items.iter().map(|s| s.insert.as_str()).collect::<Vec<_>>(), ["/con_4326 ", "/con_4326_2 ", "/con_merc "]);
        // Just the slash lists everything; division doesn't.
        assert_eq!(complete("/", 1, &catalog(), "public", false).unwrap().items.len(), crate::commands::defaults().len());
        assert!(complete("select a/b", 9, &catalog(), "public", false).is_none_or(|c| c.items.iter().all(|s| s.kind != ItemKind::Command)));
        let new = complete("/ne", 3, &catalog(), "public", false).unwrap();
        assert_eq!(new.items.last().unwrap().insert, "/new ");
        // No SQL suggestions while typing a command's arguments.
        assert!(complete("/con_4326 us", 12, &catalog(), "public", false).is_none());
    }

    #[test]
    fn arcgis_snippets() {
        let mut cat = catalog();
        cat.tables.push(CatalogTable {
            schema: "public".into(),
            name: "lands".into(),
            kind: TableKind::Table,
            columns: vec![("objectid".into(), "integer".into()), ("shape".into(), "sde.st_geometry".into())],
            keys: Vec::new(),
        });
        let items = complete_with("select objectid, sh| from lands", &cat);
        assert_eq!(items[0].label, "shape");
        assert_eq!(items[1].insert, "ST_SetSRID(st_astext(shape)::text, 4326) AS shape");
        assert!(items.iter().any(|s| s.detail == "ArcGIS Web Mercator → 4326"));
    }

    #[test]
    fn fuzzy_matching() {
        assert_eq!(labels("select * from acu|")[0], "active_users");
    }
}
