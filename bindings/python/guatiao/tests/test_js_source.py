# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A Python host adding the guatiao-source-js source: libraries written in
JavaScript, described by manifests, served afresh when their code changes,
and stopped at their limits.

The source is its own workspace (`crates/guatiao-source-js`); `GUATIAO_JS_SOURCE`
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
    os.environ.get("GUATIAO_JS_SOURCE")
    or native.REPO / "crates" / "guatiao-source-js" / "target" / "debug" / native.library_name("guatiao_source_js")
)
if not _SOURCE.is_file():
    pytest.skip(f"no guatiao-source-js build at {_SOURCE}", allow_module_level=True)

_EXAMPLES = native.REPO / "examples" / "js_provider"
_KINDS = str(native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json")
_KINDS_MODULE = native.REPO / "examples" / "greeter_kind" / "node" / "greeter_kind.js"


def _library(directory: Path, name: str, kinds: str, code: str) -> None:
    """A library of one module `name`.js whose kinds module is imported by
    its absolute path."""
    here = _KINDS_MODULE.resolve().as_posix()
    (directory / f"{name}.js").write_text(code.replace("__KINDS__", here), encoding="utf-8")
    (directory / f"{name}.guatiao-js.toml").write_text(
        f'name = "{name}"\nversion = "1.0.0"\nmain = "{name}.js"\nkinds = [{kinds}]\n'
        f'paths = [".", "{_KINDS_MODULE.resolve().parent.as_posix()}"]\n',
        encoding="utf-8",
    )


@pytest.fixture()
def reg():
    registry = Registry("js-source", "0.1")
    yield registry
    for library in registry.libraries():
        registry.retire(library["key"])
    registry.collect()
    registry.close()


def test_the_javascript_greeters_are_served_and_called(reg):
    name = reg.add_source_file(_SOURCE, {"dirs": [str(_EXAMPLES)], "kinds": [_KINDS]})
    changed = reg.refresh()
    assert sorted(s["id"] for s in changed["loaded"]) == ["quickjs_greeter", "quickjs_trees"], changed
    assert changed["refused"] == []
    assert all(s["source"] == name for s in changed["loaded"])
    hello = reg.bind("quickjs_greeter_hello", Greeter)
    assert hello.greet("ana").to_python() == {"greeting": "hello, ana"}
    polite = reg.bind("quickjs_greeter_polite", Greeter)
    assert polite.greet("bo").to_python() == {"greeting": "good day, bo"}
    counter = reg.bind("quickjs_greeter_hello", Counter)
    first = counter.count()
    assert counter.count() == first + 1
    assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}


def test_an_edit_is_served_and_a_deleted_library_retired(reg, tmp_path):
    lib = tmp_path / "edited"
    lib.mkdir()
    code = (_EXAMPLES / "quickjs_greeter" / "greeters.js").read_text(encoding="utf-8").replace(
        "'../../greeter_kind/node/greeter_kind.js'", "'__KINDS__'"
    )
    _library(lib, "js_edited", '"greeter", "counter"', code)
    reg.add_source_file(_SOURCE, {"dirs": [str(tmp_path)], "kinds": [_KINDS]})
    assert [s["id"] for s in reg.refresh()["loaded"]] == ["js_edited"]
    assert reg.bind("js_edited_hello", Greeter).greet("ana").to_python() == {"greeting": "hello, ana"}

    time.sleep(0.05)
    _library(lib, "js_edited", '"greeter", "counter"', code.replace("GREETING = 'hello'", "GREETING = 'howdy'"))
    changed = reg.refresh()
    assert [s["id"] for s in changed["loaded"]] == ["js_edited"]
    assert [s["id"] for s in changed["retired"]] == ["js_edited"]
    assert reg.bind("js_edited_hello", Greeter).greet("ana").to_python() == {"greeting": "howdy, ana"}

    shutil.rmtree(lib)
    changed = reg.refresh()
    assert [s["id"] for s in changed["retired"]] == ["js_edited"]
    assert reg.collect() == ["js_edited"]


def test_a_runaway_call_is_stopped_and_the_host_carries_on(reg, tmp_path):
    _library(
        tmp_path,
        "js_spin",
        '"greeter"',
        "import { Provider } from 'guatiao/provide';\n"
        "import { Greeter } from '__KINDS__';\n"
        "export class Spin extends Provider {\n"
        "  static kinds = [Greeter];\n"
        "  greet(name) {\n"
        "    if (name === 'loop') while (true) {}\n"
        "    return { greeting: `calm, ${name}` };\n"
        "  }\n"
        "}\n",
    )
    reg.add_source_file(
        _SOURCE, {"dirs": [str(tmp_path)], "kinds": [_KINDS], "limits": {"operations": 100000}}
    )
    assert reg.refresh()["refused"] == []
    spin = reg.bind("js_spin_spin", Greeter)
    with pytest.raises(Exception, match=r"stopped at its operations limit \(100000\)"):
        spin.greet("loop")
    assert spin.greet("bo").to_python() == {"greeting": "calm, bo"}
