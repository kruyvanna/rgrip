//! Find and replace in the console editor: where the matches are, which one
//! comes next, and the text after replacing them. Offsets are byte ranges on
//! char boundaries, so they're safe to slice with for any script (Khmer too).

use std::ops::Range;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    pub case_sensitive: bool,
    /// The match must not touch a letter, digit or `_` on either side.
    pub whole_word: bool,
}

/// All non-overlapping matches of `query` in `text`, in order. An empty query matches nothing.
pub fn find_all(text: &str, query: &str, opts: Options) -> Vec<Range<usize>> {
    let needle: Vec<char> = query.chars().collect();
    let mut found = Vec::new();
    if needle.is_empty() {
        return found;
    }
    let same = |a: char, b: char| a == b || (!opts.case_sensitive && a.to_lowercase().eq(b.to_lowercase()));
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut skip_to = 0;
    for (start, _) in text.char_indices() {
        if start < skip_to {
            continue;
        }
        let mut chars = text[start..].chars();
        let mut len = 0;
        let hit = needle.iter().all(|&n| match chars.next() {
            Some(c) if same(c, n) => {
                len += c.len_utf8();
                true
            }
            _ => false,
        });
        if !hit {
            continue;
        }
        let end = start + len;
        if opts.whole_word && (text[..start].chars().next_back().is_some_and(is_word) || text[end..].chars().next().is_some_and(is_word)) {
            continue;
        }
        found.push(start..end);
        skip_to = end;
    }
    found
}

/// The first match starting at or after `pos`, wrapping to the first one.
pub fn next_from(matches: &[Range<usize>], pos: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().position(|m| m.start >= pos).unwrap_or(0))
}

/// The last match starting before `pos`, wrapping to the last one.
pub fn prev_before(matches: &[Range<usize>], pos: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    Some(matches.iter().rposition(|m| m.start < pos).unwrap_or(matches.len() - 1))
}

/// `text` with every one of `matches` (from [`find_all`]) replaced by `with`.
pub fn replace_all(text: &str, matches: &[Range<usize>], with: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for m in matches {
        out.push_str(&text[at..m.start]);
        out.push_str(with);
        at = m.end;
    }
    out.push_str(&text[at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANY: Options = Options { case_sensitive: false, whole_word: false };

    #[test]
    fn finds_case_insensitively_by_default() {
        let text = "Select a, SELECT b, select";
        assert_eq!(find_all(text, "select", ANY), vec![0..6, 10..16, 20..26]);
        let exact = Options { case_sensitive: true, ..ANY };
        assert_eq!(find_all(text, "select", exact), vec![20..26]);
        assert!(find_all(text, "", ANY).is_empty());
    }

    #[test]
    fn matches_do_not_overlap() {
        assert_eq!(find_all("aaaa", "aa", ANY), vec![0..2, 2..4]);
    }

    #[test]
    fn whole_word_only() {
        let opts = Options { whole_word: true, ..ANY };
        assert_eq!(find_all("id, user_id, id2, (id)", "id", opts), vec![0..2, 19..21]);
    }

    #[test]
    fn non_ascii_offsets_are_char_boundaries() {
        let text = "select 'សួស្តី' as ក, 'ÉCOLE' as é; select 'école'";
        for q in ["ក", "សួស្តី", "école", "é"] {
            for m in find_all(text, q, ANY) {
                assert!(text.is_char_boundary(m.start) && text.is_char_boundary(m.end));
            }
        }
        let m = find_all(text, "សួស្តី", ANY);
        assert_eq!(m.len(), 1);
        assert_eq!(&text[m[0].clone()], "សួស្តី");
        // Case folding across a multi-byte letter.
        let m = find_all(text, "école", ANY);
        assert_eq!(m.iter().map(|r| &text[r.clone()]).collect::<Vec<_>>(), ["ÉCOLE", "école"]);
    }

    #[test]
    fn next_and_previous_wrap() {
        let m = vec![2..4, 10..12, 20..22];
        assert_eq!(next_from(&m, 0), Some(0));
        assert_eq!(next_from(&m, 4), Some(1));
        assert_eq!(next_from(&m, 21), Some(0));
        assert_eq!(prev_before(&m, 10), Some(0));
        assert_eq!(prev_before(&m, 2), Some(2));
        assert_eq!(prev_before(&m, 99), Some(2));
        assert_eq!(next_from(&[], 0), None);
        assert_eq!(prev_before(&[], 0), None);
    }

    #[test]
    fn replaces_everything() {
        let text = "a ក a";
        let m = find_all(text, "a", ANY);
        assert_eq!(replace_all(text, &m, "bb"), "bb ក bb");
        assert_eq!(replace_all(text, &[], "x"), text);
        let m = find_all("aXa", "x", ANY);
        assert_eq!(replace_all("aXa", &m, ""), "aa");
    }
}
