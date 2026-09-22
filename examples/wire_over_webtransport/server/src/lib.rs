// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! What both ends agree on: the channel, its schema, and how a frame is
//! delimited on a stream — a little-endian `u32` length, then the frame.

use guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
use guatiao::{Map, Value};

/// The one channel this example uses.
pub const CHANNEL: u64 = 1;

/// What every value on [`CHANNEL`] must be: a greeting and a count.
pub fn schema() -> Value {
    SchemaBuilder::new()
        .field(FieldBuilder::new("greeting", KindBuilder::string()).required())
        .field(FieldBuilder::new("count", KindBuilder::int()).required())
        .finish()
        .expect("a fixed schema builds")
}

/// The `n`th value the server sends. Every tenth leaves out the greeting,
/// so the receiver's refusal is part of the demonstration.
pub fn nth(n: i64) -> Value {
    let mut map = Map::new();
    if n % 10 != 0 {
        map.set("greeting", "hello").expect("a small map grows");
    }
    map.set("count", n).expect("a small map grows");
    map.into()
}

/// A frame as it goes on a stream: its length, then its bytes.
pub fn delimited(frame: &[u8]) -> Vec<u8> {
    let len = u32::try_from(frame.len()).expect("a frame under 4 GiB");
    let mut out = Vec::with_capacity(4 + frame.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(frame);
    out
}

/// Splits whole frames off the front of `buffer`, leaving a partial one.
pub fn frames(buffer: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while buffer.len() >= 4 {
        let len = u32::from_le_bytes(buffer[..4].try_into().expect("four bytes")) as usize;
        if buffer.len() < 4 + len {
            break;
        }
        out.push(buffer[4..4 + len].to_vec());
        buffer.drain(..4 + len);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use guatiao::value::wire::channel::{self, Received, Receiver};

    #[test]
    fn frames_split_where_their_lengths_say_and_the_receiver_checks_them() {
        let mut stream = delimited(&channel::announce(CHANNEL, &schema()).unwrap());
        for n in 9..=11 {
            stream.extend(delimited(&channel::send(CHANNEL, &nth(n)).unwrap()));
        }
        // Arrives split mid-frame, as a network delivers it.
        let (first, rest) = stream.split_at(7);
        let mut buffer = first.to_vec();
        assert!(frames(&mut buffer).is_empty(), "no whole frame yet");
        buffer.extend_from_slice(rest);
        let got = frames(&mut buffer);
        assert!(buffer.is_empty());
        assert_eq!(got.len(), 4);

        let mut receiver = Receiver::new();
        assert!(matches!(
            receiver.accept(&got[0]),
            Ok(Received::Schema(CHANNEL))
        ));
        assert!(matches!(
            receiver.accept(&got[1]),
            Ok(Received::Value { .. })
        ));
        assert!(
            receiver.accept(&got[2]).is_err(),
            "the tenth has no greeting"
        );
        assert!(matches!(
            receiver.accept(&got[3]),
            Ok(Received::Value { .. })
        ));
    }
}
