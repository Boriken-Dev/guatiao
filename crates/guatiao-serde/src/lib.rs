// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! serde for guatiao values: **one pair of impls, every serde format**.
//!
//! The core crate carries values and resolves no dependency to do it.
//! This crate teaches serde to read and write one, which is a better deal
//! than a format per crate: JSON, MessagePack, CBOR, TOML, YAML, RON and
//! anything else with a serde data format all work through the same two
//! types, and a format this crate has never heard of works too.
//!
//! ```
//! use guatiao::value::types::Value;
//! use guatiao_serde::{Presentation, to_serde};
//!
//! let mut map = Value::map();
//! map.set("port", 5900).unwrap();
//! let json = serde_json::to_string(&to_serde(&map)).unwrap();
//! assert_eq!(json, r#"{"port":5900}"#);
//! ```
//!
//! # Why a wrapper rather than `impl Serialize for Value`
//!
//! Two reasons, and either would be enough.
//!
//! **Serialising needs a policy.** JSON has no byte string, so something
//! has to decide how one is spelled, and that decision belongs to whoever
//! is writing the document rather than to this crate. A bare
//! `impl Serialize for Value` has nowhere to put it.
//!
//! **Deserialising needs an allocator.** Every guatiao container carries
//! the allocator that made it, so building one means naming it — and
//! `Deserialize::deserialize` takes no arguments. [`ValueSeed`] is serde's
//! own answer to that: a [`serde::de::DeserializeSeed`] carries state into
//! a deserialisation.
//!
//! (The orphan rule would forbid the bare impls anyway, `Value` and
//! `Serialize` both being foreign here. That is a consequence of the
//! design rather than a reason for it.)
//!
//! # What survives a round trip
//!
//! **Numbers on the way OUT, exactly.** A guatiao number *is* the text
//! that declared it, and it is written verbatim: `1.10` stays `1.10`, and
//! a 200-digit integer and `1e400` both survive as numbers rather than as
//! strings. Nothing here re-formats one.
//!
//! **Numbers on the way IN, as far as the format carries them.** By
//! default `serde_json` resolves any number past `i64`/`u64` to an `f64`
//! **before a visitor is ever called**, so `1.10` arrives as `1.1` and a
//! 200-digit integer arrives rounded — the spelling is gone before this
//! crate can see it.
//!
//! Turn on the **`arbitrary-numbers`** feature and it is not: the number
//! arrives as its own text and `1.10` stays `1.10`. That is more than an
//! arbitrary-precision *number type* preserves, because those hold a
//! value and normalise the spelling; this model holds the text.
//!
//! It is off by default because cargo unifies features across a build, so
//! it reaches `serde_json::Value` in every other crate in a consumer's
//! graph — a decision to take knowingly rather than to inherit.
//!
//! **Reading the token is not behind that feature**, and that is
//! deliberate: because features unify, *any* crate in a graph can turn
//! `serde_json/arbitrary_precision` on, and a reader that did not know the
//! token would then quietly turn every number in every document into a
//! map. A bug with no error attached, caused by a dependency this crate
//! never named.
//!
//! `a_number_past_u64_loses_its_spelling_on_the_way_in` pins both halves,
//! so a change to either is a decision rather than a surprise.
//!
//! **Bytes, in a format that has them.** MessagePack and CBOR carry a byte
//! string natively and get one. JSON does not, so [`Presentation`] decides
//! — and the default is honest about the cost.
//!
//! **Not absent.** `GUATIAO_ABSENT` means "there is no value here", which
//! a map expresses by not holding the key at all. Reaching one inside a
//! list is refused rather than written as null, because "nothing" and
//! "nothing, deliberately" are different statements.

// NO `forbid(unsafe_code)` HERE, and that is the point of the attribute:
// it binds child modules, so a root carrying it would bind `exports` too.
// Every other module in this crate carries it instead, `exports` is the
// one exemption, and `tests/forbid_unsafe_per_module.rs` is what keeps
// that true. Same arrangement as the core crate's `library` module.
#![deny(missing_docs)]

mod de;
// The `extern "C"` surface. Always compiled, like the core's: this is
// an FFI library, and a surface that appears only when somebody
// remembers a flag is one a C caller cannot rely on.
pub mod exports;
mod ser;
pub mod text;

pub use de::{ValueSeed, from_serde};
pub use ser::{Serializable, to_serde, to_serde_with};
pub use text::Error;

/// Standard base64 (RFC 4648 §4), the `A-Za-z0-9+/` alphabet padded to a
/// multiple of four.
///
/// Exposed because [`Bytes::DataUri`] and [`Bytes::Base64`] are built on
/// it and a consumer reading one back needs the same alphabet.
///
/// **Standard, not URL-safe.** Inside a JSON string `+` and `/` need no
/// escaping, so the URL-safe alphabet would buy nothing and differ from
/// what every other producer emits.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let sextets = [n >> 18, (n >> 12) & 63, (n >> 6) & 63, n & 63];
        for (i, s) in sextets.iter().enumerate() {
            // A chunk of one byte fills two sextets, of two bytes three;
            // the rest is padding rather than data.
            if i <= chunk.len() {
                out.push(ALPHABET[*s as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The reverse, or `None` for anything that is not standard base64.
///
/// Strict: it refuses a wrong-length input, a character outside the
/// alphabet, and padding in the middle. A lenient decoder would turn a
/// typo into different bytes rather than into an error.
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    fn sextet(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => u32::from(c - b'A'),
            b'a'..=b'z' => u32::from(c - b'a') + 26,
            b'0'..=b'9' => u32::from(c - b'0') + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        })
    }

    let bytes = text.as_bytes();
    // is_multiple_of is newer than this workspace's MSRV floor.
    if bytes.len() % 4 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let pad = chunk.iter().rev().take_while(|&&c| c == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            let s = if i >= 4 - pad {
                // Padding, and it may only be at the end of the LAST
                // chunk: `=` anywhere else is a malformed document rather
                // than a zero.
                if !std::ptr::eq(chunk.as_ptr(), bytes[bytes.len() - 4..].as_ptr()) {
                    return None;
                }
                0
            } else {
                sextet(c)?
            };
            n = (n << 6) | s;
        }
        let full = n.to_be_bytes();
        out.extend_from_slice(&full[1..4 - pad]);
    }
    Some(out)
}

/// How a byte string is written into a format that has no byte string.
///
/// A format that **does** have one — MessagePack, CBOR, bincode — is
/// handed the bytes natively whatever this says, because a native byte
/// string round-trips perfectly and a spelling would only lose that.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Bytes {
    /// An RFC 2397 `data:;base64,…` URI in an ordinary string.
    ///
    /// **The default**, and the same spelling a guatiao schema already
    /// uses for a byte-valued default — so one document cannot contain
    /// two answers.
    ///
    /// The cost, stated rather than hidden: it is indistinguishable from a
    /// string that happens to hold that text. [`Bytes::read_data_uris`]
    /// decides whether reading turns one back into bytes, and it is off
    /// unless asked for.
    #[default]
    DataUri,
    /// Bare base64 in a string, with no prefix. Shorter, and gives a
    /// reader nothing at all to recognise it by.
    Base64,
    /// A JSON array of numbers, `[1,2,255]`.
    ///
    /// Unambiguous against a string and unambiguous to read back, at
    /// roughly four characters a byte. The right answer when a document is
    /// read by something that already knows the field is bytes.
    Array,
    /// Refuse. A format with no byte string cannot carry one, and saying
    /// so beats writing a spelling the reader may not undo.
    Refuse,
}

impl Bytes {
    /// Whether reading a `data:;base64,…` string turns it back into bytes.
    ///
    /// **Off by default, and it is a trade rather than an oversight.** On,
    /// a byte string survives a JSON round trip; also on, a string a
    /// person wrote that happens to look like a data URI silently becomes
    /// bytes. Off, a string is always a string.
    pub const fn read_data_uris(self) -> bool {
        false
    }
}

/// How a number is handed to the format.
///
/// A guatiao number is arbitrary-precision TEXT, and most formats have no
/// such thing — so something has to choose, and only the caller knows
/// which format is on the other end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Numbers {
    /// `i64` or `u64` where it fits, otherwise `f64`.
    ///
    /// **The default, because every format understands it.** A number past
    /// `f64`'s precision is rounded, which is all a format with no
    /// arbitrary-precision number can do with one.
    #[default]
    Native,
    /// The text itself, spliced in through serde_json's reserved token.
    ///
    /// Exact — `1.10` stays `1.10` and a 200-digit integer survives — and
    /// **only serde_json understands it**. Anything else writes a literal
    /// one-entry map with a startling key, which is why this is a stated
    /// policy rather than something guessed from `is_human_readable`.
    /// [`text::json`] sets it; nothing else should.
    RawText,
}

/// What a writer decides and a reader is told.
///
/// Built rather than passed as arguments, so a later question — how a
/// number is written in a format with no big integers, say — is a new
/// method here rather than a new parameter at every call site.
///
/// ```
/// use guatiao_serde::{Bytes, Presentation};
///
/// let how = Presentation::new().bytes(Bytes::Array);
/// assert_eq!(how.bytes_as(), Bytes::Array);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Presentation {
    bytes: Bytes,
    numbers: Numbers,
    read_data_uris: bool,
}

impl Presentation {
    /// The defaults: bytes as a `data:;base64,` URI when the format has no
    /// byte string of its own, and a string read back as a string.
    pub const fn new() -> Presentation {
        Presentation {
            bytes: Bytes::DataUri,
            numbers: Numbers::Native,
            read_data_uris: false,
        }
    }

    /// How a byte string is written where the format has none.
    pub const fn bytes(mut self, bytes: Bytes) -> Presentation {
        self.bytes = bytes;
        self
    }

    /// Turn a `data:;base64,…` string back into bytes when reading.
    ///
    /// See [`Bytes::read_data_uris`] for what this trades away.
    pub const fn reading_data_uris(mut self) -> Presentation {
        self.read_data_uris = true;
        self
    }

    /// How a number is handed to the format. See [`Numbers`].
    pub const fn numbers(mut self, numbers: Numbers) -> Presentation {
        self.numbers = numbers;
        self
    }

    /// What it decided about bytes.
    pub const fn bytes_as(self) -> Bytes {
        self.bytes
    }

    /// What it decided about numbers.
    pub const fn numbers_as(self) -> Numbers {
        self.numbers
    }

    /// Whether reading turns a data URI back into bytes.
    pub const fn reads_data_uris(self) -> bool {
        self.read_data_uris
    }
}

/// A byte string's spelling as a data URI.
pub fn data_uri(bytes: &[u8]) -> String {
    format!("data:;base64,{}", base64(bytes))
}

/// The bytes a `data:;base64,…` URI carries, or `None`.
pub fn from_data_uri(text: &str) -> Option<Vec<u8>> {
    from_base64(text.strip_prefix("data:;base64,")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 §10's test vectors, all seven, and each back again.
    ///
    /// They exist precisely to catch the padding mistakes a hand-rolled
    /// codec makes, and they cover all three chunk remainders.
    #[test]
    fn base64_matches_rfc_4648_both_ways() {
        for (raw, encoded) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(raw), encoded, "encoding {raw:?}");
            assert_eq!(
                from_base64(encoded).as_deref(),
                Some(raw),
                "decoding {encoded}"
            );
        }
    }

    #[test]
    fn the_standard_alphabet_is_used_not_the_url_safe_one() {
        // The two differ only at 62 and 63, which is what these bytes hit.
        assert_eq!(data_uri(&[0xfb, 0xff, 0xfe]), "data:;base64,+//+");
        assert_eq!(
            from_data_uri("data:;base64,+//+").as_deref(),
            Some(&[0xfb, 0xff, 0xfe][..])
        );
    }

    /// A lenient decoder turns a typo into different bytes; this one turns
    /// it into `None`.
    #[test]
    fn a_malformed_encoding_is_refused_rather_than_guessed() {
        for bad in ["Zg=", "Zg", "Zm9vYmFy=", "Z g==", "Zg=a", "Z===", "!!!!"] {
            assert_eq!(from_base64(bad), None, "{bad} should not decode");
        }
        assert_eq!(from_data_uri("data:;base64"), None);
        assert_eq!(from_data_uri("not a data uri"), None);
    }
}
