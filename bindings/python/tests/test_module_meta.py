# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A module library's meta: its installed distribution's metadata, its
conventional attributes, and `_meta_`, merged in that order. Needs no
native library."""

from __future__ import annotations

import importlib
import sys
from pathlib import Path
from typing import Any, Iterator

import pytest

from guatiao._module_meta import meta_of

_METADATA = """Metadata-Version: 2.4
Name: meta-probe
Version: 1.2.3
Summary: A probe of what a distribution says.
Author: Ana
Author-email: Ana <ana@example.org>
License-Expression: MPL-2.0
Home-page: https://example.org
Project-URL: Source, https://example.org/src
License-File: LICENSE
"""


@pytest.fixture()
def installed(tmp_path: Path) -> Iterator[Any]:
    """`meta_probe`, a module whose distribution is installed on sys.path."""
    (tmp_path / "meta_probe.py").write_text(
        '"""The probe module.\n\nIts second paragraph."""\n__author__ = "Bo"\n', encoding="utf-8"
    )
    info = tmp_path / "meta_probe-1.2.3.dist-info"
    (info / "licenses").mkdir(parents=True)
    (info / "METADATA").write_text(_METADATA, encoding="utf-8")
    (info / "top_level.txt").write_text("meta_probe\n", encoding="utf-8")
    (info / "RECORD").write_text("meta_probe.py,,\n", encoding="utf-8")
    (info / "licenses" / "LICENSE").write_text("The licence text.\n", encoding="utf-8")
    sys.path.insert(0, str(tmp_path))
    importlib.invalidate_caches()
    try:
        yield importlib.import_module("meta_probe")
    finally:
        sys.path.remove(str(tmp_path))
        sys.modules.pop("meta_probe", None)


def test_an_installed_module_carries_its_distributions_metadata(installed: Any) -> None:
    meta = meta_of(installed)
    assert meta["summary"] == "A probe of what a distribution says."
    assert meta["license"] == "MPL-2.0"
    assert meta["urls"] == {"Source": "https://example.org/src", "Home-page": "https://example.org"}
    assert meta["notices"] == [{"component": "meta-probe", "text": "The licence text.\n", "format": "text"}]
    # The module's own attributes come after, and win.
    assert meta["description"] == "The probe module."
    assert meta["authors"] == ["Bo"]


def test_meta_overrides_both(installed: Any) -> None:
    installed._meta_ = {"license": "MIT", "notices": []}
    meta = meta_of(installed)
    assert meta["license"] == "MIT"
    assert meta["notices"] == []
    assert meta["summary"] == "A probe of what a distribution says."


def test_a_module_not_installed_says_only_what_it_holds() -> None:
    import types

    module = types.ModuleType("loose_module_nowhere")
    module.__doc__ = "Loose.\n\nDetail."
    module.__license__ = "Apache-2.0"
    module.__copyright__ = "2026 someone"
    assert meta_of(module) == {
        "description": "Loose.",
        "license": "Apache-2.0",
        "copyright": "2026 someone",
    }
    module._meta_ = ["not", "a", "map"]  # type: ignore[attr-defined]
    with pytest.raises(TypeError, match="_meta_"):
        meta_of(module)
