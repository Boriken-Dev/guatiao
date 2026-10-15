// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Enforcing a schema's `pattern`.
//!
//! JSON Schema's patterns are ECMA-262 regular expressions; the `regex`
//! crate's are close, and differ most visibly in `\d`, `\w` and their
//! complements, which are Unicode there (Arabic-Indic digits are `\d`) and
//! ASCII in ECMA-262. [`ecma_classes`] rewrites those four so the two
//! engines agree on them; [`super::vocab::PATTERN`] lists the differences
//! that remain.

#![forbid(unsafe_code)]

/// Whether `text` matches `pattern`, unanchored, as JSON Schema specifies.
///
/// A pattern the `regex` crate cannot compile -- ECMA-262 lookaround or a
/// backreference -- is carried, not enforced: refusing every value for a
/// schema this build cannot evaluate would make the field unusable, the
/// same argument [`Kind::Unknown`](super::KindRef::Unknown) makes.
/// Compiled per call, because this crate keeps no process-global cache;
/// `regex` matches in linear time, so a hostile pattern costs its compile
/// and nothing worse.
#[cfg(feature = "regex")]
pub(super) fn matches(pattern: &str, text: &str) -> bool {
    regex::Regex::new(&ecma_classes(pattern)).map_or(true, |re| re.is_match(text))
}

/// Without the `regex` feature a pattern is carried and never enforced.
#[cfg(not(feature = "regex"))]
pub(super) fn matches(_pattern: &str, _text: &str) -> bool {
    true
}

/// `pattern` with `\d`, `\D`, `\w` and `\W` spelled as the ASCII classes
/// ECMA-262 defines.
///
/// Escape-aware: `\\d` is a backslash then a `d`, and is left alone. In
/// a character class `\d` and `\w` become their ranges (`0-9`,
/// `A-Za-z0-9_`); `\D` and `\W` cannot be written as a range inside a
/// class and are left as they are, which keeps the `regex` crate's
/// Unicode meaning there.
#[cfg_attr(not(feature = "regex"), allow(dead_code))]
pub(super) fn ecma_classes(pattern: &str) -> alloc::borrow::Cow<'_, str> {
    if !pattern.contains(['d', 'D', 'w', 'W']) {
        return alloc::borrow::Cow::Borrowed(pattern);
    }
    let mut out = alloc::string::String::with_capacity(pattern.len() + 8);
    // How many character classes deep the scan is; the `regex` crate
    // nests them (`[a[b]]`, `[[:alpha:]]`), so a counter and not a flag.
    let mut classes = 0usize;
    // The first character after `[` or `[^` is a literal `]`.
    let mut class_start = false;
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('d') if classes == 0 => out.push_str("[0-9]"),
                Some('D') if classes == 0 => out.push_str("[^0-9]"),
                Some('w') if classes == 0 => out.push_str("[A-Za-z0-9_]"),
                Some('W') if classes == 0 => out.push_str("[^A-Za-z0-9_]"),
                Some('d') => out.push_str("0-9"),
                Some('w') => out.push_str("A-Za-z0-9_"),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            },
            '[' => {
                classes += 1;
                out.push('[');
                if chars.peek() == Some(&'^') {
                    out.push('^');
                    chars.next();
                }
                class_start = true;
                continue;
            }
            ']' if classes > 0 && !class_start => {
                classes -= 1;
                out.push(']');
            }
            other => out.push(other),
        }
        class_start = false;
    }
    alloc::borrow::Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::ecma_classes;

    #[test]
    fn the_four_classes_are_ascii_outside_a_character_class() {
        assert_eq!(ecma_classes(r"^\d+$"), "^[0-9]+$");
        assert_eq!(ecma_classes(r"\D\w\W"), "[^0-9][A-Za-z0-9_][^A-Za-z0-9_]");
    }

    #[test]
    fn inside_a_class_the_positive_ones_become_ranges() {
        assert_eq!(ecma_classes(r"[\d.]+"), "[0-9.]+");
        assert_eq!(ecma_classes(r"[^\w-]"), "[^A-Za-z0-9_-]");
        // Not expressible as a range: left alone.
        assert_eq!(ecma_classes(r"[\D]"), r"[\D]");
    }

    #[test]
    fn an_escaped_backslash_is_not_an_escape_of_the_letter() {
        assert_eq!(ecma_classes(r"\\d"), r"\\d");
        assert_eq!(ecma_classes(r"\\\d"), r"\\[0-9]");
    }

    #[test]
    fn a_literal_bracket_first_in_a_class_does_not_close_it() {
        assert_eq!(ecma_classes(r"[]\d]\d"), r"[]0-9][0-9]");
        assert_eq!(ecma_classes(r"[^]\d]\d"), r"[^]0-9][0-9]");
    }

    #[test]
    fn a_pattern_without_the_letters_is_borrowed() {
        assert!(matches!(
            ecma_classes("^abc$"),
            alloc::borrow::Cow::Borrowed(_)
        ));
    }
}
