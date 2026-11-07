# guatiao-channel — shipped API header

Two object kinds, `channel` and `channel_sink`, for a two-way stream of
bytes between a host and a provider. `#![forbid(unsafe_code)]`; depends on
`guatiao` (`provider`) and nothing else. Feature `c-header` (off by
default) renders `include/guatiao_channel.h`.

```rust
#[guatiao::kind(object)]
pub trait Channel: Send + Sync {          // the provider implements, the host holds
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError>;
    fn close(&self, status: u32);
}
#[guatiao::kind(object)]
pub trait ChannelSink: Send {             // the host implements, the provider owns
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError>;
    fn closed(&mut self, status: u32);
}
```

A kind of your own starts a channel: `start(config, sink:
Object<dyn ChannelSink>) -> Result<Object<dyn Channel>, ProviderError>`.
The sink is handed over; the provider destroys it, even when `start`
refuses.

## Rules

- **Back-pressure blocks.** `send` and `receive` return once the other
  side has taken the message. No `GUATIAO_ERR_BUSY`, no timeout in the
  kind; a caller wanting a bound keeps its own.
- **Order** is kept per direction, in call order. The provider calls
  `receive` from one thread at a time, possibly one of its own.
- **The host closes once.** After `close(status)` (or dropping the
  channel, which is `close(0)`), `send` answers `GUATIAO_ERR_GONE`; the
  provider calls `closed(status)` exactly once, unless it already had,
  and never calls the sink again.
- **The provider closes once.** After `closed(status)`, `send` answers
  `GUATIAO_ERR_GONE`.
- **A sink that has gone** answers `GUATIAO_ERR_GONE` from `receive`; any
  error from `receive` ends the stream, and the provider calls `closed`
  with that status and stops.

`examples/channel_echo` is a provider keeping every rule and the suite
that checks them (`tests/rules/`), with planted faults showing each test
fails when its rule breaks.

## Threads

Whether a provider's own thread may call a host's sink, by language of
the host:

| host | from the provider's own thread |
| --- | --- |
| Rust, C | yes |
