// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What the host takes from a guest, bounded: the bytes it copies out of
//! the guest's memory, the tree it decodes them into, and the work
//! decoding maps costs. A guest's own limits cannot bound what the host
//! spends on its behalf, so this does.

use guatiao::Value;
use guatiao::value::wire;
use guatiao_bridge::Limits;

/// The bytes the host takes from one guest at once with no memory limit.
pub const UNLIMITED: u64 = 64 << 20;

/// What one decoded node costs the host, at most, beyond its bytes.
const NODE_COST: u64 = 96;

/// The most work decoding maps may cost: core's decoder looks a key up
/// among the keys before it, so a map of `n` keys costs `n * n`.
pub const MAP_WORK: u64 = 1 << 24;

/// The bytes the host takes from one guest at once: its memory limit, or
/// [`UNLIMITED`].
pub(crate) fn of(limits: &Limits) -> u64 {
    limits.memory.unwrap_or(UNLIMITED)
}

/// Why the host does not decode what a guest handed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Refused {
    /// The tree would take more than `budget` bytes on the host.
    TooLarge { budget: u64 },
    /// A map, or maps together, hold more keys than decoding can afford:
    /// the largest has `keys`.
    Maps { keys: u64 },
}

impl Refused {
    /// Its words, for `what` was handed over.
    pub(crate) fn words(&self, what: &str) -> String {
        match self {
            Refused::TooLarge { budget } => format!(
                "{what} would take more than {budget} bytes on the host (the memory limit, or {UNLIMITED} without one)"
            ),
            Refused::Maps { keys } => format!(
                "{what} holds maps too large to decode (one of {keys} keys; their keys squared may add up to {MAP_WORK})"
            ),
        }
    }
}

fn leb(bytes: &[u8], mut at: usize) -> Option<(u64, usize)> {
    let mut n: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(at)?;
        at += 1;
        n |= u64::from(byte & 0x7f).checked_shl(shift)?;
        if byte & 0x80 == 0 {
            return Some((n, at));
        }
    }
    None
}

/// Map keys compared while decoding per unit of fuel charged.
const COMPARISONS_PER_FUEL: u64 = 4;

/// Walks `bytes` as the wire grammar lays them out, without building
/// anything, and refuses what would cost more than `budget`. What is not
/// a value is left to the decoder to refuse, with its offset. Answers what
/// decoding costs in fuel: a unit per node, and per
/// [`COMPARISONS_PER_FUEL`] map keys compared.
pub(crate) fn check(bytes: &[u8], budget: u64) -> Result<u64, Refused> {
    let fuel = |nodes: u64, work: u64| nodes.saturating_add(work / COMPARISONS_PER_FUEL);
    let len = bytes.len() as u64;
    if len > budget {
        return Err(Refused::TooLarge { budget });
    }
    let mut nodes: u64 = 0;
    let mut work: u64 = 0;
    let mut at = 0usize;
    // Values left to read in each open container, and whether it is a map.
    let mut open: Vec<(u64, bool)> = vec![(1, false)];
    while let Some(top) = open.last_mut() {
        if top.0 == 0 {
            open.pop();
            continue;
        }
        top.0 -= 1;
        let in_map = top.1;
        if in_map {
            let Some((n, after)) = leb(bytes, at) else {
                return Ok(fuel(nodes, work));
            };
            at = after.saturating_add(usize::try_from(n).unwrap_or(usize::MAX));
        }
        let Some(&tag) = bytes.get(at) else {
            return Ok(fuel(nodes, work));
        };
        at += 1;
        nodes += 1;
        if nodes.saturating_mul(NODE_COST).saturating_add(len) > budget {
            return Err(Refused::TooLarge { budget });
        }
        match tag {
            0 | 1 => {}
            2 => at += 1,
            3..=5 => {
                let Some((n, after)) = leb(bytes, at) else {
                    return Ok(fuel(nodes, work));
                };
                at = after.saturating_add(usize::try_from(n).unwrap_or(usize::MAX));
            }
            6 | 7 => {
                let Some((n, after)) = leb(bytes, at) else {
                    return Ok(fuel(nodes, work));
                };
                at = after;
                // Every element is at least a byte: the decoder builds no
                // more elements than bytes remain before it refuses a
                // larger count, and those are walked and counted.
                let n = n.min(bytes.len().saturating_sub(at) as u64);
                if tag == 7 {
                    work = work.saturating_add(n.saturating_mul(n));
                    if work > MAP_WORK {
                        return Err(Refused::Maps { keys: n });
                    }
                }
                open.push((n, tag == 7));
            }
            _ => return Ok(fuel(nodes, work)),
        }
    }
    Ok(fuel(nodes, work))
}

/// Why bytes a guest handed over were not taken.
#[derive(Debug)]
pub(crate) enum Undecoded {
    Refused(Refused),
    Malformed(wire::WireError),
}

/// `bytes` decoded, once [`check`] has found them within `budget`.
pub(crate) fn decode(bytes: &[u8], budget: u64) -> Result<Value, Undecoded> {
    check(bytes, budget).map_err(Undecoded::Refused)?;
    wire::decode(bytes).map_err(Undecoded::Malformed)
}

#[cfg(test)]
mod tests {
    use guatiao::{List, Map};

    use super::*;

    fn bytes(v: Value) -> Vec<u8> {
        wire::to_bytes(&v).expect("encoded")
    }

    #[test]
    fn a_value_within_the_budget_passes_and_its_nodes_are_counted() {
        let mut map = Map::new();
        map.set("a", 1i64);
        let mut list = List::new();
        list.push(map);
        list.push("x");
        assert_eq!(check(&bytes(Value::from(list)), 1 << 20), Ok(4));
        for (_, v) in guatiao_bridge::conformance::values() {
            assert!(check(&bytes(v), 1 << 20).is_ok());
        }
    }

    #[test]
    fn nodes_that_would_swell_are_refused_by_their_count() {
        let mut list = List::new();
        for _ in 0..100_000 {
            list.push(Value::null());
        }
        let encoded = bytes(Value::from(list));
        assert!(encoded.len() < 1 << 20);
        assert_eq!(
            check(&encoded, 1 << 20),
            Err(Refused::TooLarge { budget: 1 << 20 })
        );
        assert!(check(&encoded, 64 << 20).is_ok());
    }

    #[test]
    fn a_count_past_the_bytes_still_counts_the_nodes_the_decoder_would_build() {
        // One more null claimed than follows: the decoder builds 100 000
        // before it finds the end.
        let mut encoded = vec![6u8];
        let mut n: u64 = 100_001;
        while n >= 0x80 {
            encoded.push((n as u8) | 0x80);
            n >>= 7;
        }
        encoded.push(n as u8);
        encoded.extend(core::iter::repeat_n(1u8, 100_000));
        assert_eq!(
            check(&encoded, 1 << 20),
            Err(Refused::TooLarge { budget: 1 << 20 })
        );
        assert!(matches!(
            decode(&encoded, 64 << 20),
            Err(Undecoded::Malformed(_))
        ));
    }

    #[test]
    fn maps_are_refused_by_the_square_of_their_keys() {
        let mut map = Map::new();
        for i in 0..5000 {
            map.set(&format!("k{i}"), Value::null());
        }
        assert_eq!(
            check(&bytes(Value::from(map)), 64 << 20),
            Err(Refused::Maps { keys: 5000 })
        );
        let mut small = Map::new();
        for i in 0..4000 {
            small.set(&format!("k{i}"), Value::null());
        }
        assert!(check(&bytes(Value::from(small)), 64 << 20).is_ok());
    }

    #[test]
    fn what_is_not_a_value_is_left_to_the_decoder() {
        for garbage in [
            &[][..],
            &[0x09],
            &[0x06, 0xff],
            &[0x07, 0x05, 0x01],
            &[0x04, 0x80],
        ] {
            assert!(check(garbage, 1 << 20).is_ok());
            assert!(matches!(
                decode(garbage, 1 << 20),
                Err(Undecoded::Malformed(_))
            ));
        }
    }
}
