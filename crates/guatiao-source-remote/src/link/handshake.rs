// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The handshake's arithmetic: nonces, and the trusted level's proof of a
//! pre-shared key, HMAC-SHA256 over a label and both nonces, each side
//! proving it to the other.

use guatiao::value::convert::{FromValue, TryAsRef};
use guatiao::{Map, Value};

/// What a server proves with.
#[cfg(feature = "tcp")]
pub(crate) const SERVER: &[u8] = b"guatiao-remote server";
/// What a client proves with.
#[cfg(feature = "tcp")]
pub(crate) const CLIENT: &[u8] = b"guatiao-remote client";

/// Sixteen bytes no peer can predict, where the build can make them.
pub(crate) fn nonce() -> [u8; 16] {
    #[cfg(feature = "tcp")]
    {
        let mut out = [0u8; 16];
        getrandom::fill(&mut out).expect("the operating system's random source");
        out
    }
    // Only the trusted level reads a nonce, and it needs `tcp`.
    #[cfg(not(feature = "tcp"))]
    [0u8; 16]
}

/// The proof of `key` for `label` over both nonces.
#[cfg(feature = "tcp")]
pub(crate) fn proof(key: &[u8], label: &[u8], client: &[u8; 16], server: &[u8; 16]) -> Vec<u8> {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(label);
    mac.update(client);
    mac.update(server);
    mac.finalize().into_bytes().to_vec()
}

/// Whether `given` is the proof of `key`, compared in constant time.
#[cfg(feature = "tcp")]
pub(crate) fn proves(
    key: &[u8],
    label: &[u8],
    client: &[u8; 16],
    server: &[u8; 16],
    given: &[u8],
) -> bool {
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    mac.update(label);
    mac.update(client);
    mac.update(server);
    mac.verify_slice(given).is_ok()
}

/// Whether two tokens are the same, in time that does not depend on where
/// they first differ.
pub(crate) fn same_token(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What one side offers in its HELLO or WELCOME.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Offer {
    pub(crate) max_frame: usize,
    pub(crate) max_calls: usize,
    pub(crate) token: Option<String>,
}

impl Offer {
    pub(crate) fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.set("max_frame", self.max_frame as u64);
        map.set("max_calls", self.max_calls as u64);
        if let Some(token) = &self.token {
            map.set("token", token.as_str());
        }
        Value::from(map)
    }

    /// Reads an offer; a missing number takes `fallback`'s.
    pub(crate) fn from_value(value: &Value, fallback: &Offer) -> Offer {
        let map = TryAsRef::<Map>::try_as_ref(value);
        let number = |key: &str, default: usize| {
            map.and_then(|m| m.get(key))
                .and_then(|v| u64::from_value(v).ok())
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(default)
        };
        Offer {
            max_frame: number("max_frame", fallback.max_frame),
            max_calls: number("max_calls", fallback.max_calls),
            token: map
                .and_then(|m| m.get("token"))
                .and_then(TryAsRef::<str>::try_as_ref)
                .map(str::to_string),
        }
    }
}

#[cfg(all(test, feature = "tcp"))]
mod tests {
    use super::*;

    #[test]
    fn a_proof_holds_only_for_its_key_label_and_nonces() {
        let (c, s) = ([1u8; 16], [2u8; 16]);
        let p = proof(b"key", SERVER, &c, &s);
        assert_eq!(p.len(), 32);
        assert!(proves(b"key", SERVER, &c, &s, &p));
        assert!(!proves(b"other", SERVER, &c, &s, &p));
        assert!(!proves(b"key", CLIENT, &c, &s, &p));
        assert!(!proves(b"key", SERVER, &s, &c, &p));
        assert!(!proves(b"key", SERVER, &c, &s, &p[..31]));
    }

    #[test]
    fn nonces_differ() {
        assert_ne!(nonce(), nonce());
    }
}

#[cfg(test)]
mod offer_tests {
    use super::*;

    #[test]
    fn an_offer_round_trips_and_fills_what_is_missing() {
        let offer = Offer {
            max_frame: 1 << 20,
            max_calls: 8,
            token: Some("t".into()),
        };
        assert_eq!(Offer::from_value(&offer.to_value(), &offer), offer);
        let fallback = Offer {
            max_frame: 5,
            max_calls: 6,
            token: None,
        };
        assert_eq!(Offer::from_value(&Value::null(), &fallback), fallback);
        assert!(same_token(b"abc", b"abc"));
        assert!(!same_token(b"abc", b"abd"));
        assert!(!same_token(b"abc", b"ab"));
    }
}
