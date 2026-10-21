# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Sources from Python: a source file added and refreshed while the host
runs, and a watched directory."""

from __future__ import annotations

import ctypes
import os
import shutil
import sys

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native

sys.path.insert(0, str(native.GREETER_KIND_PY))

from greeter_kind import Counter  # noqa: E402

from guatiao import BadValue, NotFound, WrongKind  # noqa: E402
from guatiao.registry import Registry  # noqa: E402

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]


@pytest.fixture()
def shelf(tmp_path, request):
    # Each test maps its own copy, whose statics are its own shelf.
    path = native.build_c_source(tmp_path, f"py_shelf_{request.node.name[:20]}")
    if path is None:
        pytest.skip("no C compiler: the source written in C is not built")
    return path, ctypes.CDLL(str(path))


def test_a_source_file_serves_libraries_that_come_and_go(shelf):
    path, controls = shelf
    controls.shelf_closed.restype = ctypes.c_int
    with Registry("py-sources", "0.1") as reg:
        name = reg.add_source_file(path, {"ids": ["shelf_a", "shelf_b"]})
        assert name == str(path)
        assert reg.libraries() == [], "nothing opens before a refresh"

        first = reg.refresh()
        assert [s["id"] for s in first["loaded"]] == ["shelf_a", "shelf_b"]
        assert first["loaded"][0]["key"] == "shelf_a"
        counter = reg.bind("shelf_a_counter", Counter)
        assert counter.count() == 1
        assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}

        controls.shelf_take(1)
        gone = reg.refresh()
        assert [s["id"] for s in gone["retired"]] == ["shelf_b"]
        reg.collect()
        assert controls.shelf_closed(1) == 1

        controls.shelf_refuse(True)
        controls.shelf_put(2)
        refused = reg.refresh()
        assert refused["refused"] == [
            {"source": name, "id": "shelf_c", "why": "the shelf refuses, says C"}
        ]


def test_a_source_refuses_with_its_own_words(shelf):
    path, _ = shelf
    with Registry("py-sources", "0.1") as reg:
        with pytest.raises(BadValue) as refused:
            reg.add_source_file(path, {})
        assert "`ids`" in refused.value.message
        reg.add_source_file(path, {"ids": []})
        with pytest.raises(BadValue, match="already added"):
            reg.add_source_file(path, {"ids": []})


def test_a_file_that_is_no_source_is_refused():
    library = os.path.join(_TARGET_DEBUG, native.library_name("derived_greeter"))
    with Registry("py-sources", "0.1") as reg:
        with pytest.raises(WrongKind):
            reg.add_source_file(library)
        with pytest.raises(NotFound):
            reg.add_source_file(os.path.join(_TARGET_DEBUG, "no-such-file"))


def test_a_watched_directory_follows_its_files(tmp_path):
    watched = tmp_path / "watched"
    aside = tmp_path / "aside"
    watched.mkdir()
    aside.mkdir()
    library = os.path.join(_TARGET_DEBUG, native.library_name("hello_library"))
    with Registry("py-sources", "0.1") as reg:
        reg.watch_dir(watched, rules="kind=greeter")
        with pytest.raises(BadValue):
            reg.watch_dir(watched)
        assert reg.refresh()["loaded"] == []

        copy = watched / native.library_name("hello_py_watched")
        shutil.copy(library, copy)
        added = reg.refresh()
        assert [s["key"] for s in added["loaded"]] == ["hello_library"]

        # A mapped image cannot be deleted on Windows; moved aside, it is gone.
        os.replace(copy, aside / copy.name)
        removed = reg.refresh()
        assert [s["key"] for s in removed["retired"]] == ["hello_library"]
        assert reg.libraries() == []
