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
  kind; a caller wanting a bound keeps its own. A host never waits inside
  `receive` for its own `send` to return: a provider whose output depends
  on its input, like the echo, would deadlock on a full queue.
- **Order** is kept per direction, in call order. The provider makes
  every call on the sink, `receive` and `closed` alike, from one thread
  at a time (possibly one of its own), and none while another runs.
- **A status is a `guatiao_status` value.** In `close(status)` and
  `closed(status)`, `GUATIAO_OK` (0) is the normal end and anything else
  an error status.
- **The host closes once.** After `close(status)` (or dropping the
  channel, which is `close(GUATIAO_OK)`), `send` answers
  `GUATIAO_ERR_GONE`, a `send` waiting for room included, and none is
  taken once `close` has returned. The provider calls `closed(status)`
  exactly once, unless it already had, and never calls the sink again.
  `close` and dropping the channel return without waiting for a `receive`
  still running.
- **The provider closes once.** After `closed(status)`, `send` answers
  `GUATIAO_ERR_GONE`.
- **A sink that has gone** answers `GUATIAO_ERR_GONE` from `receive`; any
  error from `receive` ends the stream, and the provider calls `closed`
  with that status and stops: a message still queued is not delivered.
- **Unloading.** A provider's library answers `GUATIAO_ERR_BUSY` to an
  unload while a thread of its own is still inside a call on the sink,
  through `providers!`'s `unload =` hook, and joins its threads once none
  is.
- **`close` and `closed` cannot fail.** A C provider's `close` and a C
  host's `closed` answer `GUATIAO_OK`; any other answer panics the
  caller's proxy.

`examples/channel_echo` is a provider keeping every rule and the suite
that checks them (`tests/rules/`), with planted faults showing each test
fails when its rule breaks. A provider written in a language whose objects
are called on one interpreter thread answers inside the `send` that took
the message: it keeps the rules that do not need a thread of its own
(`NEEDS_A_THREAD` in the suite names the others).

## Threads

Whether a provider's own thread may call a host's sink, by language of
the host:

| host | from the provider's own thread |
| --- | --- |
| Rust, C | yes |
