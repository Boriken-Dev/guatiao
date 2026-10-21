# A Rust caller of any greeter

A cdylib with one C function, `greeter_caller_run`, for a host written in
another language to have its greeters called from Rust:

```c
guatiao_status greeter_caller_run(const guatiao_host_info *host,
                                  guatiao_str key, guatiao_str name,
                                  guatiao_value *out);
```

Given a registry's host block (`guatiao_registry_host`) and a provider's
key, it finds the greeter through the host's services and calls every slot
of the `greeter` kind through the kind's Rust proxy, as a Rust host does:
`greet` (with a name, an empty one, and `"panic"`), `shout`, `check` on a
map it lends, `greet_from` with a map it hands over, `greet_into` with a
listener it lends, and `start` with a listener it hands over, saying twice
and reading the transcript. It answers a map of what each gave; the crate
documentation lists its keys.

The Node binding's tests register greeters written in Rust, C, Python and
TypeScript in one registry and call this for each. Build it with the
workspace:

```bash
cargo build --workspace --all-features   # target/debug/greeter_caller.dll, libgreeter_caller.so, .dylib
```
