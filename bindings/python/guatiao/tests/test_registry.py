# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`Registry` against a real `--all-features` build."""

from __future__ import annotations

import ctypes
import hashlib
import json
import os

import pytest

from guatiao import _lib, errors

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

from guatiao.registry import Registry

import native

_TARGET_DEBUG = native.library_dir()


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


def test_the_import_search_is_the_librarys_until_the_host_names_path():
    from guatiao import ImportSearch

    with Registry("py-tests", "0.1") as reg:
        assert reg.import_search() is ImportSearch.LIBRARY
        reg.set_import_search(ImportSearch.PATH)
        assert reg.import_search() is ImportSearch.PATH
        with pytest.raises(errors.BadValue):
            reg.set_import_search(2)  # type: ignore[arg-type]
        assert reg.import_search() is ImportSearch.PATH


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


def test_a_providers_meta_is_a_copy_carrying_its_form():
    with _scanned() as reg:
        meta = reg.provider_meta("derived_greeter_shouter")
        assert meta is not None
        with meta:
            form = meta.as_map()["form"].to_python()
            assert [s["id"] for s in form["sections"]] == ["words", "volume"]

        assert reg.provider_meta("derived_greeter_hello") is None
        with pytest.raises(errors.NotFound):
            reg.provider_meta("nobody")


def test_the_catalog_lists_every_provider_of_a_directory_and_builds_none(tmp_path):
    built = native.build_catalog_library(tmp_path)
    if built is None:
        pytest.skip("no C compiler found")
    with Registry("catalog-host", "1.0") as reg:
        report = reg.scan_dir(str(tmp_path))
        assert report["failed"] == [] and len(report["loaded"]) == 1, report
        catalog = {entry["id"]: entry for entry in reg.catalog()}
        assert sorted(catalog) == [
            "catalog_configured",
            "catalog_implied",
            "catalog_plain",
            "catalog_unavailable",
        ]
        digest = "sha256:" + hashlib.sha256(built.read_bytes()).hexdigest()

        configured = catalog["catalog_configured"]
        assert configured["kinds"] == ["counter"]
        assert configured["display_name"] == "Configured, with a form"
        assert configured["version"] == "2.0"
        assert configured["library"] == "catalog_library"
        assert configured["library_version"] == "1.0"
        assert configured["digest"] == digest
        assert configured["available"] is True and "reason" not in configured
        assert configured["config"]["required"] == ["host", "port"]
        assert configured["meta"]["form"]["sections"] == [{"id": "net", "title": "Network"}]

        implied = catalog["catalog_implied"]
        assert implied["version"] == "1.0"
        assert "meta" not in implied
        assert implied["config"]["properties"]["token"]["x-sensitive"] is True

        unavailable = catalog["catalog_unavailable"]
        assert unavailable["available"] is False
        assert unavailable["reason"] == "no device here"
        assert "config" not in unavailable and not unavailable["has_config"]
        assert catalog["catalog_plain"]["available"] is True

        # Listing asked the library for nothing: its count says so, and so
        # does its refusal of the first instance asked for afterwards.
        library = ctypes.CDLL(str(built))
        assert library.catalog_built() == 0
        with pytest.raises(errors.GuatiaoError) as refused:
            reg.create("catalog_configured", {"host": "localhost", "port": 8080})
        assert refused.value.message == "built nothing: the first create this library was asked"
        assert library.catalog_built() == 1


def test_a_provider_carries_its_kinds_descriptions():
    committed = json.loads(
        (native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json").read_text(
            encoding="utf-8"
        )
    )
    with _scanned() as reg:
        document = reg.kind_descriptions("derived_greeter_hello")
        assert document is not None
        assert list(document) == ["kinds"]
        names = [k["name"] for k in document["kinds"]]
        assert names == ["greeter", "listener", "conversation", "counter", "wide"]
        by_name = {k["name"]: k for k in committed["kinds"]}
        for kind in document["kinds"]:
            assert kind == by_name[kind["name"]]

        assert reg.kind_descriptions("hello_library_greeter") is None
        with pytest.raises(errors.NotFound):
            reg.kind_descriptions("nobody")


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


def test_a_library_refuses_to_unload_while_an_instance_is_alive():
    with _scanned() as reg:
        instance = reg.create("derived_greeter_shouter", {"prefix": "hey"})
        with pytest.raises(errors.Busy):
            reg.unload("derived_greeter")
        assert any(lib["id"] == "derived_greeter" for lib in reg.libraries())
        instance.close()
        reg.unload("derived_greeter")
        assert not any(lib["id"] == "derived_greeter" for lib in reg.libraries())


def test_a_retired_library_is_collected_once_nothing_of_it_is_alive():
    with _scanned() as reg:
        assert reg.collect() == [], "nothing is retired"
        reg.retire("derived_greeter")
        assert reg.collect() == ["derived_greeter"]
        assert reg.collect() == [], "closed once"


def test_both_symbols_are_declared():
    lib = _lib.core()
    assert "guatiao_registry_retire" not in lib.symbols_missing()
    assert "guatiao_registry_unload" not in lib.symbols_missing()
    assert "guatiao_registry_unload_unchecked" not in lib.symbols_missing()
    assert "guatiao_registry_collect" not in lib.symbols_missing()


def test_closed_registry_raises_on_use():
    reg = Registry("py-tests", "0.1")
    reg.close()
    with pytest.raises(ValueError):
        reg.libraries()
    reg.close()  # idempotent


def test_an_instance_closes_after_its_library_was_retired():
    with _scanned() as reg:
        instance = reg.create("derived_greeter_shouter", {"prefix": "hey"})
        reg.retire("derived_greeter")
        assert reg.collect() == [], "the instance holds its library open"
        instance.close()
        assert reg.collect() == ["derived_greeter"]
