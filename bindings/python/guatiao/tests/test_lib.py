# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`_lib.py` against a real build: no missing symbol, alloc resolves."""

from __future__ import annotations

import os
from pathlib import Path

import pytest

from guatiao import _lib, ffi

import native


def test_core_loads_and_declares_every_export():
    lib = _lib.core()
    assert lib.symbols_missing() == frozenset()


def test_default_alloc_is_filled_once_and_cached():
    lib = _lib.core()
    a1 = lib.alloc
    a2 = lib.alloc
    assert a1 is a2
    assert a1.struct_size == 40
    assert bool(a1.alloc)
    assert bool(a1.free)


def _built(basename: str) -> Path:
    path = Path(native.library_dir()) / ffi.dll_filename(basename)
    if not path.is_file():
        pytest.skip(f"no {path.name} built")
    return path


def test_a_surfaces_own_variable_names_a_file_whatever_it_is_called(monkeypatch):
    intake = _built("guatiao_intake")
    monkeypatch.setenv("GUATIAO_SERDE_LIBRARY", str(intake))
    found = ffi.find_library(
        "guatiao_serde", variable="GUATIAO_SERDE_LIBRARY", symbol="guatiao_json_parse_ex"
    )
    assert found == intake


def test_a_surfaces_own_variable_names_a_directory(monkeypatch):
    serde = _built("guatiao_serde")
    monkeypatch.setenv("GUATIAO_SERDE_LIBRARY", str(serde.parent))
    monkeypatch.setenv("GUATIAO_LIBRARY", str(serde.parent / "nowhere"))
    found = ffi.find_library(
        "guatiao_serde", variable="GUATIAO_SERDE_LIBRARY", symbol="guatiao_json_parse_ex"
    )
    assert found == serde


def test_a_shared_file_carrying_the_surface_serves_it(monkeypatch):
    serde = _built("guatiao_serde")
    monkeypatch.delenv("GUATIAO_SERDE_LIBRARY", raising=False)
    monkeypatch.setenv("GUATIAO_LIBRARY", str(serde))
    found = ffi.find_library(
        "guatiao_serde", variable="GUATIAO_SERDE_LIBRARY", symbol="guatiao_json_parse_ex"
    )
    assert found == serde
    # The core, which every guatiao library carries, is taken from it too.
    assert ffi.find_library("guatiao") == serde


def test_a_shared_file_without_the_surface_points_at_its_sibling(monkeypatch):
    core = _built("guatiao")
    serde = _built("guatiao_serde")
    monkeypatch.delenv("GUATIAO_SERDE_LIBRARY", raising=False)
    monkeypatch.setenv("GUATIAO_LIBRARY", str(core))
    found = ffi.find_library(
        "guatiao_serde", variable="GUATIAO_SERDE_LIBRARY", symbol="guatiao_json_parse_ex"
    )
    assert found == serde


def test_not_found_names_every_place_tried(monkeypatch):
    monkeypatch.setenv("GUATIAO_SERDE_LIBRARY", "/nowhere/at/all")
    monkeypatch.setenv("GUATIAO_LIBRARY", "/nowhere/either")
    with pytest.raises(ffi.LibraryNotFound) as excinfo:
        ffi.find_library("not_a_real_guatiao_library", variable="GUATIAO_SERDE_LIBRARY")
    message = str(excinfo.value)
    assert "GUATIAO_SERDE_LIBRARY" in message
    assert "GUATIAO_LIBRARY" in message
    assert "find_library" in message


def test_unknown_basename_reports_where_it_looked(monkeypatch):
    # A file named here would serve any base name: the core's rule.
    monkeypatch.setenv("GUATIAO_LIBRARY", native.library_dir())
    with pytest.raises(_lib.LibraryNotFound) as excinfo:
        _lib.resolve("not_a_real_guatiao_library")
    message = str(excinfo.value)
    assert "GUATIAO_LIBRARY" in message
    assert "find_library" in message
