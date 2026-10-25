// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The remote protocol as bytes, with no I/O: frames, their fields, a
//! call's arguments and answer read against its slot's description.
//!
//! ```text
//! frame  := len:u32 LE  body              on a byte stream; a message
//!                                         transport carries the body alone
//! body   := type:u8 fields
//! uint   := LEB128, shortest form         int := uint, zigzagged
//! text   := len:uint UTF-8                bytes := len:uint octets
//! value  := len:uint wire-encoded value
//!
//! HELLO    0x01 "GTRM" version:u8 trust:u8 nonce:16 options:value
//! WELCOME  0x02 "GTRM" version:u8 nonce:16 proof:bytes options:value
//! PROOF    0x03 proof:bytes
//! LIST     0x04 call:uint
//! OPEN     0x05 call:uint library:text stamp:text
//! CLOSE    0x06 library:text stamp:text
//! CALL     0x10 call:uint parent:uint target:uint slot:uint args
//! REPLY    0x11 call:uint status:uint (status 0: answer, else message:text)
//! CANCEL   0x12 call:uint
//! RELEASE  0x13 object:uint
//! PING     0x14 n:uint          PONG 0x15 n:uint
//! GOODBYE  0x16 status:uint message:text
//! ```
//!
//! `docs/remote.md` states it for every language, with the arguments'
//! and answers' forms; `tests/vectors/remote.json` holds frames as bytes.

#![forbid(unsafe_code)]

pub mod args;
pub mod catalog;
pub(crate) mod codec;

use guatiao::{Alloc, Value};

use codec::{Reader, put_bytes, put_text, put_uint, put_value};

pub use args::{Decoded, ObjectRef, decode_answer, decode_args, encode_answer, encode_args};

/// The four bytes a handshake starts with.
pub const MAGIC: [u8; 4] = *b"GTRM";

/// This protocol's version.
pub const VERSION: u8 = 1;

/// The largest body a peer reads before the handshake ends.
pub const HANDSHAKE_MAX_FRAME: usize = 64 << 10;

/// Frame types.
pub mod kind {
    /// The client's first frame.
    pub const HELLO: u8 = 0x01;
    /// The server's answer to it.
    pub const WELCOME: u8 = 0x02;
    /// The client's proof of the link's key.
    pub const PROOF: u8 = 0x03;
    /// Asks the peer's catalog.
    pub const LIST: u8 = 0x04;
    /// Opens a library the catalog listed: its providers are held.
    pub const OPEN: u8 = 0x05;
    /// Lets an opened library go.
    pub const CLOSE: u8 = 0x06;
    /// A call of a slot.
    pub const CALL: u8 = 0x10;
    /// The answer to a call, a list or an open.
    pub const REPLY: u8 = 0x11;
    /// The caller no longer waits.
    pub const CANCEL: u8 = 0x12;
    /// An object the sender held of the receiver's is let go.
    pub const RELEASE: u8 = 0x13;
    /// Asks for a [`PONG`].
    pub const PING: u8 = 0x14;
    /// The answer to a [`PING`].
    pub const PONG: u8 = 0x15;
    /// The link is ending.
    pub const GOODBYE: u8 = 0x16;
}

/// How far a link trusts its peer, chosen by its configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Trust {
    /// The same machine: the operating system's permissions on the
    /// socket, pipe or shared memory are the gate.
    Local,
    /// A network the owner isolates: no encryption, a configured address
    /// range, a pre-shared key proved in the handshake. Not a defence
    /// against anyone on that network.
    Trusted,
    /// Any network: TLS 1.3 or QUIC, with a token or client certificates.
    Secure,
}

impl Trust {
    fn byte(self) -> u8 {
        match self {
            Trust::Local => 0,
            Trust::Trusted => 1,
            Trust::Secure => 2,
        }
    }

    fn from_byte(b: u8, at: usize) -> Result<Trust, ProtocolError> {
        Ok(match b {
            0 => Trust::Local,
            1 => Trust::Trusted,
            2 => Trust::Secure,
            _ => {
                return Err(ProtocolError::Field {
                    at,
                    why: "a trust level",
                });
            }
        })
    }

    /// Its name in a configuration: `local`, `trusted`, `secure`.
    pub fn name(self) -> &'static str {
        match self {
            Trust::Local => "local",
            Trust::Trusted => "trusted",
            Trust::Secure => "secure",
        }
    }
}

/// Why bytes are not a frame, with the offset into the body.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProtocolError {
    /// The body ends inside a field.
    Truncated {
        /// Where the missing byte would have been.
        at: usize,
    },
    /// A LEB128 longer than its shortest form, or wider than 64 bits.
    Overlong {
        /// Where it starts.
        at: usize,
    },
    /// Text that is not UTF-8.
    NotUtf8 {
        /// Where its length starts.
        at: usize,
    },
    /// A frame type this version does not define.
    UnknownFrame {
        /// The type byte.
        kind: u8,
    },
    /// A handshake that does not start with [`MAGIC`].
    NotThisProtocol,
    /// A peer speaking another version.
    Version {
        /// The version it speaks.
        theirs: u8,
    },
    /// A field holding something it cannot.
    Field {
        /// Where it starts.
        at: usize,
        /// What it should have held.
        why: &'static str,
    },
    /// A value whose wire bytes are malformed.
    Wire {
        /// Where its length starts.
        at: usize,
        /// The wire decoder's words.
        why: String,
    },
    /// A value that could not be encoded.
    Value(String),
    /// Bytes after the last field.
    Trailing {
        /// The first one.
        at: usize,
    },
    /// A body over the largest a frame carries.
    TooLarge {
        /// Its length.
        len: usize,
        /// The largest allowed.
        max: usize,
    },
}

impl core::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProtocolError::Truncated { at } => write!(f, "the frame ends inside a field at {at}"),
            ProtocolError::Overlong { at } => {
                write!(f, "an integer at {at} is not in its shortest form")
            }
            ProtocolError::NotUtf8 { at } => write!(f, "text at {at} is not UTF-8"),
            ProtocolError::UnknownFrame { kind } => {
                write!(f, "frame type {kind:#04x} is not one this version defines")
            }
            ProtocolError::NotThisProtocol => f.write_str("the peer does not speak guatiao-remote"),
            ProtocolError::Version { theirs } => {
                write!(f, "the peer speaks version {theirs}, this side {VERSION}")
            }
            ProtocolError::Field { at, why } => write!(f, "the field at {at} is not {why}"),
            ProtocolError::Wire { at, why } => write!(f, "the value at {at}: {why}"),
            ProtocolError::Value(why) => write!(f, "a value could not be encoded: {why}"),
            ProtocolError::Trailing { at } => write!(f, "bytes follow the last field, from {at}"),
            ProtocolError::TooLarge { len, max } => {
                write!(f, "a frame of {len} bytes is over the {max} allowed")
            }
        }
    }
}

impl core::error::Error for ProtocolError {}

/// Who a call is for, in the receiver's own numbering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// A provider's table, by the id the receiver's catalog gave it.
    Table(u64),
    /// An object the receiver holds and the sender was given.
    Object(u64),
}

impl Target {
    fn code(self) -> u64 {
        match self {
            Target::Table(id) => id << 1,
            Target::Object(id) => (id << 1) | 1,
        }
    }

    fn from_code(code: u64) -> Target {
        if code & 1 == 0 {
            Target::Table(code >> 1)
        } else {
            Target::Object(code >> 1)
        }
    }
}

/// One frame's body, decoded. A call's arguments and a reply's answer
/// stay bytes: they are read against the slot's description.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Frame<'a> {
    /// The client's first frame.
    Hello {
        /// The trust level the client means to use.
        trust: Trust,
        /// Its half of the handshake's nonce.
        nonce: [u8; 16],
        /// What it offers and asks (`max_frame`, `max_calls`, `compress`,
        /// `token`).
        options: Value,
    },
    /// The server's answer.
    Welcome {
        /// Its half of the nonce.
        nonce: [u8; 16],
        /// Its proof of the key (`trusted`), else empty.
        proof: &'a [u8],
        /// What it accepts.
        options: Value,
    },
    /// The client's proof of the key.
    Proof {
        /// HMAC-SHA256 over both nonces.
        proof: &'a [u8],
    },
    /// Asks the catalog.
    List {
        /// The asker's id for it.
        call: u64,
    },
    /// Opens a listed library.
    Open {
        /// The asker's id for it.
        call: u64,
        /// The library's id.
        library: &'a str,
        /// The stamp it was listed under.
        stamp: &'a str,
    },
    /// Lets an opened library go.
    Close {
        /// The library's id.
        library: &'a str,
        /// The stamp it was opened under.
        stamp: &'a str,
    },
    /// A call.
    Call {
        /// The caller's id for it.
        call: u64,
        /// The receiver's call this one is made during, or 0.
        parent: u64,
        /// What is called.
        target: Target,
        /// The slot's index in its kind.
        slot: u64,
        /// The arguments, in the slot's order.
        args: &'a [u8],
    },
    /// The answer to a call, a list or an open.
    Reply {
        /// The asker's id.
        call: u64,
        /// 0, or the status it failed with.
        status: u32,
        /// The answer (status 0) or the message.
        body: &'a [u8],
    },
    /// The caller no longer waits for a call.
    Cancel {
        /// The caller's id.
        call: u64,
    },
    /// An object the receiver gave is let go.
    Release {
        /// The receiver's id for it.
        object: u64,
    },
    /// Asks for a pong.
    Ping {
        /// Echoed back.
        n: u64,
    },
    /// The answer to a ping.
    Pong {
        /// The ping's.
        n: u64,
    },
    /// The link is ending.
    Goodbye {
        /// Why, as a status.
        status: u32,
        /// Why, in words.
        message: &'a str,
    },
}

/// Starts a frame in `out`: room for its length. Answers where it starts.
pub fn start(out: &mut Vec<u8>, kind: u8) -> usize {
    let at = out.len();
    out.extend_from_slice(&[0, 0, 0, 0, kind]);
    at
}

/// Ends the frame started at `at` with its length, refusing a body over
/// `max`.
pub fn finish(out: &mut [u8], at: usize, max: usize) -> Result<(), ProtocolError> {
    finish_with(out, at, 0, max)
}

/// [`finish`] for a frame whose body goes on for `more` bytes written
/// after `out` (a large buffer sent as its own slice).
pub fn finish_with(
    out: &mut [u8],
    at: usize,
    more: usize,
    max: usize,
) -> Result<(), ProtocolError> {
    let len = out.len() - at - 4 + more;
    if len > max || len > u32::MAX as usize {
        return Err(ProtocolError::TooLarge { len, max });
    }
    out[at..at + 4].copy_from_slice(&(len as u32).to_le_bytes());
    Ok(())
}

fn status_of(n: u64, at: usize) -> Result<u32, ProtocolError> {
    u32::try_from(n).map_err(|_| ProtocolError::Field {
        at,
        why: "a status",
    })
}

impl<'a> Frame<'a> {
    /// Reads one body. Values in it are decoded into `alloc`.
    pub fn decode(body: &'a [u8], alloc: Alloc) -> Result<Frame<'a>, ProtocolError> {
        let mut r = Reader::new(body);
        let kind = r.byte()?;
        let frame = match kind {
            kind::HELLO | kind::WELCOME => {
                if r.fixed::<4>()? != MAGIC {
                    return Err(ProtocolError::NotThisProtocol);
                }
                let version = r.byte()?;
                if version != VERSION {
                    return Err(ProtocolError::Version { theirs: version });
                }
                if kind == kind::HELLO {
                    let at = r.at();
                    let trust = Trust::from_byte(r.byte()?, at)?;
                    let nonce = r.fixed::<16>()?;
                    let options = r.value(alloc)?;
                    Frame::Hello {
                        trust,
                        nonce,
                        options,
                    }
                } else {
                    let nonce = r.fixed::<16>()?;
                    let proof = r.bytes()?;
                    let options = r.value(alloc)?;
                    Frame::Welcome {
                        nonce,
                        proof,
                        options,
                    }
                }
            }
            kind::PROOF => Frame::Proof { proof: r.bytes()? },
            kind::LIST => Frame::List { call: r.uint()? },
            kind::OPEN => Frame::Open {
                call: r.uint()?,
                library: r.text()?,
                stamp: r.text()?,
            },
            kind::CLOSE => Frame::Close {
                library: r.text()?,
                stamp: r.text()?,
            },
            kind::CALL => {
                let call = r.uint()?;
                let parent = r.uint()?;
                let target = Target::from_code(r.uint()?);
                let slot = r.uint()?;
                let args = r.rest();
                return Ok(Frame::Call {
                    call,
                    parent,
                    target,
                    slot,
                    args,
                });
            }
            kind::REPLY => {
                let call = r.uint()?;
                let at = r.at();
                let status = status_of(r.uint()?, at)?;
                return Ok(Frame::Reply {
                    call,
                    status,
                    body: r.rest(),
                });
            }
            kind::CANCEL => Frame::Cancel { call: r.uint()? },
            kind::RELEASE => Frame::Release { object: r.uint()? },
            kind::PING => Frame::Ping { n: r.uint()? },
            kind::PONG => Frame::Pong { n: r.uint()? },
            kind::GOODBYE => {
                let at = r.at();
                Frame::Goodbye {
                    status: status_of(r.uint()?, at)?,
                    message: r.text()?,
                }
            }
            other => return Err(ProtocolError::UnknownFrame { kind: other }),
        };
        r.end()?;
        Ok(frame)
    }

    /// Appends this frame, its length first, to `out`.
    pub fn encode(&self, out: &mut Vec<u8>, max: usize) -> Result<(), ProtocolError> {
        let at = match self {
            Frame::Hello {
                trust,
                nonce,
                options,
            } => {
                let at = start(out, kind::HELLO);
                out.extend_from_slice(&MAGIC);
                out.push(VERSION);
                out.push(trust.byte());
                out.extend_from_slice(nonce);
                put_value(out, options)?;
                at
            }
            Frame::Welcome {
                nonce,
                proof,
                options,
            } => {
                let at = start(out, kind::WELCOME);
                out.extend_from_slice(&MAGIC);
                out.push(VERSION);
                out.extend_from_slice(nonce);
                put_bytes(out, proof);
                put_value(out, options)?;
                at
            }
            Frame::Proof { proof } => {
                let at = start(out, kind::PROOF);
                put_bytes(out, proof);
                at
            }
            Frame::List { call } => {
                let at = start(out, kind::LIST);
                put_uint(out, *call);
                at
            }
            Frame::Open {
                call,
                library,
                stamp,
            } => {
                let at = start(out, kind::OPEN);
                put_uint(out, *call);
                put_text(out, library);
                put_text(out, stamp);
                at
            }
            Frame::Close { library, stamp } => {
                let at = start(out, kind::CLOSE);
                put_text(out, library);
                put_text(out, stamp);
                at
            }
            Frame::Call {
                call,
                parent,
                target,
                slot,
                args,
            } => {
                let at = start_call(out, *call, *parent, *target, *slot);
                out.extend_from_slice(args);
                at
            }
            Frame::Reply { call, status, body } => {
                let at = start_reply(out, *call, *status);
                out.extend_from_slice(body);
                at
            }
            Frame::Cancel { call } => {
                let at = start(out, kind::CANCEL);
                put_uint(out, *call);
                at
            }
            Frame::Release { object } => {
                let at = start(out, kind::RELEASE);
                put_uint(out, *object);
                at
            }
            Frame::Ping { n } => {
                let at = start(out, kind::PING);
                put_uint(out, *n);
                at
            }
            Frame::Pong { n } => {
                let at = start(out, kind::PONG);
                put_uint(out, *n);
                at
            }
            Frame::Goodbye { status, message } => {
                let at = start(out, kind::GOODBYE);
                put_uint(out, u64::from(*status));
                put_text(out, message);
                at
            }
        };
        finish(out, at, max)
    }
}

/// Starts a CALL frame: everything up to its arguments, which the caller
/// appends before [`finish`].
pub fn start_call(out: &mut Vec<u8>, call: u64, parent: u64, target: Target, slot: u64) -> usize {
    let at = start(out, kind::CALL);
    put_uint(out, call);
    put_uint(out, parent);
    put_uint(out, target.code());
    put_uint(out, slot);
    at
}

/// Starts a REPLY frame: everything up to its body.
pub fn start_reply(out: &mut Vec<u8>, call: u64, status: u32) -> usize {
    let at = start(out, kind::REPLY);
    put_uint(out, call);
    put_uint(out, u64::from(status));
    at
}

/// A REPLY refusing `call` with `status` and `message`.
pub fn refusal(out: &mut Vec<u8>, call: u64, status: u32, message: &str) {
    let at = start_reply(out, call, status);
    put_text(out, message);
    finish(out, at, usize::MAX).expect("a refusal fits");
}

/// The words of a refusing REPLY's body.
pub fn refusal_words(body: &[u8]) -> Result<&str, ProtocolError> {
    let mut r = Reader::new(body);
    let words = r.text()?;
    r.end()?;
    Ok(words)
}

/// The catalog a LIST is answered with, read from a REPLY's body.
pub fn catalog_of(body: &[u8], alloc: Alloc) -> Result<Value, ProtocolError> {
    let mut r = Reader::new(body);
    let value = r.value(alloc)?;
    r.end()?;
    Ok(value)
}

/// Appends a catalog as a REPLY's body.
pub fn put_catalog(out: &mut Vec<u8>, catalog: &Value) -> Result<(), ProtocolError> {
    put_value(out, catalog)
}

#[cfg(test)]
mod tests {
    use super::*;
    use guatiao::Map;

    fn round_trip(frame: Frame<'_>) {
        let mut out = Vec::new();
        frame.encode(&mut out, usize::MAX).expect("encodes");
        let len = u32::from_le_bytes(out[..4].try_into().unwrap()) as usize;
        assert_eq!(len, out.len() - 4);
        let back = Frame::decode(&out[4..], Alloc::rust()).expect("decodes");
        assert_eq!(back, frame);
    }

    #[test]
    fn every_frame_round_trips() {
        let mut options = Map::new();
        options.set("max_calls", 64i64);
        round_trip(Frame::Hello {
            trust: Trust::Trusted,
            nonce: [7; 16],
            options: Value::from(options.clone()),
        });
        round_trip(Frame::Welcome {
            nonce: [9; 16],
            proof: &[1, 2, 3],
            options: Value::from(options),
        });
        round_trip(Frame::Proof { proof: &[4; 32] });
        round_trip(Frame::List { call: 1 });
        round_trip(Frame::Open {
            call: 2,
            library: "derived_greeter",
            stamp: "s1",
        });
        round_trip(Frame::Close {
            library: "derived_greeter",
            stamp: "s1",
        });
        round_trip(Frame::Call {
            call: 300,
            parent: 0,
            target: Target::Table(3),
            slot: 0,
            args: &[5, b'w', b'o', b'r', b'l', b'd'],
        });
        round_trip(Frame::Call {
            call: 4,
            parent: 9,
            target: Target::Object(2),
            slot: 1,
            args: &[],
        });
        round_trip(Frame::Reply {
            call: 4,
            status: 12,
            body: &[3, b'b', b'y', b'e'],
        });
        round_trip(Frame::Cancel { call: 4 });
        round_trip(Frame::Release { object: 2 });
        round_trip(Frame::Ping { n: 1 });
        round_trip(Frame::Pong { n: 1 });
        round_trip(Frame::Goodbye {
            status: 7,
            message: "wrong key",
        });
    }

    #[test]
    fn a_stranger_or_another_version_is_refused() {
        let mut out = Vec::new();
        Frame::Hello {
            trust: Trust::Local,
            nonce: [0; 16],
            options: Value::from(Map::new()),
        }
        .encode(&mut out, usize::MAX)
        .unwrap();
        let mut stranger = out[4..].to_vec();
        stranger[1] = b'X';
        assert_eq!(
            Frame::decode(&stranger, Alloc::rust()),
            Err(ProtocolError::NotThisProtocol)
        );
        let mut later = out[4..].to_vec();
        later[5] = 2;
        assert_eq!(
            Frame::decode(&later, Alloc::rust()),
            Err(ProtocolError::Version { theirs: 2 })
        );
        assert_eq!(
            Frame::decode(&[0x7f], Alloc::rust()),
            Err(ProtocolError::UnknownFrame { kind: 0x7f })
        );
        assert_eq!(
            Frame::decode(&[kind::LIST, 1, 0], Alloc::rust()),
            Err(ProtocolError::Trailing { at: 2 })
        );
    }

    #[test]
    fn a_body_over_the_limit_is_refused_when_it_is_finished() {
        let mut out = Vec::new();
        let at = start(&mut out, kind::PING);
        out.extend_from_slice(&[0; 20]);
        assert_eq!(
            finish(&mut out, at, 8),
            Err(ProtocolError::TooLarge { len: 21, max: 8 })
        );
    }
}
