//! Lightweight SQL text helpers: statement splitting and identifier quoting.

use std::ops::Range;

/// A statement in a script. `raw` spans up to and including its `;`,
/// `trimmed` excludes surrounding whitespace and the `;`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Statement {
    pub raw: Range<usize>,
    pub trimmed: Range<usize>,
}

/// Split a script on top-level `;`, respecting quotes, comments and
/// dollar-quoted bodies. Returned ranges are byte offsets.
pub fn split_statements(sql: &str) -> Vec<Statement> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < b.len() {
        match b[i] {
            b'\'' | b'"' => i = skip_until(b, i + 1, &[b[i]]),
            b'-' if b.get(i + 1) == Some(&b'-') => i = skip_until(b, i + 2, b"\n"),
            b'/' if b.get(i + 1) == Some(&b'*') => i = skip_until(b, i + 2, b"*/"),
            b'$' => match dollar_tag(b, i) {
                Some(tag_end) => {
                    let tag = &b[i..tag_end];
                    i = skip_until(b, tag_end, tag);
                }
                None => i += 1,
            },
            b';' => {
                push_statement(sql, start, i + 1, &mut out);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    push_statement(sql, start, b.len(), &mut out);
    out
}

/// The statement under a byte cursor. When the cursor sits in whitespace
/// after a statement, the preceding statement is returned.
pub fn statement_at(sql: &str, cursor: usize) -> Option<Range<usize>> {
    let stmts = split_statements(sql);
    let mut best = None;
    for s in &stmts {
        if s.trimmed.start > cursor {
            break;
        }
        best = Some(s.trimmed.clone());
        if cursor < s.raw.end {
            break;
        }
    }
    best.or_else(|| stmts.first().map(|s| s.trimmed.clone()))
}

pub fn quote_ident(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

const PAGE_PREFIX: &str = "select * from (";

/// Wrap a single read-only query so one page can be fetched server-side.
/// Returns the SQL and how many chars were prepended (to map error
/// positions back). `None` for anything that isn't safely wrappable.
pub fn paged_query(stmt: &str, limit: usize, offset: usize) -> Option<(String, usize)> {
    // Cut at the end of the statement so a trailing `;` and any comments
    // after it stay out of the subquery. Leading text is kept as-is so
    // error positions only shift by the prefix.
    let [s] = &split_statements(stmt)[..] else { return None };
    let body = &stmt[..s.trimmed.end];
    let words = crate::complete::words(body);
    let first = words.first()?.as_str();
    if !matches!(first, "select" | "with" | "values" | "table") {
        return None;
    }
    // Data-modifying CTEs, SELECT INTO and locking clauses can't be wrapped.
    const BLOCK: [&str; 7] = ["insert", "update", "delete", "merge", "into", "for", "fetch"];
    if words.iter().any(|w| BLOCK.contains(&w.as_str())) {
        return None;
    }
    // Newline before `)` so a trailing line comment can't swallow it.
    let sql = format!("{PAGE_PREFIX}{body}\n) as _rgrip_page limit {limit} offset {offset}");
    Some((sql, PAGE_PREFIX.len()))
}

/// `stmt` wrapped to sort by `column` (for sorting a query result on the
/// server, so every page is in order). `None` if it can't be wrapped.
pub fn sorted_query(stmt: &str, column: &str, ascending: bool) -> Option<String> {
    paged_query(stmt, 1, 0)?;
    let [s] = &split_statements(stmt)[..] else { return None };
    let body = &stmt[..s.trimmed.end];
    let dir = if ascending { "asc nulls last" } else { "desc nulls last" };
    Some(format!("select * from (\n{body}\n) as _rgrip_sort order by {} {dir}", quote_ident(column)))
}

/// Compact display form of verbose Postgres type names.
pub fn short_type(ty: &str) -> String {
    const NAMES: [(&str, &str); 5] = [
        ("timestamp without time zone", "timestamp"),
        ("timestamp with time zone", "timestamptz"),
        ("time without time zone", "time"),
        ("time with time zone", "timetz"),
        ("character varying", "varchar"),
    ];
    let mut out = ty.to_string();
    for (long, short) in NAMES {
        out = out.replace(long, short);
    }
    out
}

/// Convert a char index (as used by egui cursors) into a byte index.
pub fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(i, _)| i)
}

/// 1-based line and column for a 1-based character position.
pub fn line_col(s: &str, char_pos: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for c in s.chars().take(char_pos.saturating_sub(1)) {
        if c == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn push_statement(sql: &str, start: usize, end: usize, out: &mut Vec<Statement>) {
    let body = &sql[start..end];
    let body = body.strip_suffix(';').unwrap_or(body);
    let lead = body.len() - body.trim_start().len();
    let trimmed = body.trim();
    if trimmed.is_empty() || is_only_comments(trimmed) {
        return;
    }
    let t0 = start + lead;
    out.push(Statement {
        raw: start..end,
        trimmed: t0..t0 + trimmed.len(),
    });
}

fn is_only_comments(mut s: &str) -> bool {
    loop {
        s = s.trim_start();
        if let Some(rest) = s.strip_prefix("--") {
            s = rest.split_once('\n').map_or("", |(_, r)| r);
        } else if let Some(rest) = s.strip_prefix("/*") {
            s = rest.split_once("*/").map_or("", |(_, r)| r);
        } else {
            return s.is_empty();
        }
    }
}

/// Returns the index just after `needle`, or the end of input.
fn skip_until(b: &[u8], from: usize, needle: &[u8]) -> usize {
    let mut i = from;
    while i + needle.len() <= b.len() {
        if &b[i..i + needle.len()] == needle {
            return i + needle.len();
        }
        i += 1;
    }
    b.len()
}

/// If `$tag$` starts at `i`, returns the index after its closing `$`.
fn dollar_tag(b: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if b.get(j).is_some_and(|c| c.is_ascii_digit()) {
        return None; // $1 parameter
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
        j += 1;
    }
    (b.get(j) == Some(&b'$')).then_some(j + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(sql: &str) -> Vec<&str> {
        split_statements(sql)
            .into_iter()
            .map(|s| &sql[s.trimmed])
            .collect()
    }

    #[test]
    fn splits_simple() {
        assert_eq!(texts("select 1; select 2;"), ["select 1", "select 2"]);
        assert_eq!(texts("select 1"), ["select 1"]);
        assert_eq!(texts("  ;; \n"), Vec::<&str>::new());
    }

    #[test]
    fn respects_quotes_and_comments() {
        let sql = "select ';' as a, \"x;y\" -- c;\nfrom t; /* ; */ select 2";
        assert_eq!(
            texts(sql),
            ["select ';' as a, \"x;y\" -- c;\nfrom t", "/* ; */ select 2"]
        );
    }

    #[test]
    fn respects_dollar_quotes() {
        let sql = "create function f() returns int as $body$ select 1; $body$ language sql; select $1";
        assert_eq!(texts(sql).len(), 2);
        assert_eq!(texts("do $$ begin; end $$; select 1").len(), 2);
    }

    #[test]
    fn skips_comment_only_tail() {
        assert_eq!(texts("select 1;\n-- done\n"), ["select 1"]);
        assert_eq!(texts("select 1; /* a */ -- b\n/* c */"), ["select 1"]);
    }

    #[test]
    fn statement_under_cursor() {
        let sql = "select 1;\n\nselect 2;\n";
        assert_eq!(&sql[statement_at(sql, 3).unwrap()], "select 1");
        // In whitespace after the first statement → first statement.
        assert_eq!(&sql[statement_at(sql, 10).unwrap()], "select 1");
        assert_eq!(&sql[statement_at(sql, 13).unwrap()], "select 2");
        assert_eq!(&sql[statement_at(sql, sql.len()).unwrap()], "select 2");
        assert_eq!(statement_at("", 0), None);
    }

    #[test]
    fn paging() {
        let (sql, shift) = paged_query("select * from t order by id; ", 1001, 2000).unwrap();
        assert_eq!(sql, "select * from (select * from t order by id\n) as _rgrip_page limit 1001 offset 2000");
        assert_eq!(shift, 15);
        assert!(paged_query("-- c\nwith x as (select 1) select * from x -- tail", 10, 0).is_some());
        assert!(paged_query("values (1), (2)", 10, 0).is_some());
        // Comments after the `;` stay outside the subquery.
        let (sql, _) = paged_query("select 1; -- note\n/* more */", 10, 0).unwrap();
        assert_eq!(sql, "select * from (select 1\n) as _rgrip_page limit 10 offset 0");
        for sql in [
            "select 1; select 2",
            "insert into t values (1) returning *",
            "with d as (delete from t returning *) select * from d",
            "select * into t2 from t",
            "select * from t for update",
            "update t set a = 1",
            "explain select 1",
            "",
        ] {
            assert!(paged_query(sql, 10, 0).is_none(), "{sql}");
        }
        // Keywords inside strings don't count.
        assert!(paged_query("select 'insert into' as x", 10, 0).is_some());
    }

    #[test]
    fn helpers() {
        assert_eq!(quote_ident("a\"b"), "\"a\"\"b\"");
        assert_eq!(char_to_byte("héllo", 2), 3);
        assert_eq!(line_col("ab\ncd", 5), (2, 2));
        assert_eq!(short_type("timestamp with time zone[]"), "timestamptz[]");
        assert_eq!(short_type("character varying(20)"), "varchar(20)");
    }

    #[test]
    fn sorts_a_query_on_the_server() {
        let q = sorted_query("select a, b from t where x = 1;", "b", false).unwrap();
        assert_eq!(q, "select * from (\nselect a, b from t where x = 1\n) as _rgrip_sort order by \"b\" desc nulls last");
        // Still pageable, and writes can't be wrapped.
        assert!(paged_query(&q, 1000, 0).is_some());
        assert!(sorted_query("delete from t", "a", true).is_none());
    }

}
