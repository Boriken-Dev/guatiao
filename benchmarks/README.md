# guatiao-bench

Times the paths a host takes through a loaded library, over the example
libraries the test suite loads (`hello_library`, `derived_greeter`). Run
by hand when a change touches the loader, the registry or a kind's glue;
never on every push.

**Every number in `results/` is local and provisional.** They come from
one developer machine, whose timings vary two- to fourfold from run to
run. A performance claim in a release or the changelog needs a CI run.

## Running it

```bash
cargo run -p guatiao-bench --release -- --save NAME
```

writes `benchmarks/results/NAME.json`. `--rounds N` (default 6) runs every
metric once per round, each round starting one metric later;
`--samples N` (default 15) is how many timed samples a metric takes per
round, and `--target-ms N` (default 5) how long one sample runs. `--only
a,b` restricts the run. The libraries are found beside the runner, so
build the workspace in the same profile first.

To compare two builds, run their two binaries alternately, A B A B, each
saving to its own file, and pool each side's runs:

```bash
guatiao-bench --merge a-1.json a-2.json ... --save NAME
```

The files in `results/` were made that way: five alternating runs per
side, pooled.

## The result file

One file per (version, toolchain):

```json
{
  "name": "0.0.0-alpha.1",
  "metrics": {
    "call_map": {"min_ms": 1.38e-4, "median_ms": 1.42e-4, "max_ms": 2.67e-4}
  },
  "raw": {
    "processor": "aarch64",
    "os": "windows",
    "runs": 5,
    "rounds": 6,
    "samples_per_round": 15,
    "round_medians_ms": {"call_map": [1.42e-4]},
    "samples_ms": {"call_map": [1.40e-4]}
  }
}
```

Each metric is milliseconds per call: min, median and max over every
sample. Compare on the median; `raw` keeps each round's median and every
sample, which is what `--merge` pools.

## The metrics

| metric | what one call is |
| --- | --- |
| `load_file_hello`, `load_file_derived` | `Registry::load_file` of one library into a fresh registry |
| `scan_dir` | `scan_dir` over a directory holding a copy of both libraries |
| `offers` | `registry.offers::<dyn Greeter>()` collected (two offers) |
| `call_scalar` | `Counter::count` through a provider's table |
| `call_map` | `Greeter::greet`, answering a `Map` |
| `call_string` | `Greeter::shout`, answering a `String` |
| `instantiate_drop` | `Offer::instantiate` from a configuration, then drop |
| `object_ref` | `Conversation::turns`, a `&self` object method |
| `object_mut` | `Conversation::read`, a `&mut self` object method |
| `lent_call` | `Greeter::greet_into`, lending a listener for the call |
| `remote_clone` | cloning a `Remote` and dropping the clone |
| `retire` | `Registry::retire` of a library loaded just before |
| `host_get` | `hello_library`'s echo: a library listing the host's greeters through its services and calling one |

## Results

Baselines, Windows ARM64, local and provisional:

- `0.0.0-alpha.1-pre-checks`: before descriptors were checked where they
  enter.
- `0.0.0-alpha.1-pre-lease`: before handles held a lease on their
  library.
