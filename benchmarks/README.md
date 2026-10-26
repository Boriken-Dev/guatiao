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
side, pooled; see Results for which session each came from.

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
| `call_map` | `Greeter::greet`, answering a `Map` the provider builds in the caller's allocator (`greet_in`) |
| `call_map_copied` | the same through a configured instance whose `greet` builds in its own allocator, so the map is copied on arrival |
| `call_map_arg` | `Greeter::greet_from`, handing over a one-entry `Map` built for the call (by value: a move), answered as `call_map` is |
| `call_map_mut` | `Greeter::check`, lending a one-entry `Map` built for the call (`&mut`), to which the provider adds an entry built in the map's own allocator |
| `call_map_mut_moved` | the same through a configured instance that builds the entry's value in its own allocator, which the proxy moves into the map's after the call |
| `call_string` | `Greeter::shout`, answering a `String` |
| `instantiate_drop` | `Offer::instantiate` from a configuration, then drop |
| `object_ref` | `Conversation::turns`, a `&self` object method |
| `object_mut` | `Conversation::read`, a `&mut self` object method |
| `lent_call` | `Greeter::greet_into`, lending a listener for the call |
| `remote_clone` | cloning a `Remote` and dropping the clone |
| `retire` | `Registry::retire` of a library loaded just before |
| `host_get` | `hello_library`'s echo: a library listing the host's greeters through its services and calling one |
| `clone_map` | `clone()` of an eight-entry map (text, numbers, a nested map, a list), and dropping the copy |
| `into_local_local` | `into_local()` of that map when it is already local: judged and handed back |
| `into_local_foreign` | that map moved between this binary's allocator and another, alternately: one copy and one free per call, what `into_local()` costs a tree from elsewhere |
| `config_wire_tree`, `config_wire_walk` | a derived configuration (twenty servers, about 1.3 KB) to wire bytes: `to_value` then `wire::to_bytes`, against `wire::to_bytes_of` |
| `wire_config_tree`, `wire_config_walk` | those bytes back: `wire::decode` then `from_value`, against `wire::decode_as` |
| `floats_wire_*`, `wire_floats_*` | the same for ten thousand `f64`s with fractions |
| `bytes_wire_*`, `wire_bytes_*` | the same for a 64 KiB byte string |
| `config_json_tree`, `config_json_walk` | the configuration to JSON: `to_value` then `text::json::to_string`, against `to_string_of` |
| `json_config_tree`, `json_config_walk` | that text back: `json::from_str` then `from_value`, against `json::from_str_as` |
| `floats_json_*`, `json_floats_*` | the same for the ten thousand floats |

## Results

Local and provisional: one Windows ARM64 machine, and a Linux (aarch64)
environment on the same machine. Microseconds per call, median.

- `pre-checks`: before descriptors were checked where they enter
  (`a443b8f`, the runner copied in).
- `pre-lease`: before handles held a lease on their library (`cfec53a`,
  the runner copied in).
- `pre-zero-copy`: before a call's answer was built in the caller's
  allocator and a loaded image was placed from the loader's own record
  (`52fcdb2`).
- `0.0.0-alpha.1`: all of them.

Every file in this table comes from one session per platform: the four
builds run alternately, five runs each, pooled. `call_map_copied` did not
exist in the runner before `0.0.0-alpha.1`; the older builds were measured
with this runner, so they have it too.

### Windows (`0.0.0-alpha.1-pre-checks.json`, `-pre-lease.json`, `-pre-zero-copy.json`, `0.0.0-alpha.1.json`)

| metric | pre-checks | pre-lease | pre-zero-copy | 0.0.0-alpha.1 | vs pre-zero-copy | vs pre-lease | vs pre-checks |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `call_map` | 0.1385 | 0.1483 | 0.2908 | 0.1558 | -46.4% | +5.0% | +12.4% |
| `call_map_copied` | 0.1656 | 0.1705 | 0.3233 | 0.3281 | +1.5% | +92.5% | +98.2% |
| `call_scalar` | 0.00216 | 0.002135 | 0.002127 | 0.002123 | -0.2% | -0.6% | -1.7% |
| `call_string` | 0.201 | 0.2081 | 0.2152 | 0.2146 | -0.3% | +3.1% | +6.8% |
| `host_get` | 2.545 | 2.567 | 2.714 | 2.722 | +0.3% | +6.0% | +7.0% |
| `instantiate_drop` | 0.5789 | 0.5804 | 0.5833 | 0.5782 | -0.9% | -0.4% | -0.1% |
| `lent_call` | 0.1379 | 0.1473 | 0.1463 | 0.1515 | +3.6% | +2.8% | +9.9% |
| `load_file_derived` | 56.34 | 79.17 | 81.94 | 55.99 | -31.7% | -29.3% | -0.6% |
| `load_file_hello` | 55.84 | 80.05 | 83.37 | 56.84 | -31.8% | -29.0% | +1.8% |
| `object_mut` | 0.003496 | 0.003744 | 0.003496 | 0.003496 | +0.0% | -6.6% | +0.0% |
| `object_ref` | 0.001621 | 0.001612 | 0.001615 | 0.001614 | -0.0% | +0.1% | -0.4% |
| `offers` | 1.265 | 1.239 | 1.297 | 1.285 | -0.9% | +3.7% | +1.6% |
| `remote_clone` | 0.0007479 | 0.0007474 | 0.003602 | 0.003603 | +0.0% | +382.0% | +381.7% |
| `retire` | 1.633 | 1.581 | 1.7 | 1.617 | -4.9% | +2.3% | -1.0% |
| `scan_dir` | 222.7 | 267.9 | 274 | 224.6 | -18.0% | -16.2% | +0.9% |

### Linux (`0.0.0-alpha.1-linux-pre-checks.json`, `-linux-pre-lease.json`, `-linux-pre-zero-copy.json`, `-linux.json`)

| metric | pre-checks | pre-lease | pre-zero-copy | 0.0.0-alpha.1 | vs pre-zero-copy | vs pre-lease | vs pre-checks |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `call_map` | 0.07137 | 0.07085 | 0.1606 | 0.08514 | -47.0% | +20.2% | +19.3% |
| `call_map_copied` | 0.08551 | 0.0844 | 0.1722 | 0.17 | -1.2% | +101.5% | +98.8% |
| `call_scalar` | 0.004578 | 0.004576 | 0.004575 | 0.004581 | +0.1% | +0.1% | +0.1% |
| `call_string` | 0.1044 | 0.1047 | 0.1182 | 0.1191 | +0.8% | +13.8% | +14.1% |
| `host_get` | 1.899 | 1.932 | 2.239 | 2.095 | -6.4% | +8.4% | +10.4% |
| `instantiate_drop` | 0.5522 | 0.5514 | 0.5576 | 0.5586 | +0.2% | +1.3% | +1.2% |
| `lent_call` | 0.07587 | 0.07918 | 0.07879 | 0.08325 | +5.7% | +5.1% | +9.7% |
| `load_file_derived` | 5.351 | 21.74 | 23.17 | 6.34 | -72.6% | -70.8% | +18.5% |
| `load_file_hello` | 6.069 | 27.62 | 29.9 | 7.371 | -75.3% | -73.3% | +21.5% |
| `object_mut` | 0.002427 | 0.002757 | 0.002533 | 0.005495 | +116.9% | +99.3% | +126.4% |
| `object_ref` | 0.00162 | 0.001621 | 0.001623 | 0.001626 | +0.2% | +0.3% | +0.4% |
| `offers` | 0.9106 | 0.8891 | 1.028 | 0.9341 | -9.1% | +5.1% | +2.6% |
| `remote_clone` | 0.0007536 | 0.0007527 | 0.00914 | 0.00916 | +0.2% | +1116.8% | +1115.5% |
| `retire` | 1.036 | 1.09 | 1.237 | 1.084 | -12.3% | -0.5% | +4.7% |
| `scan_dir` | 95.69 | 130.9 | 135.4 | 97.18 | -28.2% | -25.8% | +1.6% |

### Built with LTO and one codegen unit (`*-lto*.json`)

Linux metrics under a microsecond move 10-50% between builds with no
change on their path, because the code is laid out differently. Built
with `CARGO_PROFILE_RELEASE_LTO=true CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1`,
the same code is laid out alike; these runs alternate three such builds,
five runs each, over the metrics in question.

Windows:

| metric | pre-checks | pre-zero-copy | final | vs pre-zero-copy | vs pre-checks |
| --- | --- | --- | --- | --- | --- |
| `call_map` | 0.1184 | 0.244 | 0.1278 | -47.6% | +8.0% |
| `call_map_copied` | 0.1412 | 0.2704 | 0.2687 | -0.6% | +90.3% |
| `call_string` | 0.1771 | 0.1787 | 0.1789 | +0.1% | +1.0% |
| `host_get` | 2.142 | 2.295 | 2.304 | +0.4% | +7.6% |
| `lent_call` | 0.1196 | 0.1246 | 0.1206 | -3.3% | +0.8% |
| `load_file_hello` | 54.19 | 80.36 | 56.92 | -29.2% | +5.0% |

Linux:

| metric | pre-checks | pre-zero-copy | final | vs pre-zero-copy | vs pre-checks |
| --- | --- | --- | --- | --- | --- |
| `call_map` | 0.08274 | 0.107 | 0.06935 | -35.2% | -16.2% |
| `call_map_copied` | 0.08241 | 0.1155 | 0.1778 | +53.9% | +115.8% |
| `call_string` | 0.1342 | 0.07955 | 0.1294 | +62.7% | -3.6% |
| `host_get` | 1.581 | 1.944 | 1.777 | -8.6% | +12.3% |
| `lent_call` | 0.08582 | 0.06185 | 0.08614 | +39.3% | +0.4% |
| `load_file_derived` | 5.043 | 20.47 | 5.952 | -70.9% | +18.0% |
| `load_file_hello` | 5.67 | 28.03 | 6.797 | -75.8% | +19.9% |
| `object_mut` | 0.00196 | 0.001962 | 0.001956 | -0.3% | -0.2% |

### Owning without copying (`ownership-before.json`, `ownership.json`)

Windows, one session: the `0.0.0-alpha.1` build (the runner copied in,
without the metrics that need the new API) and the build where `clone()`
copies locally, `into_local` exists and a tree crosses a kind call by
value, alternating, five runs each, pooled. Microseconds per call, median.

| metric | 0.0.0-alpha.1 | this change | change |
| --- | --- | --- | --- |
| `call_map` | 0.1565 | 0.1548 | -1.1% |
| `call_map_arg` | — | 0.2628 | new |
| `call_map_copied` | 0.3277 | 0.3247 | -0.9% |
| `call_scalar` | 0.002116 | 0.002121 | +0.2% |
| `call_string` | 0.2147 | 0.2156 | +0.4% |
| `clone_map` | 0.9638 | 0.9185 | -4.7% |
| `host_get` | 2.745 | 2.63 | -4.2% |
| `instantiate_drop` | 0.5846 | 0.5826 | -0.3% |
| `into_local_foreign` | — | 0.9455 | new |
| `into_local_local` | — | 0.06915 | new |
| `lent_call` | 0.1492 | 0.1471 | -1.4% |
| `load_file_derived` | 64.91 | 64.95 | +0.1% |
| `load_file_hello` | 66.01 | 66.24 | +0.3% |
| `object_mut` | 0.003487 | 0.003493 | +0.2% |
| `object_ref` | 0.001605 | 0.001606 | +0.1% |
| `offers` | 1.277 | 1.252 | -1.9% |
| `remote_clone` | 0.003578 | 0.003585 | +0.2% |
| `retire` | 1.601 | 1.631 | +1.9% |
| `scan_dir` | 213 | 211.8 | -0.6% |

No metric is more than 2% slower. Against the committed
`0.0.0-alpha.1.json`, `load_file_*` read +16%: the same `0.0.0-alpha.1`
build measured 65 µs in this session against 56 µs in that one, so the
difference is the session, not the code.

Owning a tree that is already local costs a judgment and no copy:
`into_local_local` is 0.069 µs for the eight-entry map whose `clone()`
costs 0.92 µs, and `into_local_foreign`, a copy and a free, costs about
what `clone()` does. `call_map_arg` is `call_map` plus building the
one-entry map it hands over; the map itself crosses by pointer.

### Lending a tree to be changed (`mutable-before.json`, `mutable.json`)

Windows, one session: the build before a kind method could take `&mut
Map` and the build after, alternating, five runs each, pooled.
Microseconds per call, median.

| metric | before | this change | change |
| --- | --- | --- | --- |
| `call_map` | 0.1559 | 0.16 | +2.7% |
| `call_map_arg` | 0.2677 | 0.2839 | +6.0% |
| `call_map_copied` | 0.3323 | 0.3336 | +0.4% |
| `call_map_mut` | — | 0.2581 | new |
| `call_map_mut_moved` | — | 0.2514 | new |
| `call_scalar` | 0.002129 | 0.00212 | -0.4% |
| `call_string` | 0.2204 | 0.2192 | -0.6% |
| `clone_map` | 0.9453 | 0.9579 | +1.3% |
| `host_get` | 2.764 | 2.718 | -1.7% |
| `instantiate_drop` | 0.5843 | 0.5852 | +0.2% |
| `into_local_foreign` | 0.9693 | 0.9825 | +1.4% |
| `into_local_local` | 0.0696 | 0.0685 | -1.5% |
| `lent_call` | 0.1491 | 0.1467 | -1.6% |
| `load_file_derived` | 72.57 | 71.03 | -2.1% |
| `load_file_hello` | 73.61 | 73.58 | -0.0% |
| `object_mut` | 0.003502 | 0.003502 | +0.0% |
| `object_ref` | 0.001612 | 0.001612 | -0.0% |
| `offers` | 1.27 | 1.31 | +3.2% |
| `remote_clone` | 0.003588 | 0.003586 | -0.1% |
| `retire` | 1.616 | 1.640 | +1.5% |
| `scan_dir` | 235.8 | 226.4 | -4.0% |

No existing metric moved by more than 10%; none of their paths changed
(`call_map_arg`'s +6% is a by-value call this change does not touch).
`call_map_mut` is building a one-entry map, lending it, and the provider
adding a formatted entry in the map's allocator, after which the walk
judges the map and moves nothing. `call_map_mut_moved` adds an entry whose
value the provider built in its own allocator, which the walk then moves
into the map's: one allocation and one free, about what the first
provider's own formatting costs, so the two read alike.

### Every metric more than 10% slower than `pre-checks`, and why

- **`call_map_copied`, about twice the time (both platforms): accepted.**
  The provider builds its map in its own allocator, so the proxy copies it
  into the caller's and frees the original, and nothing the host holds
  needs the library once it is unloaded. It costs what it cost before
  this change (`pre-zero-copy`, +1.5% Windows, -1.2% Linux). A provider
  that implements `<method>_in` and builds there, as `call_map` measures,
  is not copied.
- **`call_map`, +12% Windows and +19% Linux: within 10% laid out alike.**
  The copy is gone (-46% on both against `pre-zero-copy`). What is left is
  reading the carrier and judging on arrival that every container of the
  answer records the caller's allocator, about 9 ns: +8% on Windows built
  with LTO, and on Linux built that way the final build is faster than
  `pre-checks` (-16%). The walk that judges allocates nothing for a map
  with no nested container.
- **`remote_clone`, 0.75 ns to 3.6 ns (Windows) and 9.2 ns (Linux):
  accepted, from the lease work.** A `Remote` was `Copy`; cloning one now
  increments and later decrements its lease's reference count. No call
  path clones a `Remote`: calls go through `&self`.
- **`load_file_*`, +19% and +22% on Linux (about 1 µs): accepted, from
  the descriptor checks and the lease.** The file is no longer read: the
  image comes from `dl_iterate_phdr`, measured at 42 ns per lookup in a
  process like the runner's. What remains is checking the descriptor
  where it enters and building the library's lease (copying its tables,
  one descriptor per provider), the latter +1.4 µs from `pre-lease` to
  `pre-zero-copy`; `pre-checks` did neither. On Windows the same work is
  within 2% of a load that costs 56 µs.
- **`host_get`, +10% Linux (+7% Windows): accepted, from the lease work.**
  It takes a pin and gives it back for each provider it reads, the fix
  for a lookup racing a retire; built with LTO it reads +12% on Linux and
  +8% on Windows.
- **`call_string` +14% and `object_mut` +126%, Linux only: code layout.**
  Neither path changed. Built with LTO they read -4% and -0% against
  `pre-checks`, and `object_mut` reads 2.4 ns or 5.5 ns from one build to
  the next whichever commit it is.

`scan_dir` is back to `pre-checks` (+1% both platforms, from +18% and
+42%): a scan parses each file once, in its probe, and the load that
follows reads the image from the loader.

Fixed while measuring: an `Offer` held its library's lease twice (now
once, through its view), the services' snapshot took a hold per
provider (now per library), the walk that decides whether a returned
tree needs copying allocated before judging the root, and then for every
member of a flat map (it allocates only for a nested container now), and
`Provider::offer` copied a provider's view before the check that turns
away most providers of another kind (it checks first).

## Values as events (`visitor.json`, `visitor-linux.json`)

Local and provisional: the Windows ARM64 machine and its Linux (aarch64)
environment, one run each of six rounds, the runner built alone
(`cargo build -p guatiao-bench --release`). Built with the workspace, the
example application's `exact-numbers` turns on `serde_json`'s
`arbitrary_precision` for every crate, every JSON number then arrives as
its text under a reserved key, and the JSON readings below are slower on
both paths. Microseconds per call, median; the tree builds a `Value` in
the middle, the walk does not.

| metric | Windows tree | Windows walk | | Linux tree | Linux walk | |
| --- | --- | --- | --- | --- | --- | --- |
| configuration to wire bytes | 12.31 | 4.09 | 3.0x | 10.76 | 3.48 | 3.1x |
| wire bytes to configuration | 11.84 | 6.09 | 1.9x | 11.98 | 5.18 | 2.3x |
| 10k floats to wire bytes | 1825 | 759 | 2.4x | 1019 | 668 | 1.5x |
| wire bytes to 10k floats | 969 | 312 | 3.1x | 579 | 304 | 1.9x |
| 64 KiB to wire bytes | 3.74 | 1.55 | 2.4x | 2.32 | 1.32 | 1.8x |
| wire bytes to 64 KiB | 3.55 | 1.77 | 2.0x | 2.27 | 1.31 | 1.7x |
| configuration to JSON | 12.39 | 2.58 | 4.8x | 11.68 | 2.50 | 4.7x |
| JSON to configuration | 16.49 | 7.08 | 2.3x | 15.46 | 6.03 | 2.6x |
| 10k floats to JSON | 1744 | 792 | 2.2x | 1148 | 730 | 1.6x |
| JSON to 10k floats | 1567 | 250 | 6.3x | 1338 | 216 | 6.2x |

Writing a float costs its `Display` text either way, which is what a
value stores; what the walk saves is a node allocated per number, which
is why a list of floats gains least. Reading floats from JSON gains most:
the format hands over an `f64` that the walk keeps, where the tree turns
it into text and back.

## Documents to Python and Dart objects (`documents-python.json`, `documents-dart.json`)

A document to a binding's plain objects and back, three ways: through a
value (`loads` then `to_python()`/`toDart()`, `dumps` of
`Value.from_python`/`Value.fromDart`), through the wire bytes as a tape
read and written in the binding's own language (`parse`, `render`), and
through callbacks (a visitor table the library calls per event, a walker
that calls the library's visitor per event). Each script first checks
that the three read and write the same thing.

```bash
cargo build --release -p guatiao -p guatiao-serde \
  --features guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
GUATIAO_LIBRARY=target/release \
  PYTHONPATH=bindings/python/guatiao/src:bindings/python/guatiao-serde/src \
  python benchmarks/python/documents.py --save documents-python
(cd benchmarks/dart && dart pub get && \
  GUATIAO_LIBRARY=../../target/release dart run bin/documents.dart --save documents-dart)
```

Local and provisional: the Windows ARM64 machine, CPython 3.14.7 and
Dart 3.12.2 (`dart run`, the JIT), one run of fifteen samples each.
Microseconds per call, median. The configuration is fifty servers (7.6 KB
of JSON), then ten thousand floats with fractions, then one map
holding a 1 MiB string.

| Python | value | tape | callbacks | value / tape |
| --- | --- | --- | --- | --- |
| configuration, parse | 1074 | 410 | 937 | 2.6x |
| configuration, render | 2872 | 277 | 1551 | 10.4x |
| 10k floats, parse | 13846 | 5703 | 11131 | 2.4x |
| 10k floats, render | 46139 | 5663 | 19607 | 8.1x |
| 1 MiB string, parse | 589 | 464 | 393 | 1.3x |
| 1 MiB string, render | 828 | 609 | 719 | 1.4x |

| Dart | value | tape | callbacks | value / tape |
| --- | --- | --- | --- | --- |
| configuration, parse | 220 | 122 | 168 | 1.8x |
| configuration, render | 440 | 87 | 121 | 5.1x |
| 10k floats, parse | 1987 | 1416 | 1734 | 1.4x |
| 10k floats, render | 5437 | 1526 | 1924 | 3.6x |
| 1 MiB string, parse | 4066 | 4320 | 4136 | 0.94x |
| 1 MiB string, render | 4579 | 6180 | 4562 | 0.74x |

Both bindings take the tape. A crossing costs a Python program about a
microsecond, so a crossing per event loses to one crossing per document
even with the decoding done in Python; Dart crosses cheaply, and the tape
still wins wherever a document has structure. One large string is the
exception in Dart: the tape copies it into the wire bytes and again into
native memory, where a callback hands it over once, and the cost is
UTF-8 conversion either way.

## Documents to Node and Go data (`documents-node.json`, `documents-go.json`, `documents-go-before.json`)

The same three documents and three ways as for Python and Dart: through a
value (Node: `loads` then `toJS()`, `dumps` of plain data, which builds a
tree; Go: `serde.Parse` then `Decode`, `serde.Emit` of a
`guatiao.NewValue`), through the tape (Node: `parse`, `render`; Go:
`serde.Unmarshal`, `serde.Emit` of Go data), and through callbacks (koffi
callbacks in Node, cgo exports in Go). Each program first checks that the
three read and write the same thing.

```bash
cargo build --release -p guatiao -p guatiao-serde \
  --features guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
(cd bindings/node && npm ci) && (cd bindings/node/guatiao && npm run build) \
  && (cd bindings/node/guatiao-serde && npm run build)
GUATIAO_LIBRARY=target/release node benchmarks/node/documents.mjs --save documents-node
(cd benchmarks/go && GOWORK=off GUATIAO_LIBRARY=../../target/release go run . -save documents-go)
```

Local and provisional: the Windows ARM64 machine, Node 22.23.4 and Go
1.27.3, fifteen samples each, microseconds per call, median. The machine
was shared with other builds: Node is one run; Go is the lowest median of
three runs alternating with a run of the value path on the commit before
(`documents-go-before.json`, the "before" column), whose `serde.Emit`
built a tree from Go data.

| Node | value | tape | callbacks | value / tape |
| --- | --- | --- | --- | --- |
| configuration, parse | 4298 | 174 | 1841 | 24.7x |
| configuration, render | 1119 | 124 | 3580 | 9.0x |
| 10k floats, parse | 47462 | 2344 | 21425 | 20.2x |
| 10k floats, render | 13726 | 1615 | 33467 | 8.5x |
| 1 MiB string, parse | 622 | 722 | 650 | 0.86x |
| 1 MiB string, render | 1520 | 1369 | 1464 | 1.1x |

| Go | before | value | tape | callbacks | before / tape |
| --- | --- | --- | --- | --- | --- |
| configuration, parse | 151 | 137 | 98 | 163 | 1.5x |
| configuration, render | 334 | 213 | 93 | 116 | 3.6x |
| 10k floats, parse | 2163 | 1875 | 1057 | 1649 | 2.0x |
| 10k floats, render | 4687 | 2750 | 1033 | 1709 | 4.5x |
| 1 MiB string, parse | 714 | 719 | 1051 | 210 | 0.68x |
| 1 MiB string, render | 1191 | 1254 | 1690 | 1564 | 0.70x |

Both bindings take the tape. Node reads a tree through koffi a field at a
time, and builds one a koffi call per node, so one crossing per document
wins by an order of magnitude; a koffi callback per event loses too, as a
ctypes one does in Python. Go reads a tree's memory directly, so its
value path was already cheap to read, and the tape still saves the tree
built and freed in between; Go's writer gained most, from one cgo call
per node to none. The Go "value" column is faster than "before" because
the walk over Go data that both the tree and the tape use now passes
plain data (`map[string]any`, `[]any`, strings, numbers) without
reflection and spells a path only for an error.

One large string is the exception, as in Dart: the tape copies it into
the wire bytes and again on each side, where a callback hands it over
once. Go's callbacks read it fastest of all, and lose everywhere else.

## Documents to Julia data (`documents-julia.json`)

The same three documents and three ways: through a value
(`GuatiaoSerde.parse(text)[]`, `GuatiaoSerde.emit(Value(data))`), through
the tape (wire bytes read and written in Julia), and through `@cfunction`
callbacks. The package reads through the callbacks
(`GuatiaoSerde.parse(Any, text)`) and writes through the tape
(`GuatiaoSerde.emit(data)`); the script carries the other two ways.

```bash
cargo build --release -p guatiao -p guatiao-serde \
  --features guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
julia --project=<env holding Guatiao and GuatiaoSerde from the checkout> \
  benchmarks/julia/documents.jl --save documents-julia   # GUATIAO_LIBRARY=target/release
```

Local and provisional: Julia 1.13.2 on the ARM64 machine's Linux
environment (there is no Windows ARM64 Julia), one run of fifteen
samples, microseconds per call, median. Before this, `emit` of plain data
built a `Value`, the "value" column.

| Julia | value | tape | callbacks | value / chosen |
| --- | --- | --- | --- | --- |
| configuration, parse | 89 | 70 | **64** | 1.4x |
| configuration, render | 140 | **93** | 115 | 1.5x |
| 10k floats, parse | 1720 | 1448 | **1448** | 1.2x |
| 10k floats, render | 2032 | **1097** | 1096 | 1.9x |
| 1 MiB string, parse | 173 | 214 | **183** | 0.95x |
| 1 MiB string, render | 428 | **683** | 452 | 0.63x |

Julia is the one binding where callbacks win reading: an `@cfunction`
call from C costs a few nanoseconds, so building plain data per event
beats decoding bytes in Julia, and both beat reading a tree (whose nodes
the library allocates and frees). Writing, the tape wins on documents
with structure; one large string is the exception again, copied once
more on its way through the bytes.

## Documents to R data (`documents-r.json`)

The same three documents, two ways: through a value
(`value_data(serde_parse(text))`, `serde_emit(value(data))`) and through
the tape (`serde_data(text)`, `serde_emit(data)`), the wire bytes read and
written in the package's C. There is no callback way: an R error must
never unwind through the library's frames, so R objects are made only
once its call has returned.

```bash
cargo build --release -p guatiao -p guatiao-serde \
  --features guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
R CMD INSTALL bindings/r/guatiao bindings/r/guatiao.serde
GUATIAO_LIBRARY=target/release Rscript benchmarks/r/documents.R --save documents-r
```

Local and provisional: R 4.6.1 on the ARM64 machine's Linux environment,
one run of fifteen samples, microseconds per call, median, timed with
`proc.time()` (a millisecond clock, so the larger cases are coarse).
Spelling a double now tries 15 significant digits first; searching from
one digit up, rendering the 10k floats took 7250 microseconds both ways.

| R | value | tape | value / tape |
| --- | --- | --- | --- |
| configuration, parse | 106 | 70 | 1.5x |
| configuration, render | 141 | 109 | 1.3x |
| 10k floats, parse | 1563 | 1188 | 1.3x |
| 10k floats, render | 2375 | 2250 | 1.06x |
| 1 MiB string, parse | 1125 | 1125 | 1.0x |
| 1 MiB string, render | 1375 | 2250 | 0.61x |

The package takes the tape for both. R's value path was already C on
both sides of one crossing, so the tape saves the tree built and freed in
between and no more; most of the time left is making R objects and
spelling numbers. One large string is slower through the bytes, as in
every binding.

## Documents to MATLAB and Octave data (`documents-octave.json`)

The same three documents, two ways, as for R: through a value
(`guatiao.serde.parse(text).data()`, `guatiao.serde.emit(guatiao.Value(x))`)
and through the tape (`guatiao.serde.data(text)`, `guatiao.serde.emit(x)`),
the wire bytes read and written in the MEX gateway. No callback way, for
R's reason: an error raised in M must never unwind through the library's
frames.

```bash
cargo build --release -p guatiao -p guatiao-serde \
  --features guatiao-serde/json,guatiao-serde/toml,guatiao-serde/yaml
GUATIAO_LIBRARY=target/release octave-cli --eval \
  "addpath bindings/matlab/guatiao/inst benchmarks/matlab; guatiao.build(); documents('documents-octave')"
```

Local and provisional: Octave 10.3.0 on the ARM64 machine's Linux
environment, one run of fifteen samples, microseconds per call, median.
MATLAB was not measured.

| Octave | value | tape | value / tape |
| --- | --- | --- | --- |
| configuration, parse | 956 | 862 | 1.11x |
| configuration, render | 759 | 645 | 1.18x |
| 10k floats, parse | 6463 | 6467 | 1.0x |
| 10k floats, render | 5357 | 5266 | 1.02x |
| 1 MiB string, parse | 1134 | 1089 | 1.04x |
| 1 MiB string, render | 1382 | 2069 | 0.67x |

The gateway takes the tape for both: it saves the tree built and freed in
between, and little else, since Octave's own arrays cost most of the
time (making one per element reading, inspecting one per element
writing). One large string is slower through the bytes, as in every
binding.

## guatiao-source-remote across a link (`remote-*.jsonl`)

`guatiao-remote-bench` (`benchmarks/remote`) is a server of a provider
made for measuring and a client that prints one JSON line of five
metrics: a small call's p50 and p99 (20 000 `count` calls, one at a
time); calls a second with 32 threads calling at once; MB/s for a 4 MiB
argument answered back (8 MiB a round trip); protocol bytes per small
call, counted at the link, and for TCP on Linux bytes on the wire from
the kernel's counters (payload, plus 52 bytes of IPv4 and TCP headers per
segment); process CPU per small call on each end.

```bash
cargo build -p guatiao-remote-bench --release
target/release/guatiao-remote-bench serve tcp://192.0.2.21:47300 --key K &   # on the server
target/release/guatiao-remote-bench run tcp://192.0.2.21:47300 --key K --label NAME --out FILE
```

`--mss N` caps TCP's segment size (Unix), as on a path without jumbo
frames; `--spin MICROSECONDS` is `LinkOptions::spin` on both ends;
`local://` and `shm://` take a socket path or a pipe name. `run` ends the
server when it is done.

Local and provisional: three runs each, medians, unless a row says
otherwise. The host is an x86-64 Linux server, the VM a guest on it
(virtio, MTU 9000); the FreeBSD VM (4 cores) and the macOS VM are guests
on the same host (`remote-freebsd-macos.jsonl`); Windows is the ARM64
workstation. The macOS VM is slow at everything, raw sockets included
(about 500 µs a raw TCP round trip from the host): its rows show that the
crate builds and runs there, not what macOS costs.

| run | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | protocol B/call | wire B/call | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- | --- | --- |
| host to VM, TCP, MTU 9000 | 116.2 | 192.2 | 17 958 | 387 | 21.3 | 125.3 | 81.1 / 88.3 |
| host to VM, TCP, MSS 1448 | 123.4 | 202.3 | 17 967 | 396 | 21.3 | 125.3 | 77.9 / 94.1 |
| host to VM, TCP, MTU 9000, spin 0 | 182.2 | 261.4 | 16 829 | 384 | 21.3 | 125.3 | 48.8 / 64.6 |
| host to VM, TCP, MSS 1448, spin 0 | 177.8 | 252.3 | 17 406 | 405 | 21.3 | 125.3 | 52.8 / 61.8 |
| host, TCP loopback | 21.8 | 43.6 | 19 713 | 475 | 21.3 | 125.3 | 29.9 / 29.7 |
| host, Unix socket | 15.7 | 27.5 | 101 631 | 500 | 21.3 | - | 23.2 / 22.9 |
| host, shared memory | 4.4 | 9.2 | 183 127 | 474 | 21.3 | - | 9.2 / 9.2 |
| Windows, TCP loopback | 28.8 | 73.2 | 52 377 | 792 | 21.3 | - | 32.0 / 30.5 |
| Windows, named pipe | 16.2 | 42.9 | 138 732 | 1 571 | 21.3 | - | 19.5 / 18.8 |
| Windows, shared memory | 1.3 | 5.4 | 331 859 | 1 289 | 21.3 | - | 4.7 / 4.7 |
| host to FreeBSD VM, TCP | 120.5 | 186.6 | 17 657 | 266 | 21.3 | 125.3 | 81.8 / 81.7 |
| FreeBSD VM, TCP loopback (one run) | 17.5 | 30.2 | 32 855 | 444 | 21.3 | - | 22.6 / 22.4 |
| FreeBSD VM, Unix socket (one run) | 16.3 | 35.5 | 39 108 | 372 | 21.3 | - | 23.1 / 22.4 |
| FreeBSD VM, shared memory (one run) | 6.3 | 18.2 | 33 729 | 735 | 21.3 | - | 13.4 / 13.4 |
| host to macOS VM, TCP | 702.5 | 1 555.1 | 7 197 | 56 | 21.3 | 177.3 | 116.7 / 411.7 |
| macOS VM, TCP loopback (one run) | 656.7 | 1 268.5 | 6 790 | 631 | 21.3 | - | 378.1 / 372.1 |
| macOS VM, Unix socket (one run) | 587.0 | 1 205.1 | 6 947 | 58 | 21.3 | - | 379.1 / 376.0 |
| macOS VM, shared memory (one run) | 676.5 | 1 261.3 | 6 948 | 627 | 21.3 | - | 459.2 / 453.2 |

How long a waiting thread polls (`remote-host-to-vm-spin.jsonl`, host to
VM, MTU 9000):

| spin | p50 µs | p99 µs | CPU µs/call, client / server |
| --- | --- | --- | --- |
| 50 µs (the default) | 113.6 | 186.9 | 76.3 / 86.2 |
| 100 µs | 78.1 | 177.0 | 95.7 / 96.6 |
| 200 µs | 74.2 | 120.2 | 89.7 / 86.8 |
| 400 µs | 69.9 | 125.5 | 85.3 / 82.1 |

- **A small call is 21 bytes of frames and 125 on the wire**: two
  segments, one each way, so the headers outweigh the call.
- **Jumbo frames change nothing here**: a small call fits one segment
  either way, and a 4 MiB round trip moves about 400 MB/s, a tenth of
  what raw TCP moves on the same path (3.2-3.9 GB/s), so the bound is
  the link's own copies and hand-offs, not the wire. On the Windows
  workstation a raw TCP echo of 4 MiB (a length, the bytes, back again)
  moves about 1.1 GB/s over loopback, about what the link moves; in a
  Linux (aarch64) VM on it raw TCP moves 3.5 GB/s and the link 1.5-1.8
  GB/s. Writing a large last argument or answer from where it lies,
  rather than copying it into its frame first, saved about 2 ms of a
  9 ms round trip per stage on Windows but was not kept: over 300 rounds
  a run, it moved the named pipe on Windows 38 % faster and shared memory
  and the Unix socket in the VM 28 % and 12 % slower. Reading a frame
  with `read_to_end` instead of over zeros halved every stream there: it
  reads in small pieces a socket rarely fills.
- **Polling pays only past the round trip.** At 50 µs a waiting thread
  polls, then sleeps and pays the wake-up as well; polling past the
  answer's arrival drops the wake-up for about the same CPU, and not
  polling at all saves about 40 % of the CPU for about 55 % more latency. On the
  same machine every answer arrives inside 50 µs, so the default stands
  for local links.
- **TCP from 32 threads on Linux reached 17 000-20 000 calls a second**,
  on loopback and to the VM alike, where the Unix socket and shared
  memory reached 100 000-180 000 through the same link: every frame was
  one write under the link's writer, and every hand-off of the writer a
  wake-up. Threads now share writes (below).

Threads calling at once share the link's writes
(`remote-windows-shared-writes.jsonl`, `remote-linux-vm-shared-writes.jsonl`):
before, each frame was one write under the link's writer, about 20-25 µs
of write over loopback TCP in the Linux VM plus the wake-up of the next thread
waiting for the writer; now a frame is queued and the thread that finds
the writer free writes every queued frame in one vectored write. The
Windows workstation and a Linux (aarch64) VM on it, the two builds alternating, five runs
each, a run started only once the machine was quiet (under 50 % busy on
Windows, whose own services take 30 % or more; under 20 % in the VM);
medians. Other work on the machine still moved single runs by 2x: read
the ratios, not the figures. A single call over the VM's Unix socket took
about 120 µs here against about 50 µs over its TCP, before and after alike:
its answers came after the 50 µs poll (below).

| run | p50 µs, before / after | calls/s, 32 threads, before | after | change |
| --- | --- | --- | --- | --- |
| Windows, TCP loopback | 29.4 / 29.7 | 50 695 | 83 304 | 1.64x |
| Windows, named pipe | 17.8 / 18.1 | 129 736 | 142 215 | 1.10x |
| Windows, shared memory | 2.0 / 1.3 | 271 756 | 364 262 | 1.34x |
| Linux VM, TCP loopback | 51.5 / 54.6 | 20 696 | 41 435 | 2.00x |
| Linux VM, Unix socket | 122.2 / 131.9 | 32 834 | 46 462 | 1.42x |
| Linux VM, shared memory | 2.2 / 2.5 | 53 231 | 65 079 | 1.22x |

A waiting thread polls just past the link's typical wait
(`remote-windows-to-linux-vm-polling.jsonl`, `remote-windows-polling.jsonl`,
`remote-linux-vm-polling.jsonl`): 1.5x the typical answer round trip (for
a caller) or gap between calls (for an idle pool thread), at least 50 µs,
at most `spin` (now 200 µs by default), and not at all past it; against
the fixed 50 µs poll before. The Windows client to a server in the Linux
VM is a path whose round trip (~100 µs) is longer than the old poll, as
host to VM was above; the same protocol as the table before, five runs,
medians; CPU is the client's.

| run | p50 µs, fixed / adaptive | calls/s, 32 threads, fixed / adaptive | CPU µs/call, fixed / adaptive |
| --- | --- | --- | --- |
| Windows to the Linux VM, TCP | 159.8 / 95.9 | 57 397 / 62 409 | 89.1 / 89.1 |
| Windows, TCP loopback | 29.2 / 30.2 | 77 728 / 85 859 | 35.2 / 38.3 |
| Windows, named pipe | 17.9 / 18.1 | 136 844 / 151 388 | 23.4 / 21.9 |
| Windows, shared memory | 1.3 / 1.4 | 291 083 / 380 957 | 4.7 / 4.7 |
| Linux VM, TCP loopback | 55.8 / 56.7 | 35 065 / 53 628 | 73.4 / 70.4 |
| Linux VM, Unix socket | 130.8 / 59.5 | 41 749 / 39 735 | 117.3 / 105.6 |
| Linux VM, shared memory | 1.8 / 2.3 | 53 264 / 67 143 | 7.5 / 8.2 |

On the same machine every answer still comes inside the 50 µs floor, so
local links poll as before; the Linux VM's Unix socket, whose answers
took longer than 50 µs, halves its latency. Polling less than the floor
was tried and lost: on shared memory a thread polling just past a 2 µs
round trip missed often enough to pay wake-ups, and the Linux VM's p99
went from 8 to 120-180 µs at four times the CPU.

TLS, the secure level (`remote-tls-windows.jsonl`,
`remote-tls-wsl.jsonl`): `tls://ADDRESS:PORT --identity PATH` on both
ends, each pinning the one identity, beside trusted TCP; five runs each,
alternating, medians. The Windows workstation was quiet (busy 8-48 %),
WSL less so (8-20 %, and its runs moved the most).

| link, loopback | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- |
| Windows ARM64, trusted TCP | 30.2 | 67.0 | 95 276 | 1 301 | 35.9 / 38.3 |
| Windows ARM64, TLS | 32.8 | 65.9 | 121 606 | 662 | 41.4 / 37.5 |
| WSL (aarch64), trusted TCP | 56.2 | 172.8 | 59 866 | 1 807 | 59.5 / 58.6 |
| WSL (aarch64), TLS | 37.4 | 171.7 | 80 360 | 931 | 49.7 / 50.3 |

- rustls alone seals and opens about 2 GB/s on one thread of the Windows
  machine; a 4 MiB frame one way between two threads crosses 750-1 080
  MB/s since the reader grows its reads to 1 MiB (280-400 MB/s at 64
  KiB), so the round trip's gap to TCP is the extra pass over every byte
  on each side.
- More calls a second than TCP with 32 threads: the writer seals every
  frame queued in one go and the reader opens what one read brought, so
  fewer system calls carry the same calls. Not explained further.

QUIC, the secure level over UDP (`remote-quic-windows.jsonl`,
`remote-quic-wsl.jsonl`): `quic://ADDRESS:PORT --identity PATH` on both
ends, beside `tls://`; five runs each, alternating, each started with the
machine under 50 % busy (Windows) or 20 % (WSL), medians.

| link, loopback | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- |
| Windows ARM64, TLS | 32.0 | 85.6 | 108 965 | 617 | 37.5 / 36.7 |
| Windows ARM64, QUIC | 70.3 | 261.8 | 137 089 | 384 | 120.3 / 120.3 |
| WSL (aarch64), TLS | 60.1 | 188.6 | 59 149 | 1 223 | 67.9 / 67.4 |
| WSL (aarch64), QUIC | 217.3 | 2 321.5 | 33 764 | 226 | 129.9 / 132.0 |

- QUIC's small call costs two (Windows) to four (WSL) times TLS's, and
  three (Windows) or two (WSL) times its CPU; with 32 threads Windows'
  QUIC passes TLS.
- WSL's QUIC moved a 4 MiB round trip at 40-374 MB/s from run to run and
  its p99 reached 4 ms. Not explained; UDP socket buffers are a guess,
  unmeasured.

RDMA, on Soft-RoCE only (`remote-vm-rdma.jsonl`): built with
`--features rdma`, `rdma://ADDRESS:PORT` is the address of the link's TCP
set-up. One x86-64 Linux VM with one rxe device, both ends on it, beside
trusted TCP and the Unix socket on the same VM, three runs each, medians.
**Soft-RoCE is RDMA done in software by the kernel's network stack: these
numbers say nothing about an RDMA NIC.**

| run, same VM | p50 µs | p99 µs | calls/s, 32 threads | MB/s, 4 MiB | protocol B/call | CPU µs/call, client / server |
| --- | --- | --- | --- | --- | --- | --- |
| RDMA, Soft-RoCE loopback | 11.2 | 18.6 | 17 536 | 388 | 21.3 | 22.9 / 22.9 |
| Unix socket | 19.4 | 44.5 | 108 172 | 540 | 21.3 | 28.9 / 27.9 |
| TCP loopback | 41.2 | 132.4 | 9 539 | 561 | 21.3 | 48.2 / 46.6 |

- perftest on the same device: 64-byte RDMA writes take about 3.0 µs one
  way (`ib_write_lat`), 4 MiB writes move 1 751-1 815 MiB/s
  (`ib_write_bw`).
- **The bulk path stays where the link's copies put it**: 388 MB/s for a
  4 MiB round trip against 1.8 GB/s of raw Soft-RoCE, as TCP's ~400 MB/s
  stands against 3.2-3.9 GB/s of raw TCP. RDMA removes none of the
  link's copies and adds one into the staging area.
- CPU per call is the two processes' own; Soft-RoCE's work in kernel
  threads is not counted.
- The runs were noisy (TCP's p50 was 38.9, 93.2 and 41.2 µs): read the
  medians as an order, not as figures.
