// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The browser's end: opens WebTransport to the server with its
//! certificate pinned by hash, reads the one stream the server opens,
//! splits it into frames and hands each to a `Receiver`, which checks
//! every value against the schema its channel announced. Each outcome is
//! passed to `log` as a line of text.

use guatiao::value::read::Dump;
use guatiao::value::wire::channel::{Received, Receiver};
use js_sys::{Function, Reflect, Uint8Array};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    ReadableStream, ReadableStreamDefaultReader, WebTransport, WebTransportHash,
    WebTransportOptions,
};
use wire_over_webtransport::frames;

/// Connects to `url`, trusting the certificate whose SHA-256 is `hash`,
/// and calls `log` with a line for every frame until the stream ends.
#[wasm_bindgen]
pub async fn run(url: String, hash: Vec<u8>, log: Function) -> Result<(), JsValue> {
    let certificate = WebTransportHash::new();
    certificate.set_algorithm("sha-256");
    certificate.set_value_u8_array(&Uint8Array::from(hash.as_slice()));
    let options = WebTransportOptions::new();
    options.set_server_certificate_hashes(&[certificate]);

    let transport = WebTransport::new_with_options(&url, &options)?;
    JsFuture::from(transport.ready().unchecked_into::<js_sys::Promise>()).await?;

    // The server opens one stream; read the first, then its bytes.
    let incoming = reader(&transport.incoming_unidirectional_streams())?;
    let first = JsFuture::from(incoming.read()).await?;
    let stream: ReadableStream = Reflect::get(&first, &"value".into())?.dyn_into()?;
    let bytes = reader(&stream)?;

    let mut receiver = Receiver::new();
    let mut buffer = Vec::new();
    loop {
        let chunk = JsFuture::from(bytes.read()).await?;
        if Reflect::get(&chunk, &"done".into())?.as_bool() == Some(true) {
            return Ok(());
        }
        let data: Uint8Array = Reflect::get(&chunk, &"value".into())?.dyn_into()?;
        buffer.extend(data.to_vec());
        for frame in frames(&mut buffer) {
            let line = match receiver.accept(&frame) {
                Ok(Received::Schema(channel)) => format!("schema on channel {channel}"),
                Ok(Received::Value { channel, value }) => {
                    format!("value on channel {channel}: {:?}", Dump(&value))
                }
                Ok(Received::Closed(channel)) => format!("channel {channel} closed"),
                Err(e) => format!("refused: {e}"),
            };
            log.call1(&JsValue::NULL, &line.into())?;
        }
    }
}

fn reader(stream: &ReadableStream) -> Result<ReadableStreamDefaultReader, JsValue> {
    stream.get_reader().dyn_into().map_err(JsValue::from)
}
