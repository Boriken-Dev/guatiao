# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`Registry` against a real `--all-features` build."""

from __future__ import annotations

import os

import pytest

from guatiao import _lib, errors

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

from guatiao.registry import Registry

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]


def _scanned() -> Registry:
    reg = Registry("py-tests", "0.1")
    reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
    return reg


def test_scan_dir_rules_finds_the_greeter_providers():
    reg = Registry("py-tests", "0.1")
    try:
        report = reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        loaded = report.get("loaded", [])
        assert any("derived_greeter" in path for path in loaded)
        assert any("hello_library" in path for path in loaded)

        providers = reg.providers("greeter")
        ids = {p["id"] for p in providers}
        assert "derived_greeter_hello" in ids
        assert "hello_library_greeter" in ids
    finally:
        reg.close()


def test_best_provider_carries_the_documented_fields():
    with _scanned() as reg:
        best = reg.best("greeter")
        for field in (
            "key",
            "id",
            "version",
            "library",
            "display_name",
            "from",
            "kinds",
            "has_config",
            "vtable_size",
        ):
            assert field in best, field
        assert "greeter" in best["kinds"]


def test_why_not_explains_an_unclaimed_kind():
    with _scanned() as reg:
        why = reg.why_not("nothing-claims-this-kind")
        assert why["available"] is False
        assert why["why"] == "nothing-claims-it"


def test_provider_table_and_ctx_for_a_known_provider():
    with _scanned() as reg:
        # hello_library's table is hand-written straight into `vtable`;
        # a derived multi-kind provider files its tables under
        # `tables` instead, which this single-vtable accessor does not
        # read (see AGENTS.md "Only a per-kind table..." on the crate).
        table, size, ctx = reg.provider_table("hello_library_greeter")
        assert table
        assert size > 0


def test_create_a_configured_provider_and_close_it():
    with _scanned() as reg:
        instance = reg.create("derived_greeter_shouter", {"prefix": "hey"})
        try:
            assert instance.ctx
        finally:
            instance.close()
        instance.close()  # idempotent


def test_provider_config_is_a_borrowed_schema():
    with _scanned() as reg:
        schema = reg.provider_config("derived_greeter_shouter")
        assert schema is not None
        assert schema.tag == 7  # GUATIAO_MAP
        assert reg.provider_config("derived_greeter_hello") is None


def test_retire_takes_a_librarys_providers_and_frees_its_key():
    with _scanned() as reg:
        before = {p["id"] for p in reg.providers("greeter")}
        assert "derived_greeter_hello" in before

        reg.retire("derived_greeter")
        after = {p["id"] for p in reg.providers("greeter")}
        assert "derived_greeter_hello" not in after
        assert "hello_library_greeter" in after, "only that library left"
        assert not any(
            lib["id"] == "derived_greeter" for lib in reg.libraries()
        )

        # The key is free: a retired library is not "already-loaded".
        report = reg.scan_dir(_TARGET_DEBUG, rules="kind=greeter")
        assert any("derived_greeter" in path for path in report.get("loaded", []))


def test_retire_takes_the_library_key_not_a_provider_key():
    with _scanned() as reg:
        with pytest.raises(errors.NotFound):
            reg.retire("derived_greeter_hello")
        assert any(lib["id"] == "derived_greeter" for lib in reg.libraries())


def test_unload_unmaps_a_library():
    with _scanned() as reg:
        reg.unload("derived_greeter")
        assert not any(
            lib["id"] == "derived_greeter" for lib in reg.libraries()
        )
        assert "derived_greeter_hello" not in {
            p["id"] for p in reg.providers("greeter")
        }
        with pytest.raises(errors.NotFound):
            reg.unload("derived_greeter")


def test_both_symbols_are_declared():
    lib = _lib.core()
    assert "guatiao_registry_retire" not in lib.symbols_missing()
    assert "guatiao_registry_unload" not in lib.symbols_missing()


def test_closed_registry_raises_on_use():
    reg = Registry("py-tests", "0.1")
    reg.close()
    with pytest.raises(ValueError):
        reg.libraries()
    reg.close()  # idempotent
