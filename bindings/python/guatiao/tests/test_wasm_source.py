# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A Python host adding the guatiao-source-wasm source: libraries compiled to
WebAssembly from Rust and from Go, described by manifests, served afresh
when their module changes, and stopped at their limits.

The source is its own workspace (`crates/guatiao-source-wasm`); `GUATIAO_WASM_SOURCE`
names the build, else its debug build is looked for. The example guests
are built by that crate's tests (or by the commands in their manifests);
without them these skip and say so."""

from __future__ import annotations

import os
import shutil
import sys
import time
from pathlib import Path

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native

sys.path.insert(0, str(native.GREETER_KIND_PY))

from greeter_kind import Counter, Greeter  # noqa: E402

from guatiao import Registry  # noqa: E402

_SOURCE = Path(
    os.environ.get("GUATIAO_WASM_SOURCE")
    or native.REPO / "crates" / "guatiao-source-wasm" / "target" / "debug" / native.library_name("guatiao_source_wasm")
)
if not _SOURCE.is_file():
    pytest.skip(f"no guatiao-source-wasm build at {_SOURCE}", allow_module_level=True)

_RUST = native.REPO / "examples" / "wasm_greeter_rust"
_RUST_MODULE = _RUST / "target" / "wasm32-unknown-unknown" / "release" / "wasm_greeter_rust.wasm"
_GO = native.REPO / "examples" / "wasm_greeter_go"
_GO_MODULE = _GO / "wasm_greeter_go.wasm"
for _module in (_RUST_MODULE, _GO_MODULE):
    if not _module.is_file():
        pytest.skip(f"the example guest {_module.name} is not built", allow_module_level=True)

_KINDS = str(native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json")


@pytest.fixture()
def reg():
    registry = Registry("wasm-source", "0.1")
    yield registry
    for library in registry.libraries():
        registry.retire(library["key"])
    registry.collect()
    registry.close()


@pytest.mark.parametrize("module", ["wasm_greeter_rust", "wasm_greeter_go"])
def test_the_wasm_greeters_are_served_and_called(reg, module):
    name = reg.add_source_file(_SOURCE, {"dirs": [str(_RUST), str(_GO)], "kinds": [_KINDS]})
    changed = reg.refresh()
    assert sorted(s["id"] for s in changed["loaded"]) == ["wasm_greeter_go", "wasm_greeter_rust"], changed
    assert changed["refused"] == []
    assert all(s["source"] == name for s in changed["loaded"])
    hello = reg.bind(f"{module}_hello", Greeter)
    assert hello.greet("ana").to_python() == {"greeting": "hello, ana"}
    polite = reg.bind(f"{module}_polite", Greeter)
    assert polite.greet("bo").to_python() == {"greeting": "good day, bo"}
    counter = reg.bind(f"{module}_hello", Counter)
    first = counter.count()
    assert counter.count() == first + 1
    assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}


def test_a_rebuilt_module_is_served_and_a_deleted_library_retired(reg, tmp_path):
    lib = tmp_path / "rebuilt"
    lib.mkdir()
    (lib / "wasm_rebuilt.guatiao.toml").write_text(
        'type = "wasm"\nmodule = "wasm_rebuilt"\nversion = "1.0.0"\nkinds = ["greeter", "counter", "wide"]\n',
        encoding="utf-8",
    )
    shutil.copy(_RUST_MODULE, lib / "wasm_rebuilt.wasm")
    reg.add_source_file(_SOURCE, {"dirs": [str(tmp_path)], "kinds": [_KINDS]})
    assert [s["id"] for s in reg.refresh()["loaded"]] == ["wasm_rebuilt"]
    assert reg.bind("wasm_rebuilt_polite", Greeter).greet("ana").to_python() == {"greeting": "good day, ana"}

    # Another build under the same name: the Go one.
    time.sleep(0.05)
    shutil.copy(_GO_MODULE, lib / "wasm_rebuilt.wasm")
    changed = reg.refresh()
    assert [s["id"] for s in changed["loaded"]] == ["wasm_rebuilt"]
    assert [s["id"] for s in changed["retired"]] == ["wasm_rebuilt"]
    with pytest.raises(Exception, match="nobody to greet, politely"):
        reg.bind("wasm_rebuilt_polite", Greeter).greet("")

    shutil.rmtree(lib)
    changed = reg.refresh()
    assert [s["id"] for s in changed["retired"]] == ["wasm_rebuilt"]
    assert "wasm_rebuilt" in reg.collect()


def test_a_trap_is_contained_and_the_host_carries_on(reg):
    reg.add_source_file(
        _SOURCE,
        {"dirs": [str(_RUST)], "kinds": [_KINDS], "limits": {"operations": 10_000_000, "time_ms": 2000}},
    )
    assert reg.refresh()["refused"] == []
    hello = reg.bind("wasm_greeter_rust_hello", Greeter)
    with pytest.raises(Exception, match="a provider bug, on request"):
        hello.greet("panic")
    assert hello.greet("bo").to_python() == {"greeting": "hello, bo"}
