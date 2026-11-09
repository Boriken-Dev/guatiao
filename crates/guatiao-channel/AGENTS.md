# guatiao-channel — shipped API header

Two object kinds, `channel` and `channel_sink`, for a two-way stream of
bytes between a host and a provider. `#![forbid(unsafe_code)]`; depends on
`guatiao` (`provider`) and nothing else. Feature `c-header` (off by
default) renders `include/guatiao_channel.h`.

```rust
#[guatiao::kind(object)]
pub trait Channel: Send + Sync {          // the provider implements, the host holds
    fn send(&self, bytes: &[u8]) -> Result<(), ProviderError>;
    fn close(&self, status: u32, reason: &str);
}
#[guatiao::kind(object)]
pub trait ChannelSink: Send {             // the host implements, the provider owns
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError>;
    fn closed(&mut self, status: u32, reason: &str);
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
- **A status is a `guatiao_status` value.** In `close(status, reason)`
  and `closed(status, reason)`, `GUATIAO_OK` (0) is the normal end and
  anything else an error status.
- **A reason is text for people.** UTF-8 (`guatiao_str` in C), lent for
  the call, empty when there is none: the host's reason for giving up, the
  provider's failure. For logs and messages only; no one parses it. A
  sink that has gone (`GONE`) or a dropped channel says the same as `OK`
  and an empty reason.
- **The host closes once.** After `close(status, reason)` (or dropping the
  channel, which is `close(GUATIAO_OK, "")`), `send` answers
  `GUATIAO_ERR_GONE`, a `send` waiting for room included, and none is
  taken once `close` has returned. The provider calls
  `closed(status, reason)` exactly once, unless it already had, and never
  calls the sink again; it may pass the host's reason on or give its own.
  `close` and dropping the channel return without waiting for a `receive`
  still running.
- **The provider closes once.** After `closed(status, reason)`, `send` answers
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

Whether a provider's own thread may call a host's sink, by the language
the host's sink is written in (measured with the Rust echo, each in its
own test):

| host | from the provider's own thread |
| --- | --- |
| Rust, C | yes |
| Python | yes: the GIL lets any thread call a Python object |
| Node | yes, while the event loop is free: koffi queues the call to the loop and the provider's thread waits for the answer, so a host blocked inside a synchronous `send` hears nothing until it returns (send a few messages, then `await`) |
| Lua | no: a Lua host's table is called on its interpreter's thread only, and a call from a provider's own thread is refused `GUATIAO_ERR_INTERNAL` |
| Dart | not measured: a call into a Dart-served table from a thread Dart did not start ends the process |

And by the language a provider is written in, whether it can push from a
thread of its own:

| provider | pushes from its own thread |
| --- | --- |
| Rust | yes (the echo) |
| Python | yes (`threading.Thread`; the echo in `examples/channel_echo/python`) |
| QuickJS, Node | no: objects are called on the engine's thread only, so the echo answers inside the `send` that took the message |
| Lua | no: the same; the echo answers inside `send` |

A provider with no thread of its own keeps every rule except the three
listed in `NEEDS_A_THREAD` (queued messages left unsent when the sink says
GONE, `close` while a `receive` is blocked, and a `send` woken by `close`),
which need a second thread to be observable.
