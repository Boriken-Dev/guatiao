// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The repeated-key check of a wide map, shared by every reader and
//! writer of wire bytes: each key hashed once, under seeds drawn per read
//! or write, into a set sized from the map's count. A hash seen before is
//! a repeat only if the bytes of an earlier key match, which the caller
//! checks against the keys it holds.

#![forbid(unsafe_code)]

use core::hash::{BuildHasher, BuildHasherDefault};
use std::collections::HashSet;
use std::hash::RandomState;

/// A map counting more than this many keys checks each against a set
/// rather than comparing it with every earlier key.
pub(super) const LINEAR: usize = 16;

/// How many more keys a map's count may reserve room for: a count is
/// bounded only by the bytes that remain.
const RESERVE: usize = 1024;

/// Hashes keys under seeds drawn from the standard library's random
/// state: a multiply-and-fold per eight bytes, a few nanoseconds for a
/// short key where SipHash takes several times that. A collision costs a
/// comparison with the earlier keys, never a wrong answer.
#[derive(Debug, Clone, Copy)]
pub(super) struct Seeded(pub(super) [u64; 2]);

impl Seeded {
    /// The seeds `drawn` holds, drawn there first if it holds none.
    pub(super) fn of(drawn: &mut Option<Seeded>) -> Seeded {
        *drawn.get_or_insert_with(|| {
            let state = RandomState::new();
            Seeded([state.hash_one(1u64), state.hash_one(2u64)])
        })
    }

    pub(super) fn hash_one(&self, key: &[u8]) -> u64 {
        const M: u64 = 0x9e37_79b9_7f4a_7c15;
        let fold = |a: u64, b: u64| {
            let p = u128::from(a) * u128::from(b);
            (p as u64) ^ ((p >> 64) as u64)
        };
        let [s0, s1] = self.0;
        let mut h = s0 ^ (key.len() as u64).wrapping_mul(M);
        let (words, rest) = key.as_chunks::<8>();
        for &w in words {
            h = fold(h ^ u64::from_le_bytes(w), s1 ^ M);
        }
        if !rest.is_empty() {
            let mut last = [0u8; 8];
            last[..rest.len()].copy_from_slice(rest);
            h = fold(h ^ u64::from_le_bytes(last), s1);
        }
        fold(h, s0 ^ M)
    }
}

/// The set's hasher: its elements are hashes already.
#[derive(Debug, Default)]
struct Prehashed(u64);

impl core::hash::Hasher for Prehashed {
    fn finish(&self) -> u64 {
        self.0
    }

    // Only `write_u64` is reached, by `HashSet<u64>`; this keeps the
    // trait's contract for anything else.
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = self.0.rotate_left(8) ^ u64::from(b);
        }
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = n;
    }
}

/// The hashes of one wide map's keys.
#[derive(Debug)]
pub(super) struct KeySet {
    seeds: Seeded,
    hashes: HashSet<u64, BuildHasherDefault<Prehashed>>,
}

impl KeySet {
    /// For a map holding `held` keys with `left` more to come, under the
    /// seeds `drawn` holds (drawn there first if it holds none).
    pub(super) fn new(drawn: &mut Option<Seeded>, held: usize, left: u64) -> KeySet {
        let room = usize::try_from(left).unwrap_or(usize::MAX).min(RESERVE);
        KeySet {
            seeds: Seeded::of(drawn),
            hashes: HashSet::with_capacity_and_hasher(
                held.saturating_add(room),
                Default::default(),
            ),
        }
    }

    /// Adds `key`'s hash: false when an earlier key had the same, and
    /// only the bytes can then tell whether it is the same key.
    pub(super) fn insert(&mut self, key: &[u8]) -> bool {
        self.hashes.insert(self.seeds.hash_one(key))
    }
}
