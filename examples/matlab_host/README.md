# A plugin host in MATLAB or GNU Octave

`host.m` loads greeter libraries written in any language and calls each
one through the `greeter` kind's table: a map back, a map lent and changed
in place, a map handed over, an object handed in and one handed back (a
conversation), an object lent for one call (a listener). A provider that
builds instances from a configuration is given one its schema refuses,
then one it accepts. The greeters written in M
([`../matlab_provider`](../matlab_provider/)) are registered beside the
ones it loads, and the greeters written in Python are served by the
guatiao-py bridge when one is named. It closes everything it opened and
prints what it did.

The kinds are declared in
[`../greeter_kind/matlab/+greeterkind`](../greeter_kind/matlab/+greeterkind/Greeter.m),
from the kind's C header
([`../greeter_kind/include/greeter_kind.h`](../greeter_kind/include/greeter_kind.h)).

## Run it

From the repository root, after building the workspace, build the MEX
file once and run the host (it puts the package, the kinds and the M
greeters on the path itself):

```bash
cargo build --workspace --all-features
export GUATIAO_LIBRARY=$PWD/target/debug
octave-cli --eval "addpath bindings/matlab/inst; guatiao.build(); addpath examples/matlab_host; host('target/debug')"
matlab -batch "addpath bindings/matlab/inst; guatiao.build(); addpath examples/matlab_host; host('target/debug')"
```

`target/debug` holds the Rust greeters (`derived_greeter`, written with
the derive macros) and `hello_library`, whose table is not this kind's,
which the host reports and passes over.

To add the greeter written in C, build it into a folder of its own and
name that folder too:

```bash
mkdir -p c-greeter
cc -shared -fPIC -I crates/guatiao/include -I examples/greeter_kind/include \
   -o c-greeter/libc_greeter.so crates/guatiao/tests/c_consumer/greeter_in_c.c
octave-cli --eval "addpath bindings/matlab/inst examples/matlab_host; host('target/debug', 'c-greeter')"
```

On Windows the output is `c-greeter/c_greeter.dll`; on macOS,
`c-greeter/libc_greeter.dylib`. The library must be built for the
process's architecture.

The greeter written in Go is a library file too
([`../go_provider/README.md`](../go_provider/README.md) builds it); name
its folder the same way. To add the greeters written in Python, through
the `guatiao-py` bridge (built as
[`../python_provider/README.md`](../python_provider/README.md) says), name
the bridge, where its manifests are, and what they import:

```matlab
host('target/debug', '--python-source', 'crates/guatiao-py/target/debug/libguatiao_py.so', ...
     '--python-dir', 'examples/python_provider', '--python-path', 'bindings/python/src', ...
     '--python-path', 'examples/greeter_kind/python')
```
