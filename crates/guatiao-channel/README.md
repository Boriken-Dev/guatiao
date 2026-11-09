# guatiao-channel

[![Crate](https://img.shields.io/crates/v/guatiao-channel.svg)](https://crates.io/crates/guatiao-channel)
[![docs.rs](https://img.shields.io/docsrs/guatiao-channel)](https://docs.rs/guatiao-channel)
[![License: MPL 2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/guatiao/blob/main/LICENSE)

A two-way stream of bytes between a host and a provider, as two
[guatiao](https://crates.io/crates/guatiao) object kinds with stated rules.

> **Status: alpha.** Released in lockstep with `guatiao`.

- **`channel`**, what the host writes on: `send(bytes)` and `close(status, reason)`.
- **`channel_sink`**, what the provider writes back on, handed in by the
  host: `receive(bytes)` and `closed(status, reason)`.

A provider kind of one's own starts a channel,
`start(config, sink) -> channel`, and the host and the provider can be
written in any language guatiao reaches. Messages are bytes, lent for the
call. Back-pressure blocks, order is kept per direction, and either side
closes once; the rules are in the crate documentation and in `AGENTS.md`.

`include/guatiao_channel.h` declares both tables for C and
`include/guatiao_channel.kinds.json` describes them as data, for the
generators of every language.

Licensed under the MPL 2.0.
