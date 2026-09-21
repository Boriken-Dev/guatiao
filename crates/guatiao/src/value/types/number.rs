// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! A number: the exact text that declared it.
//!
//! What counts as one is RFC 8259 section 6, checked at construction and
//! never at read. `f64::from_str` is a different grammar in both
//! directions -- it takes `inf`, `NaN`, `.5`, `5.` and `+1`, and it
//! rounds away the reason the text is kept. A 200-digit integer is a
//! number here and there is no `f64` for it.

#![allow(missing_docs)]
use std::fmt;
use std::str::FromStr;

use crate::value::alloc::Alloc;
use crate::value::error::ValueError;

use super::Text;

/// A JSON number, stored as its exact text: `1.10` reads back as `1.10`
/// and `u64::MAX` survives, because nothing converts. A machine width is
/// reached with `TryInto`, where the refusal is visible.
///
/// Wraps a [`Text`], so it has the layout of one and its own arm in the
/// node.
#[repr(transparent)]
#[derive(Debug)]
pub struct Number(Text);

impl Number {
    /// A number from its exact text. Fallible even in the short form,
    /// because the refusal is about the **text**: `"1,5"` is not a JSON
    /// number. Allocation failure aborts, as [`Text::new`] does.
    pub fn new(text: &str) -> Result<Number, ValueError> {
        validate_json_number(text.as_bytes()).map_err(|_| ValueError::NotANumber)?;
        Ok(Number(Text::new(text)))
    }

    /// The same, through an allocator you name. The grammar is checked
    /// before anything is allocated.
    pub fn new_in(alloc: Alloc, text: &str) -> Result<Number, ValueError> {
        validate_json_number(text.as_bytes()).map_err(|_| ValueError::NotANumber)?;
        Ok(Number(Text::new_in(alloc, text)?))
    }

    /// The text this number was written as, as `String::as_str` gives it.
    pub fn as_str(&self) -> &str {
        self
    }

    /// The bytes, for a reader that must tell "not UTF-8" from empty.
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }

    /// A number from a float, through an allocator you name.
    pub fn float_in(alloc: Alloc, v: f64) -> Result<Number, ValueError> {
        Number::new_in(alloc, &float_text(v)?)
    }
}

/// A float's decimal text, or the refusal: `NaN` and the infinities have
/// no JSON spelling, so taking one means inventing a spelling or losing
/// the value at the first boundary.
fn float_text(v: f64) -> Result<String, ValueError> {
    if v.is_finite() {
        Ok(v.to_string())
    } else {
        Err(ValueError::NotANumber)
    }
}

impl Number {
    /// A copy, grown through `alloc`.
    pub fn clone_in(&self, alloc: Alloc) -> Result<Number, ValueError> {
        Ok(Number(self.0.clone_in(alloc)?))
    }

    /// The allocator this number's text recorded.
    pub fn alloc(&self) -> Result<Alloc, ValueError> {
        self.0.alloc()
    }
}

impl Clone for Number {
    fn clone(&self) -> Number {
        Number(self.0.clone())
    }
}

impl PartialEq for Number {
    /// Byte equality of the text, which is what makes `1.10` different
    /// from `1.1`. Comparing as numbers would be the lossy view.
    fn eq(&self, other: &Number) -> bool {
        self.as_bytes() == other.as_bytes()
    }
}

impl Eq for Number {}

/// The text, as a `String` gives its `str`.
impl std::ops::Deref for Number {
    type Target = str;

    fn deref(&self) -> &str {
        // SAFETY: a `Number` is checked once, when it is made -- by its
        // constructor, or for a foreign one by the door that reads it out
        // of a value -- and the JSON grammar is ASCII. Nothing changes its
        // text afterwards, so it is never checked again.
        unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }
    }
}

/// For a bound, which does not deref: `fn f(x: impl AsRef<str>)` takes a
/// `Number` only through this, as it takes a `String`.
impl AsRef<str> for Number {
    fn as_ref(&self) -> &str {
        self
    }
}

impl AsRef<[u8]> for Number {
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

/// By bytes, as its equality is: `1.10` and `1.1` hash apart.
impl std::hash::Hash for Number {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_bytes().hash(state);
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Number {
    type Err = ValueError;

    fn from_str(text: &str) -> Result<Number, ValueError> {
        Number::new(text)
    }
}

/// Every integer width, as its **decimal text**: `u64::MAX` and
/// `i128::MIN` cross as themselves, with no machine width to overflow.
macro_rules! number_from_integer {
    ($($t:ty),* $(,)?) => {$(
        impl From<$t> for Number {
            fn from(v: $t) -> Number {
                // An integer's decimal text is a JSON number by
                // construction, so the only refusal `new` can make is one
                // this cannot reach.
                Number::new(&v.to_string()).expect("an integer's decimal text is a JSON number")
            }
        }
    )*};
}

number_from_integer!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

/// A float converts **fallibly**, which keeps the refusal out of every
/// consumer's own formatting.
impl TryFrom<f64> for Number {
    type Error = ValueError;

    fn try_from(v: f64) -> Result<Number, ValueError> {
        Number::new(&float_text(v)?)
    }
}

impl TryFrom<f32> for Number {
    type Error = ValueError;

    fn try_from(v: f32) -> Result<Number, ValueError> {
        Number::try_from(f64::from(v))
    }
}

/// Checks `bytes` against RFC 8259 section 6, answering the byte offset at
/// which it stopped conforming.
///
/// The offset equals the text's length when the text ended early (`"1."`,
/// `"1e"`, `""`).
pub(crate) fn validate_json_number(bytes: &[u8]) -> Result<(), usize> {
    let mut i = 0usize;

    if bytes.first() == Some(&b'-') {
        i += 1;
    }

    // int = "0" / ( digit1-9 *DIGIT ). A leading zero may not be followed
    // by more digits, which is what rejects `01`.
    match bytes.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while matches!(bytes.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return Err(i),
    }

    // frac = "." 1*DIGIT -- at least one digit, so `5.` is not a number.
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        if !matches!(bytes.get(i), Some(b'0'..=b'9')) {
            return Err(i);
        }
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
    }

    // exp = ("e" / "E") [ "-" / "+" ] 1*DIGIT. The sign IS allowed here,
    // unlike the leading `+` the grammar refuses.
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(bytes.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if !matches!(bytes.get(i), Some(b'0'..=b'9')) {
            return Err(i);
        }
        while matches!(bytes.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
    }

    // Trailing anything -- `1_000`, `0x1F`, `5 ` -- is not a number.
    if i != bytes.len() {
        return Err(i);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every shape RFC 8259 section 6 allows, and the ones a `f64` check
    /// would take or reject wrongly. **This test bites**: deleting the
    /// leading-zero rule makes `01` pass, deleting the `1*DIGIT` after
    /// `.` makes `5.` pass, and deferring to `parse::<f64>()` makes `.5`,
    /// `+1`, `inf` and `NaN` pass.
    #[test]
    fn the_json_number_grammar_is_accepted_exactly() {
        for text in [
            "0",
            "-0",
            "1",
            "-1",
            "1234567890",
            "1.0",
            "1.10", // a trailing zero survives, because the text is the value
            "-2.5",
            "1e10",
            "1E10",
            "1e+10",
            "1e-10",
            "-2.5E-3",
            "1e400", // no finite f64, still a number
            "0.0000000000000000000000000001",
        ] {
            assert_eq!(
                validate_json_number(text.as_bytes()),
                Ok(()),
                "{text} is a JSON number and must be accepted"
            );
        }

        // A 200-digit integer: bigger than i64, bigger than f64's exact
        // range, and still just text.
        let big = "9".repeat(200);
        assert_eq!(validate_json_number(big.as_bytes()), Ok(()));

        for (text, position) in [
            ("", 0usize),
            ("+1", 0),
            (".5", 0),
            ("5.", 2),
            ("01", 1),
            ("00", 1),
            ("-01", 2),
            ("0x1F", 1),
            ("Infinity", 0),
            ("-Infinity", 1),
            ("NaN", 0),
            ("nan", 0),
            ("inf", 0),
            ("1_000", 1),
            ("1e", 2),
            ("1e+", 3),
            ("1.2.3", 3),
            ("--1", 1),
            (" 5", 0),
            ("5 ", 1),
            ("0b101", 1),
            ("1,000", 1),
        ] {
            assert_eq!(
                validate_json_number(text.as_bytes()),
                Err(position),
                "{text} is not a JSON number, and the offset must say where"
            );
        }
    }

    /// The type carries the grammar: a refusal is a refusal whichever
    /// door it comes through.
    #[test]
    fn a_number_keeps_the_text_it_was_given() {
        assert_eq!(Number::new("1.10").expect("a JSON number").as_str(), "1.10");
        assert_eq!(Number::from(u64::MAX).as_str(), "18446744073709551615");
        assert_eq!(Number::from(i128::MIN).as_str(), &i128::MIN.to_string());
        assert_eq!(Number::new("1,5"), Err(ValueError::NotANumber));
        assert_eq!("5".parse::<Number>().expect("a JSON number").as_str(), "5");
        assert_eq!(Number::try_from(f64::NAN), Err(ValueError::NotANumber));
        assert_eq!(Number::try_from(f32::INFINITY), Err(ValueError::NotANumber));
        assert_ne!(
            Number::new("1.10").expect("a JSON number"),
            Number::new("1.1").expect("a JSON number")
        );
    }
}
