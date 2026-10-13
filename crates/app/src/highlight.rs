//! Minimal SQL syntax highlighting: keywords, strings, numbers, comments.

use crate::theme;
use eframe::egui::{Color32, FontId, TextFormat, text::LayoutJob};

const KEYWORDS: &[&str] = &[
    "add", "all", "alter", "and", "any", "as", "asc", "begin", "between", "by", "cascade", "case",
    "cast", "check", "column", "commit", "constraint", "create", "cross", "database", "default",
    "delete", "desc", "distinct", "do", "drop", "else", "end", "except", "exists", "explain",
    "false", "fetch", "for", "foreign", "from", "full", "function", "grant", "group", "having",
    "if", "ilike", "in", "index", "inner", "insert", "intersect", "into", "is", "join", "key",
    "language", "lateral", "left", "like", "limit", "materialized", "not", "null", "nulls",
    "offset", "on", "or", "order", "outer", "over", "partition", "primary", "references",
    "returning", "returns", "revoke", "right", "rollback", "schema", "select", "sequence", "set",
    "show", "table", "then", "to", "transaction", "trigger", "true", "truncate", "union",
    "unique", "update", "using", "values", "view", "when", "where", "window", "with",
];

pub fn sql(text: &str, font: FontId) -> LayoutJob {
    let mut job = LayoutJob::default();
    let fmt = |color: Color32, italics: bool| TextFormat {
        font_id: font.clone(),
        color,
        italics,
        ..Default::default()
    };
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let (end, format) = match b[i] {
            b'-' if b.get(i + 1) == Some(&b'-') => {
                (find(b, i, b"\n").unwrap_or(b.len()), fmt(theme::FAINT, true))
            }
            b'/' if b.get(i + 1) == Some(&b'*') => (
                find(b, i + 2, b"*/").map_or(b.len(), |e| e + 2),
                fmt(theme::FAINT, true),
            ),
            b'\'' => (
                find(b, i + 1, b"'").map_or(b.len(), |e| e + 1),
                fmt(theme::STRING, false),
            ),
            b'"' => (
                find(b, i + 1, b"\"").map_or(b.len(), |e| e + 1),
                fmt(theme::TEXT, false),
            ),
            c if c.is_ascii_digit() => {
                let mut e = i;
                while e < b.len() && (b[e].is_ascii_digit() || b[e] == b'.') {
                    e += 1;
                }
                (e, fmt(theme::WARN, false))
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let mut e = i;
                while e < b.len() && (b[e].is_ascii_alphanumeric() || b[e] == b'_') {
                    e += 1;
                }
                let word = text[i..e].to_ascii_lowercase();
                let color = if KEYWORDS.binary_search(&word.as_str()).is_ok() {
                    theme::ACCENT
                } else {
                    theme::TEXT
                };
                (e, fmt(color, false))
            }
            _ => {
                // Run of plain characters; stop at anything that may start a token.
                let mut e = i + 1;
                while e < b.len()
                    && !b[e].is_ascii_alphanumeric()
                    && !matches!(b[e], b'_' | b'\'' | b'"' | b'-' | b'/')
                {
                    e += 1;
                }
                // Never split inside a UTF-8 sequence.
                while !text.is_char_boundary(e) {
                    e += 1;
                }
                (e, fmt(theme::TEXT, false))
            }
        };
        job.append(&text[start..end], 0.0, format);
        i = end;
    }
    job
}

fn find(b: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

#[cfg(test)]
mod tests {
    #[test]
    fn keywords_sorted() {
        let mut sorted = super::KEYWORDS.to_vec();
        sorted.sort();
        assert_eq!(sorted, super::KEYWORDS);
    }

    #[test]
    fn covers_all_text() {
        let src = "select 'é', \"x\" -- c\n/* d */ 12.5 from t; ünï";
        let job = super::sql(src, Default::default());
        assert_eq!(job.text, src);
    }
}
