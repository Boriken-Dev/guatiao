// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! An engine's values and guatiao's, both ways, and the shared vectors
//! every conversion is checked against.

use guatiao::Value;

/// Converts between guatiao values and an engine's own.
///
/// `Native<'a>` may borrow from the converter (an engine context a value
/// lives in), so a conversion runs where that context is: on the engine's
/// thread for a pinned engine.
pub trait Convert {
    /// The engine's value.
    type Native<'a>
    where
        Self: 'a;
    /// Why a value has no counterpart on the other side.
    type Error: core::fmt::Display;

    /// A guatiao value as the engine's.
    fn to_native<'a>(&'a self, value: &Value) -> Result<Self::Native<'a>, Self::Error>;

    /// The engine's value as a guatiao one.
    fn to_value<'a>(&'a self, native: Self::Native<'a>) -> Result<Value, Self::Error>;
}

/// The shared vectors: the values every implementation of the wire
/// encoding is checked against, read from the same file the Rust code and
/// each binding read.
pub mod conformance {
    use guatiao::value::convert::TryAsRef;
    use guatiao::{Alloc, List, Map, Value};

    use super::Convert;

    const VECTORS: &str = include_str!("../vectors/wire.json");

    fn hex(text: &str) -> Option<Vec<u8>> {
        if !text.len().is_multiple_of(2) {
            return None;
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
            .collect()
    }

    /// Every value of the shared vectors, by name.
    pub fn values() -> Vec<(String, Value)> {
        let document = guatiao_serde::text::json::from_str(
            VECTORS,
            Alloc::rust(),
            guatiao_serde::Presentation::new(),
        )
        .expect("the vectors are JSON");
        let list: &List = TryAsRef::<Map>::try_as_ref(&document)
            .and_then(|m| m.get("values"))
            .and_then(TryAsRef::try_as_ref)
            .expect("the vectors hold values");
        list.iter()
            .filter_map(|entry| {
                let entry: &Map = TryAsRef::try_as_ref(entry)?;
                let name = TryAsRef::<str>::try_as_ref(entry.get("name")?)?;
                let bytes = hex(TryAsRef::<str>::try_as_ref(entry.get("hex")?)?)?;
                let value = guatiao::value::wire::decode(&bytes).ok()?;
                Some((name.to_string(), value))
            })
            .collect()
    }

    /// What one vector came back as.
    #[derive(Debug)]
    #[non_exhaustive]
    pub enum Outcome {
        /// Equal to what went in.
        Same,
        /// Converted both ways, and different.
        Changed(Value),
        /// Refused one way or the other, in the converter's words.
        Refused(String),
    }

    /// Every shared vector through `convert` and back: each one's name
    /// and outcome. An engine's tests assert which must be
    /// [`Outcome::Same`] and say why any other is not.
    pub fn round_trip<C: Convert>(convert: &C) -> Vec<(String, Outcome)> {
        values()
            .into_iter()
            .map(|(name, value)| {
                let outcome = match convert
                    .to_native(&value)
                    .and_then(|native| convert.to_value(native))
                {
                    Ok(back) if back == value => Outcome::Same,
                    Ok(back) => Outcome::Changed(back),
                    Err(e) => Outcome::Refused(e.to_string()),
                };
                (name, outcome)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::conformance::{Outcome, round_trip, values};
    use super::*;

    /// The identity: guatiao's own values as the "engine's".
    struct Same;

    impl Convert for Same {
        type Native<'a> = Value;
        type Error = String;
        fn to_native(&self, value: &Value) -> Result<Value, String> {
            Ok(value.clone())
        }
        fn to_value(&self, native: Value) -> Result<Value, String> {
            Ok(native)
        }
    }

    #[test]
    fn the_vectors_are_the_cores_own() {
        let ours = include_bytes!("../vectors/wire.json");
        let core = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../guatiao/tests/vectors/wire.json"),
        )
        .expect("the core's vectors");
        assert!(
            ours[..] == core[..],
            "crates/guatiao-bridge/vectors/wire.json is a copy of the core's: copy it again"
        );
        assert!(values().len() >= 20, "every value decodes");
    }

    #[test]
    fn the_identity_keeps_every_vector() {
        for (name, outcome) in round_trip(&Same) {
            assert!(matches!(outcome, Outcome::Same), "{name}: {outcome:?}");
        }
    }
}
