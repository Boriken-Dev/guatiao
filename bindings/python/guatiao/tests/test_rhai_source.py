# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A Python host adding the guatiao-source-rhai source: libraries written in
Rhai, described by manifests, served afresh when their code changes, and
stopped at their limits.

The source is its own workspace (`crates/guatiao-source-rhai`); `GUATIAO_RHAI_SOURCE`
names the build, else its debug build is looked for, else these skip and
say so."""

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
    os.environ.get("GUATIAO_RHAI_SOURCE")
    or native.REPO / "crates" / "guatiao-source-rhai" / "target" / "debug" / native.library_name("guatiao_source_rhai")
)
if not _SOURCE.is_file():
    pytest.skip(f"no guatiao-source-rhai build at {_SOURCE}", allow_module_level=True)

_EXAMPLES = native.REPO / "examples" / "rhai_provider"
_KINDS = str(native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json")


@pytest.fixture()
def reg():
    registry = Registry("rhai-source", "0.1")
    yield registry
    for library in registry.libraries():
        registry.retire(library["key"])
    registry.collect()
    registry.close()


def test_the_rhai_greeters_are_served_and_called(reg):
    name = reg.add_source_file(_SOURCE, {"dirs": [str(_EXAMPLES)], "kinds": [_KINDS]})
    changed = reg.refresh()
    assert [s["id"] for s in changed["loaded"]] == ["rhai_greeter"], changed
    assert changed["refused"] == []
    assert all(s["source"] == name for s in changed["loaded"])
    hello = reg.bind("rhai_greeter_hello", Greeter)
    assert hello.greet("ana").to_python() == {"greeting": "hello, ana"}
    polite = reg.bind("rhai_greeter_polite", Greeter)
    assert polite.greet("bo").to_python() == {"greeting": "good day, bo"}
    with pytest.raises(Exception, match="nobody to greet"):
        hello.greet("")
    counter = reg.bind("rhai_greeter_hello", Counter)
    first = counter.count()
    assert counter.count() == first + 1
    assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}


def test_an_edit_is_served_and_a_deleted_library_retired(reg, tmp_path):
    lib = tmp_path / "edited"
    lib.mkdir()
    code = (_EXAMPLES / "rhai_greeter" / "rhai_greeter.rhai").read_text(encoding="utf-8")
    (lib / "rhai_edited.rhai").write_text(code, encoding="utf-8")
    (lib / "rhai_edited.guatiao.toml").write_text(
        'type = "rhai"\nmodule = "rhai_edited"\nkinds = ["greeter", "counter"]\n', encoding="utf-8"
    )
    reg.add_source_file(_SOURCE, {"dirs": [str(tmp_path)], "kinds": [_KINDS]})
    assert [s["id"] for s in reg.refresh()["loaded"]] == ["rhai_edited"]
    assert reg.bind("rhai_edited_hello", Greeter).greet("ana").to_python() == {"greeting": "hello, ana"}

    time.sleep(0.05)
    (lib / "rhai_edited.rhai").write_text(
        code.replace('const GREETING = "hello"', 'const GREETING = "howdy"'), encoding="utf-8"
    )
    changed = reg.refresh()
    assert [s["id"] for s in changed["loaded"]] == ["rhai_edited"]
    assert [s["id"] for s in changed["retired"]] == ["rhai_edited"]
    assert reg.bind("rhai_edited_hello", Greeter).greet("ana").to_python() == {"greeting": "howdy, ana"}

    shutil.rmtree(lib)
    changed = reg.refresh()
    assert [s["id"] for s in changed["retired"]] == ["rhai_edited"]
    assert reg.collect() == ["rhai_edited"]


def test_a_runaway_call_is_stopped_and_the_host_carries_on(reg, tmp_path):
    (tmp_path / "rhai_spin.rhai").write_text(
        """
const VERSION = "1.0.0";
let spin = #{ kinds: ["greeter"], greet: |name| {
  if name == "loop" { loop {} }
  #{ greeting: `calm, ${name}` }
} };
#{ providers: #{ spin: spin } }
""",
        encoding="utf-8",
    )
    (tmp_path / "rhai_spin.guatiao.toml").write_text(
        'type = "rhai"\nmodule = "rhai_spin"\nkinds = ["greeter"]\n', encoding="utf-8"
    )
    reg.add_source_file(
        _SOURCE, {"dirs": [str(tmp_path)], "kinds": [_KINDS], "limits": {"operations": 100000}}
    )
    assert reg.refresh()["refused"] == []
    spin = reg.bind("rhai_spin_spin", Greeter)
    with pytest.raises(Exception, match=r"stopped at its operations limit \(100000\)"):
        spin.greet("loop")
    assert spin.greet("bo").to_python() == {"greeting": "calm, bo"}
