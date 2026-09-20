# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`_lib.py` against a real build: no missing symbol, alloc resolves."""

from __future__ import annotations

import pytest

from guatiao import _lib


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


def test_serde_and_form_load_with_all_features_built():
    assert _lib.serde().symbols_missing() == frozenset()
    assert _lib.form().symbols_missing() == frozenset()


def test_unknown_basename_reports_the_three_places():
    with pytest.raises(_lib.LibraryNotFound) as excinfo:
        _lib.resolve("not_a_real_guatiao_library")
    message = str(excinfo.value)
    assert "GUATIAO_LIBRARY" in message
    assert "find_library" in message
    assert "_native" in message
