//! Auto-closing quotes and brackets in the SQL editor. Pure text edits on
//! byte offsets; the editor turns them into cursor moves.

use std::ops::Range;

/// The closing character for an opening one the editor pairs.
fn closer(open: char) -> Option<char> {
    match open {
        '\'' => Some('\''),
        '"' => Some('"'),
        '(' => Some(')'),
        '[' => Some(']'),
        _ => None,
    }
}

/// Typing `typed` with `sel` selected (byte range). Returns the new text and
/// selection, or `None` to let the editor insert the character as usual.
/// - an opening quote or bracket inserts its pair, cursor in between, or
///   wraps the selection;
/// - typing the closer that's already right after the cursor steps over it.
pub fn type_char(text: &str, sel: Range<usize>, typed: char) -> Option<(String, Range<usize>)> {
    let next = text[sel.end..].chars().next();
    let prev = text[..sel.start].chars().next_back();
    let quote = typed == '\'' || typed == '"';
    if sel.start == sel.end && matches!(typed, '\'' | '"' | ')' | ']') && next == Some(typed) {
        // Step over a closer (for a quote: one we just paired).
        let at = sel.end + typed.len_utf8();
        return Some((text.to_string(), at..at));
    }
    let close = closer(typed)?;
    if sel.start != sel.end {
        let inner = &text[sel.clone()];
        let out = format!("{}{typed}{inner}{close}{}", &text[..sel.start], &text[sel.end..]);
        let start = sel.start + typed.len_utf8();
        return Some((out, start..start + inner.len()));
    }
    // Only pair where a pair makes sense: before whitespace, the end, or
    // punctuation; and not a quote right after a word (it's, o'brien).
    let free_after = next.is_none_or(|c| c.is_whitespace() || matches!(c, ')' | ']' | ',' | ';'));
    let word_before = prev.is_some_and(|c| c.is_alphanumeric() || c == '_');
    if !free_after || (quote && word_before) {
        return None;
    }
    let out = format!("{}{typed}{close}{}", &text[..sel.start], &text[sel.end..]);
    let at = sel.start + typed.len_utf8();
    Some((out, at..at))
}

/// Backspace with the cursor at byte `at`, between an empty pair: delete
/// both. `None` otherwise.
pub fn backspace(text: &str, at: usize) -> Option<(String, usize)> {
    let prev = text[..at].chars().next_back()?;
    let next = text[at..].chars().next()?;
    if closer(prev) != Some(next) {
        return None;
    }
    let start = at - prev.len_utf8();
    let out = format!("{}{}", &text[..start], &text[at + next.len_utf8()..]);
    Some((out, start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_steps_over_and_wraps() {
        // A quote pairs, then typing it again steps over the closer.
        let (t, sel) = type_char("where a = ", 10..10, '\'').unwrap();
        assert_eq!((t.as_str(), sel.clone()), ("where a = ''", 11..11));
        let t = format!("{}x{}", &t[..11], &t[11..]);
        assert_eq!(type_char(&t, 12..12, '\''), Some((t.clone(), 13..13)));
        // Brackets, before punctuation.
        assert_eq!(type_char("count;", 5..5, '('), Some(("count();".to_string(), 6..6)));
        assert_eq!(type_char("f()", 2..2, ')'), Some(("f()".to_string(), 3..3)));
        // Wrap a selection and keep it selected.
        assert_eq!(type_char("select abc", 7..10, '"'), Some(("select \"abc\"".to_string(), 8..11)));
        // Khmer is fine around it.
        let (t, sel) = type_char("= ", 2..2, '\'').unwrap();
        assert_eq!((t.as_str(), sel), ("= ''", 3..3));
    }

    #[test]
    fn leaves_words_and_text_alone() {
        // Apostrophe after a word, and before text: just the character.
        assert_eq!(type_char("it", 2..2, '\''), None);
        assert_eq!(type_char("abc", 0..0, '('), None);
        assert_eq!(type_char("x", 1..1, 'a'), None);
    }

    #[test]
    fn backspace_removes_an_empty_pair() {
        assert_eq!(backspace("a = ''", 5), Some(("a = ".to_string(), 4)));
        assert_eq!(backspace("f()", 2), Some(("f".to_string(), 1)));
        assert_eq!(backspace("'a'", 2), None);
        assert_eq!(backspace("", 0), None);
    }
}
