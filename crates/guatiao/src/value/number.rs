// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What counts as a number.
//!
//! A number crosses as the **exact text that declared it**, so something
//! has to say which texts are numbers. That is RFC 8259 §6 and nothing
//! else: the grammar a JSON document already uses, so a producer and a
//! consumer that agree about JSON agree about this without being told.
//!
//! # Why not `f64::from_str`
//!
//! It is a *different* grammar, wrong in both directions. It accepts
//! `inf`, `NaN`, `.5`, `5.` and `+1`, none of which are JSON numbers, and
//! it silently rounds everything it takes — which throws away the whole
//! reason the text is kept. A 200-digit integer is a number here and
//! there is no `f64` for it.
//!
//! The check is at **construction**, never at read. A number that only
//! failed when somebody asked for an integer would report the error a
//! long way from the mistake that caused it.

#![forbid(unsafe_code)]

/// Checks `bytes` against RFC 8259 §6, answering the byte offset at which
/// it stopped conforming.
///
/// The offset is the diagnostic that says *where*, and it equals the
/// text's length when the text ended early (`"1."`, `"1e"`, `""`).
pub(crate) fn validate_json_number(bytes: &[u8]) -> Result<(), usize> {
    let mut i = 0usize;

    if bytes.first() == Some(&b'-') {
        i += 1;
    }

    // int = "0" / ( digit1-9 *DIGIT ). A leading zero may not be followed
    // by more digits, which is what rejects `01` -- octal-looking input
    // that a permissive reader would silently take as 1.
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

    /// Every shape RFC 8259 §6 allows, including the ones a hand-rolled
    /// `f64::from_str` check would silently accept or reject wrongly.
    ///
    /// **This test bites.** Deleting the leading-zero rule makes `01`
    /// pass; deleting the `1*DIGIT` after `.` makes `5.` pass; deferring
    /// to `parse::<f64>()` instead of the grammar makes `.5`, `+1`,
    /// `inf` and `NaN` pass and makes a 200-digit integer round.
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
}
