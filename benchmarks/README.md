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

Local and provisional: one Windows ARM64 machine, and a Linux (aarch64)
environment on the same machine. Microseconds per call, median.

- `pre-checks`: before descriptors were checked where they enter.
- `pre-lease`: before handles held a lease on their library.
- `0.0.0-alpha.1`: both.

The Windows baselines were taken before the lease work began, each
pooled from five runs alternating with the other; the `0.0.0-alpha.1`
file pools five final runs from a session that alternated all three
builds (where pre-lease to final read the same, within a point or
two). The Linux files pool five runs each from one session alternating
all three.

### Windows (`0.0.0-alpha.1-pre-checks.json`, `-pre-lease.json`, `0.0.0-alpha.1.json`)

| metric | pre-checks | pre-lease | 0.0.0-alpha.1 | vs pre-lease | vs pre-checks |
| --- | --- | --- | --- | --- | --- |
| `call_map` | 0.1384 | 0.1421 | 0.2893 | +103.6% | +109.1% |
| `call_scalar` | 0.002117 | 0.00212 | 0.002117 | -0.2% | -0.0% |
| `call_string` | 0.2016 | 0.2069 | 0.2166 | +4.7% | +7.5% |
| `host_get` | 2.572 | 2.573 | 2.674 | +3.9% | +3.9% |
| `instantiate_drop` | 0.5784 | 0.5775 | 0.5821 | +0.8% | +0.6% |
| `lent_call` | 0.1403 | 0.1423 | 0.1497 | +5.2% | +6.7% |
| `load_file_derived` | 58.93 | 81.64 | 82.54 | +1.1% | +40.1% |
| `load_file_hello` | 58.82 | 83.33 | 83.05 | -0.3% | +41.2% |
| `object_mut` | 0.003496 | 0.003493 | 0.003493 | +0.0% | -0.1% |
| `object_ref` | 0.00162 | 0.001609 | 0.001613 | +0.2% | -0.4% |
| `offers` | 1.276 | 1.263 | 1.27 | +0.5% | -0.5% |
| `remote_clone` | 0.0007469 | 0.0007486 | 0.003601 | +381.0% | +382.1% |
| `retire` | 1.61 | 1.683 | 1.692 | +0.6% | +5.1% |
| `scan_dir` | 231.1 | 274.7 | 272.3 | -0.8% | +17.9% |

### Linux (`0.0.0-alpha.1-linux-pre-checks.json`, `-linux-pre-lease.json`, `-linux.json`)

| metric | pre-checks | pre-lease | 0.0.0-alpha.1 | vs pre-lease | vs pre-checks |
| --- | --- | --- | --- | --- | --- |
| `call_map` | 0.07173 | 0.07083 | 0.1647 | +132.5% | +129.6% |
| `call_scalar` | 0.004578 | 0.004568 | 0.004579 | +0.2% | +0.0% |
| `call_string` | 0.1054 | 0.1047 | 0.1191 | +13.7% | +12.9% |
| `host_get` | 1.905 | 1.922 | 2.256 | +17.4% | +18.4% |
| `instantiate_drop` | 0.5588 | 0.5567 | 0.5586 | +0.3% | -0.0% |
| `lent_call` | 0.07641 | 0.07622 | 0.0796 | +4.4% | +4.2% |
| `load_file_derived` | 5.447 | 21.86 | 23.65 | +8.2% | +334.2% |
| `load_file_hello` | 6.207 | 27.95 | 30.15 | +7.9% | +385.7% |
| `object_mut` | 0.002412 | 0.002477 | 0.002385 | -3.7% | -1.1% |
| `object_ref` | 0.001621 | 0.001619 | 0.001624 | +0.3% | +0.2% |
| `offers` | 0.9545 | 0.902 | 1.041 | +15.4% | +9.1% |
| `remote_clone` | 0.0007518 | 0.0007526 | 0.009148 | +1115.6% | +1116.9% |
| `retire` | 1.055 | 1.093 | 1.255 | +14.8% | +19.0% |
| `scan_dir` | 97.21 | 132.5 | 138.1 | +4.2% | +42.0% |

### Every metric more than 10% slower, and why

- **`call_map`, about twice the time (both platforms): accepted.** A
  `Map` a library builds lives in the library's allocator, and the
  proxy now copies it into the host's and frees the original, so the
  host holds nothing that needs the library once it is unloaded. The
  copy is the price of that guarantee; a tree already in the host's
  allocator is handed over as it is. Measured cost: about 0.15 µs on
  Windows and 0.09 µs on Linux for a one-entry map.
- **`remote_clone`, 0.75 ns to 3.6 ns (Windows) and 9.1 ns (Linux):
  accepted.** A `Remote` was `Copy`; cloning one now increments and
  later decrements its lease's reference count. No call path clones a
  `Remote`: calls go through `&self`. The Linux build's atomics are the
  slower of the two.
- **`call_string`, `host_get`, `offers`, `retire`, 13% to 17% on Linux
  only: code layout, accepted.** On Windows the same changes are within
  5%. Built for Linux with LTO and one codegen unit, which lays the code
  out differently, the same four read -6.7%, -0.0%, +5.9% and +5.1%
  against the pre-lease build made the same way, while `call_map` and
  `remote_clone` stay as above. `call_string`'s own path did not change.
  `host_get` does take a pin and give it back for each provider it reads
  (the fix for a lookup racing a retire), and `retire` republishes with
  one lease hold per library; both are inside the layout noise once the
  code is laid out alike.
- **`load_file_*` (+40% Windows, about 4x Linux) and `scan_dir` (+18%,
  +42%) against `pre-checks`: accepted here, the previous change's.**
  These came with checking descriptors where they enter: each load reads
  the file's layout (segments, executable sections, the entry symbol) to
  place the image, and `scan_dir` does so after already probing the
  file. The lease work moved them by under 9%. Parsing once on the scan
  path is follow-up work.

Fixed while measuring: an `Offer` held its library's lease twice (now
once, through its view), the services' snapshot took a hold per
provider (now per library), the walk that decides whether a returned
tree needs copying allocated before judging the root (it no longer
does), and `Provider::offer` copied a provider's view before the check
that turns away most providers of another kind (it checks first).
