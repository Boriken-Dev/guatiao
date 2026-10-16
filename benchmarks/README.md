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
| `call_string` | `Greeter::shout`, answering a `String` |
| `instantiate_drop` | `Offer::instantiate` from a configuration, then drop |
| `object_ref` | `Conversation::turns`, a `&self` object method |
| `object_mut` | `Conversation::read`, a `&mut self` object method |
| `lent_call` | `Greeter::greet_into`, lending a listener for the call |
| `remote_clone` | cloning a `Remote` and dropping the clone |
| `retire` | `Registry::retire` of a library loaded just before |
| `host_get` | `hello_library`'s echo: a library listing the host's greeters through its services and calling one |

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
