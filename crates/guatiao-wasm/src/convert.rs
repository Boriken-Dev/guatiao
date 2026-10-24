// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What every value crosses into and out of a guest as: its wire bytes.

use guatiao::Value;
use guatiao::value::wire;
use guatiao_bridge::Convert;

/// guatiao's values as a guest receives them, and back: the wire
/// encoding, so every shared vector crosses unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct WireBytes;

impl Convert for WireBytes {
    type Native<'a> = Vec<u8>;
    type Error = String;

    fn to_native(&self, value: &Value) -> Result<Vec<u8>, String> {
        wire::to_bytes(value).map_err(|e| e.to_string())
    }

    fn to_value(&self, native: Vec<u8>) -> Result<Value, String> {
        wire::decode(&native).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use guatiao_bridge::conformance::{Outcome, round_trip};

    use super::*;

    #[test]
    fn every_shared_vector_crosses_unchanged() {
        let outcomes = round_trip(&WireBytes);
        assert!(outcomes.len() >= 20);
        for (name, outcome) in outcomes {
            assert!(matches!(outcome, Outcome::Same), "{name}: {outcome:?}");
        }
    }
}
