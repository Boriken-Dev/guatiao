# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A Python host adding the guatiao-source-py bridge as a source: Python
libraries described by manifests, reloaded when their code changes.

The bridge is its own workspace (`crates/guatiao-source-py`), built against this
Python; `GUATIAO_PY_BRIDGE` names the build, else its debug build is
looked for, else these skip and say so."""

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

_BRIDGE = Path(
    os.environ.get("GUATIAO_PY_BRIDGE")
    or native.REPO / "crates" / "guatiao-source-py" / "target" / "debug" / native.library_name("guatiao_source_py")
)
if not _BRIDGE.is_file():
    pytest.skip(f"no guatiao-source-py build at {_BRIDGE}", allow_module_level=True)

_EXAMPLES = native.REPO / "examples" / "python_provider"


@pytest.fixture()
def reg():
    registry = Registry("py-bridge", "0.1")
    yield registry
    # Served libraries close only when the registry lets them go: retire
    # every one, so the next test imports its modules afresh.
    for library in registry.libraries():
        registry.retire(library["key"])
    registry.collect()
    registry.close()


def _copy_greeter(into: Path, module: str) -> Path:
    """`py_greeter` under another module name, so its edits never touch
    the repository and its import never meets the one other tests made."""
    into.mkdir(parents=True, exist_ok=True)
    source = (_EXAMPLES / "py_greeter" / "py_greeter.py").read_text(encoding="utf-8")
    (into / f"{module}.py").write_text(source, encoding="utf-8")
    (into / f"{module}.guatiao.toml").write_text(
        f'type = "python"\nmodule = "{module}"\nkinds = ["greeter", "counter"]\npaths = ["."]\n',
        encoding="utf-8",
    )
    return into / f"{module}.py"


def test_the_examples_are_served_and_called(reg):
    name = reg.add_source_file(_BRIDGE, {"dirs": [str(_EXAMPLES)]})
    changed = reg.refresh()
    assert sorted(s["id"] for s in changed["loaded"]) == ["py_greeter", "py_tally"], changed
    assert changed["refused"] == []
    assert all(s["source"] == name for s in changed["loaded"])
    greeter = reg.bind("py_greeter_hello", Greeter)
    assert greeter.greet("ana").to_python() == {"greeting": "hello, ana"}
    with reg.create("py_tally_tally", {"start": 40}) as tally:
        counter = tally.bind(Counter)
        assert [counter.count(), counter.count()] == [41, 42]
    assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}


def test_an_edit_serves_new_code_while_an_instance_of_the_old_answers(reg, tmp_path):
    source = _copy_greeter(tmp_path / "edited", "py_edited")
    reg.add_source_file(_BRIDGE, {"dirs": [str(tmp_path)]})
    assert [s["id"] for s in reg.refresh()["loaded"]] == ["py_edited"]
    old = reg.create("py_edited_shouter", {"prefix": "hey"})
    assert old.bind(Greeter).greet("bo").to_python() == {"greeting": "hey BO"}
    first = sys.modules["py_edited"]

    time.sleep(0.05)
    text = source.read_text(encoding="utf-8")
    source.write_text(
        text.replace('GREETING = "hello"', 'GREETING = "howdy"').replace('MARK = ""', 'MARK = "!"'),
        encoding="utf-8",
    )
    changed = reg.refresh()
    assert [s["id"] for s in changed["loaded"]] == ["py_edited"]
    assert [s["id"] for s in changed["retired"]] == ["py_edited"]
    assert reg.bind("py_edited_hello", Greeter).greet("ana").to_python() == {"greeting": "howdy, ana"}
    assert old.bind(Greeter).greet("bo").to_python() == {"greeting": "hey BO"}, "the old code answers"
    with reg.create("py_edited_shouter", {"prefix": "hey"}) as new:
        assert new.bind(Greeter).greet("bo").to_python() == {"greeting": "hey BO!"}
    assert sys.modules["py_edited"] is not first, "imported afresh"

    assert reg.collect() == [], "the old instance holds its build open"
    old.close()
    assert reg.collect() == ["py_edited"]
    assert sys.modules["py_edited"] is not first, "closing the old build left the new module"


def test_a_deleted_manifest_retires_its_library(reg, tmp_path):
    shutil.copytree(_EXAMPLES / "py_tally", tmp_path / "py_tally")
    reg.add_source_file(_BRIDGE, {"dirs": [str(tmp_path)]})
    assert [s["id"] for s in reg.refresh()["loaded"]] == ["py_tally"]
    assert "py_tally" in sys.modules
    (tmp_path / "py_tally" / "py_tally.guatiao.toml").unlink()
    changed = reg.refresh()
    assert [s["id"] for s in changed["retired"]] == ["py_tally"]
    assert reg.collect() == ["py_tally"]
    assert "py_tally" not in sys.modules, "its modules left with it"


def test_what_cannot_be_opened_is_refused_in_words(reg, tmp_path):
    clash = tmp_path / "clash"
    clash.mkdir()
    (clash / "json.py").write_text('__version__ = "1"\n', encoding="utf-8")
    (clash / "clash.guatiao.toml").write_text('type = "python"\nmodule = "json"\nkinds = ["greeter"]\n', encoding="utf-8")
    (tmp_path / "broken.guatiao.toml").write_text("module = \n", encoding="utf-8")
    liar = tmp_path / "liar"
    _copy_greeter(liar, "py_liar")
    (liar / "py_liar.guatiao.toml").write_text(
        'type = "python"\nmodule = "py_liar"\nversion = "9.9.9"\nkinds = ["greeter", "counter"]\n', encoding="utf-8"
    )
    computed = tmp_path / "computed"
    computed.mkdir()
    (computed / "py_computed.py").write_text('__version__ = ".".join(["1", "0"])\n', encoding="utf-8")
    (computed / "computed.guatiao.toml").write_text(
        'type = "python"\nmodule = "py_computed"\nkinds = ["counter"]\n', encoding="utf-8"
    )
    reg.add_source_file(_BRIDGE, {"dirs": [str(tmp_path)]})
    refused = {r["id"]: r["why"] for r in reg.refresh()["refused"]}
    assert "`json` is already imported from" in refused["json"]
    assert any("broken.guatiao.toml" in why for why in refused.values())
    assert "version `9.9.9`" in refused["py_liar"] and "`1.0.0`" in refused["py_liar"]
    assert "put `version` in the manifest" in refused["py_computed"]
    assert reg.libraries() == []
