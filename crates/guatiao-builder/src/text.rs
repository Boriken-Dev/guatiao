// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The text of a leaf, and back.
//!
//! coqui-builder holds every setting as one text, read by the leaf's own parser. These two
//! functions are the whole mapping between that text and a guatiao value, stated here so a
//! reader in another language can write the same: a value's own display text is never used.
//!
//! | a value | its text |
//! | --- | --- |
//! | a string | as it is |
//! | a boolean | `true` or `false` |
//! | a number, for an integer type or a `Duration` | its plain decimal expansion, exactly: `1e3` is `1000`, `2.0` is `2`, `-0` is `0` |
//! | a number, for any other type | as written |
//! | null | none: the key is not given |
//! | a list, a map or any other kind | refused, where one text is expected |
//!
//! And a text becomes a value by the leaf's type: `bool` a boolean, the integers and the
//! floats a number when the text is one, every other type (and a number's type whose text is
//! not a number, such as the text that writes a `None`) a string as written.

use alloc::string::{String, ToString};

use coqui_builder::SourceError;
use guatiao::{Alloc, FromValue, Number, Tag, Text, TryAsRef, Value, ValueError};

/// The types whose text is a number when it can be.
const NUMBERS: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64",
];

/// The types whose number is read as a plain decimal.
fn plain(ty: &str) -> bool {
    ty == "Duration" || (NUMBERS.contains(&ty) && !ty.starts_with('f'))
}

/// The text of `value` as the text of a leaf of type `ty`, `None` for null (a key that is not
/// given).
///
/// # Errors
///
/// [`SourceError`] for a list, a map, or a kind with no text.
pub fn from_value(value: &Value, ty: &str) -> Result<Option<String>, SourceError> {
    match value.tag() {
        Ok(Tag::GUATIAO_NULL) => Ok(None),
        Ok(Tag::GUATIAO_STRING) => <&str>::try_from(value)
            .map(|text| Some(text.to_string()))
            .map_err(|_| SourceError::new("text that is not valid UTF-8")),
        Ok(Tag::GUATIAO_BOOL) => bool::from_value(value)
            .map(|b| Some(b.to_string()))
            .map_err(|_| SourceError::new("a boolean that cannot be read")),
        Ok(Tag::GUATIAO_NUMBER) => {
            let number = TryAsRef::<Number>::try_as_ref(value)
                .ok_or_else(|| SourceError::new("a number that cannot be read"))?;
            let written: &str = number.as_ref();
            Ok(Some(if plain(ty) {
                plain_decimal(written).unwrap_or_else(|| written.to_string())
            } else {
                written.to_string()
            }))
        }
        Ok(Tag::GUATIAO_LIST) => Err(SourceError::new("a list where one value is expected")),
        Ok(Tag::GUATIAO_MAP) => Err(SourceError::new("a table where one value is expected")),
        _ => Err(SourceError::new("a value with no text form")),
    }
}

/// The largest exponent expanded; a number past it is left as written for the leaf to refuse.
const EXPONENT: i64 = 400;

/// A JSON number as plain decimal digits: no exponent, no trailing fraction zeros, no `-0`.
/// `None` for a number this does not expand.
fn plain_decimal(written: &str) -> Option<String> {
    let (negative, rest) = match written.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, written),
    };
    let (mantissa, exponent) = match rest.find(['e', 'E']) {
        Some(at) => (&rest[..at], rest[at + 1..].parse::<i64>().ok()?),
        None => (rest, 0),
    };
    if exponent.abs() > EXPONENT {
        return None;
    }
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    if whole.is_empty()
        || !whole
            .bytes()
            .chain(fraction.bytes())
            .all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits: String = whole.chars().chain(fraction.chars()).collect();
    // The decimal point sits after this many digits.
    let point = i64::try_from(whole.len()).ok()?.checked_add(exponent)?;
    let (int_part, frac_part) = if point <= 0 {
        let zeros = "0".repeat(usize::try_from(-point).ok()?);
        (String::new(), alloc::format!("{zeros}{digits}"))
    } else if point as usize >= digits.len() {
        let zeros = "0".repeat(point as usize - digits.len());
        (alloc::format!("{digits}{zeros}"), String::new())
    } else {
        (
            digits[..point as usize].to_string(),
            digits[point as usize..].to_string(),
        )
    };
    let int_part = int_part.trim_start_matches('0');
    let frac_part = frac_part.trim_end_matches('0');
    let int_part = if int_part.is_empty() { "0" } else { int_part };
    let zero = int_part == "0" && frac_part.is_empty();
    let sign = if negative && !zero { "-" } else { "" };
    Some(if frac_part.is_empty() {
        alloc::format!("{sign}{int_part}")
    } else {
        alloc::format!("{sign}{int_part}.{frac_part}")
    })
}

/// The value a leaf of type `ty` with the text `text` is, built in `alloc`.
///
/// # Errors
///
/// [`ValueError`] when `alloc` fails.
pub fn to_value_in(ty: &str, text: &str, alloc: Alloc) -> Result<Value, ValueError> {
    if ty == "bool" {
        match text {
            "true" => return Ok(Value::from(true)),
            "false" => return Ok(Value::from(false)),
            _ => {}
        }
    } else if NUMBERS.contains(&ty)
        && let Ok(number) = Number::new_in(alloc, text)
    {
        return Ok(number.into());
    }
    Ok(Text::new_in(alloc, text)?.into())
}

#[cfg(test)]
mod tests {
    use super::plain_decimal;

    #[test]
    fn a_number_expands_exactly() {
        for (written, plain) in [
            ("1e3", "1000"),
            ("2.0", "2"),
            ("-0", "0"),
            ("0.0", "0"),
            ("1.10", "1.1"),
            ("2e-3", "0.002"),
            (
                "123456789012345678901234567e3",
                "123456789012345678901234567000",
            ),
            ("-1.5E+2", "-150"),
            ("0.25", "0.25"),
            ("90", "90"),
        ] {
            assert_eq!(plain_decimal(written).as_deref(), Some(plain), "{written}");
        }
        assert_eq!(plain_decimal("1e999"), None);
    }
}
