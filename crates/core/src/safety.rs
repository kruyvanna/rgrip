//! Classifies SQL before it runs, so the UI can ask first on risky
//! statements. Works on tokens (strings and comments are skipped) and never
//! parses fully: when unsure, a statement counts as a write.

use crate::complete::{Tok, Token, tokenize};
use crate::sql::split_statements;
use std::ops::Range;

/// A statement that is not a plain read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flagged {
    /// Trimmed byte range of the statement in the classified text.
    pub range: Range<usize>,
    /// Dangerous on any connection (data or schema loss), with the reason.
    pub danger: Option<&'static str>,
    /// Changes data or schema (dangerous statements are writes too).
    pub write: bool,
}

/// Every statement of `sql` that writes or is dangerous; plain reads are omitted.
pub fn classify(sql: &str) -> Vec<Flagged> {
    split_statements(sql)
        .into_iter()
        .filter_map(|s| {
            let toks = tokenize(&sql[s.trimmed.clone()]);
            let (write, danger) = classify_tokens(&toks);
            (write || danger.is_some()).then_some(Flagged { range: s.trimmed, danger, write: true })
        })
        .collect()
}

/// Whether running `sql` should ask first: anything dangerous, or any write
/// when `production`.
pub fn needs_confirmation(sql: &str, production: bool) -> bool {
    classify(sql).iter().any(|f| f.danger.is_some() || (production && f.write))
}

/// Statements that never change data, whatever follows them.
const READS: &[&str] = &[
    "select", "values", "table", "show", "set", "reset", "begin", "start", "commit", "end", "rollback", "abort", "savepoint", "release", "fetch", "move", "close", "declare",
    "deallocate", "discard", "listen", "unlisten", "prepare",
];
const DML: &[&str] = &["insert", "update", "delete", "merge"];

fn classify_tokens(t: &[Token]) -> (bool, Option<&'static str>) {
    // Skips the "(" of a parenthesized `(select …) union …`.
    let Some(at) = t.iter().position(|x| matches!(x.tok, Tok::Word { quoted: false, .. })) else { return (false, None) };
    let first = t[at].word().unwrap_or_default();
    match first {
        "explain" => return explain(t, at),
        "truncate" => return (true, Some("TRUNCATE removes every row")),
        "drop" => return (true, Some("DROP deletes objects and their data")),
        "alter" if t.iter().any(|x| x.is_word("drop")) => return (true, Some("ALTER … DROP removes a column or constraint")),
        "copy" => return (depth0(t).any(|x| x.is_word("from")), None),
        "with" => return with_statement(t),
        "delete" | "update" => return (true, missing_where(t, at, first)),
        // SELECT … INTO creates a table.
        "select" if depth0(t).any(|x| x.is_word("into")) => return (true, None),
        _ => {}
    }
    (!READS.contains(&first), None)
}

/// A DELETE/UPDATE starting at `at` is unfiltered unless a WHERE sits at its
/// own nesting level; a WHERE inside a subquery doesn't protect it.
fn missing_where(t: &[Token], at: usize, kw: &str) -> Option<&'static str> {
    let mut depth = 0i32;
    for x in &t[at + 1..] {
        match x.tok {
            Tok::Punct('(') => depth += 1,
            Tok::Punct(')') => {
                depth -= 1;
                if depth < 0 {
                    break;
                }
            }
            _ if depth == 0 && x.is_word("where") => return None,
            _ => {}
        }
    }
    Some(if kw == "delete" { "DELETE without WHERE removes every row" } else { "UPDATE without WHERE changes every row" })
}

/// `with …` is a write when a CTE or the main statement is DML.
fn with_statement(t: &[Token]) -> (bool, Option<&'static str>) {
    let (mut write, mut danger) = (false, None);
    for (i, x) in t.iter().enumerate() {
        let Tok::Word { text, quoted: false } = &x.tok else { continue };
        // A DML keyword opens a CTE body "(" or is the main statement after ")".
        // (`for update`, `do update`, `on delete` follow words instead.)
        let opens = i > 0 && (t[i - 1].is_punct('(') || t[i - 1].is_punct(')'));
        if !opens || !DML.contains(&text.as_str()) {
            continue;
        }
        write = true;
        if (text == "delete" || text == "update") && danger.is_none() {
            danger = missing_where(t, i, text);
        }
    }
    (write || depth0(t).any(|x| x.is_word("into")), danger)
}

/// EXPLAIN only plans, but ANALYZE executes the statement.
fn explain(t: &[Token], at: usize) -> (bool, Option<&'static str>) {
    let mut i = at + 1;
    let mut analyze = false;
    if t.get(i).is_some_and(|x| x.is_punct('(')) {
        // `(analyze false)` and `(analyze 0)` are rare enough to count as on.
        while i < t.len() && !t[i].is_punct(')') {
            analyze |= t[i].is_word("analyze") || t[i].is_word("analyse");
            i += 1;
        }
        i += 1;
    } else {
        while t.get(i).is_some_and(|x| x.is_word("analyze") || x.is_word("analyse") || x.is_word("verbose")) {
            analyze |= !t[i].is_word("verbose");
            i += 1;
        }
    }
    if !analyze || i >= t.len() {
        return (false, None);
    }
    classify_tokens(&t[i..])
}

/// Tokens outside any parentheses (the parentheses themselves excluded).
fn depth0(t: &[Token]) -> impl Iterator<Item = &Token> {
    let mut depth = 0i32;
    t.iter().filter(move |x| match x.tok {
        Tok::Punct('(') => {
            depth += 1;
            false
        }
        Tok::Punct(')') => {
            depth -= 1;
            false
        }
        _ => depth == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (write, dangerous) per flagged statement.
    fn kinds(sql: &str) -> Vec<(bool, bool)> {
        classify(sql).iter().map(|f| (f.write, f.danger.is_some())).collect()
    }

    #[test]
    fn reads_are_clean() {
        for sql in [
            "select 1",
            "select * from t where a = 1;",
            "(select 1) union (select 2)",
            "with a as (select 1) select * from a",
            "explain select * from t",
            "explain delete from t",
            "explain analyze select * from t",
            "show search_path; set search_path = x; begin; commit",
            "values (1), (2)",
            "table t",
            "select * from t for update",
            "-- delete from t\nselect 1",
            "select 'delete from t; drop table x'",
            "select $$update t set a = 1$$",
            "select \"drop\" from t",
            "",
            "-- just a comment",
            "copy (select * from t) to stdout",
        ] {
            assert!(classify(sql).is_empty(), "{sql}");
        }
    }

    #[test]
    fn where_matters() {
        assert_eq!(kinds("update t set a = 1 where id = 2"), [(true, false)]);
        assert_eq!(kinds("update t set a = 1"), [(true, true)]);
        assert_eq!(kinds("delete from t"), [(true, true)]);
        assert_eq!(kinds("delete from t where id = 1"), [(true, false)]);
        assert_eq!(kinds("delete from t using (select 1 where true) s"), [(true, true)]);
        assert_eq!(kinds("update t set a = (select 1 where true)"), [(true, true)]);
        assert_eq!(kinds("update t set a = 1 from u where u.id = t.id"), [(true, false)]);
        assert_eq!(kinds("delete from t -- where id = 1"), [(true, true)]);
        assert_eq!(kinds("delete from t /* where */ returning 'where'"), [(true, true)]);
    }

    #[test]
    fn dangerous_ddl() {
        assert_eq!(kinds("truncate t"), [(true, true)]);
        assert_eq!(kinds("DROP TABLE t"), [(true, true)]);
        assert_eq!(kinds("drop index if exists i"), [(true, true)]);
        assert_eq!(kinds("alter table t drop column c"), [(true, true)]);
        assert_eq!(kinds("alter table t add column c int"), [(true, false)]);
        assert_eq!(kinds("create table t (a int)"), [(true, false)]);
        assert_eq!(kinds("grant select on t to u"), [(true, false)]);
    }

    #[test]
    fn writes_and_unknowns() {
        assert_eq!(kinds("insert into t values (1)"), [(true, false)]);
        assert_eq!(kinds("insert into t values (1) on conflict (id) do update set a = 2"), [(true, false)]);
        assert_eq!(kinds("merge into t using s on true when matched then delete"), [(true, false)]);
        assert_eq!(kinds("do $$ begin delete from t; end $$"), [(true, false)]);
        assert_eq!(kinds("call p()"), [(true, false)]);
        assert_eq!(kinds("vacuum t"), [(true, false)]);
        assert_eq!(kinds("copy t from stdin"), [(true, false)]);
        assert_eq!(kinds("select * into t2 from t"), [(true, false)]);
    }

    #[test]
    fn ctes() {
        assert_eq!(kinds("with d as (delete from t where id = 1 returning *) select * from d"), [(true, false)]);
        assert_eq!(kinds("with d as (delete from t returning *) select * from d"), [(true, true)]);
        assert_eq!(kinds("with s as (select 1) update t set a = 1"), [(true, true)]);
        assert_eq!(kinds("with s as (select 1) update t set a = 1 where true"), [(true, false)]);
        assert_eq!(kinds("with a as (select 1), b as (insert into t select * from a returning 1) select * from b"), [(true, false)]);
    }

    #[test]
    fn explain_analyze_executes() {
        assert_eq!(kinds("explain analyze delete from t where id = 1"), [(true, false)]);
        assert_eq!(kinds("explain analyze delete from t"), [(true, true)]);
        assert_eq!(kinds("explain (analyze, buffers) update t set a = 1 where id = 1"), [(true, false)]);
        assert_eq!(kinds("explain (analyse) insert into t values (1)"), [(true, false)]);
    }

    #[test]
    fn multiple_statements() {
        let sql = "select 1; delete from t; -- c\n insert into u values (1); select 2";
        let f = classify(sql);
        assert_eq!(f.len(), 2);
        assert_eq!(&sql[f[0].range.clone()], "delete from t");
        assert!(f[0].danger.is_some() && f[1].danger.is_none());
        assert!(needs_confirmation(sql, false));
        assert!(!needs_confirmation("select 1; insert into t values (1)", false));
        assert!(needs_confirmation("select 1; insert into t values (1)", true));
        assert!(!needs_confirmation("select 1", true));
    }
}
