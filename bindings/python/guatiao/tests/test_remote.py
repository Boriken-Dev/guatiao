# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A Python host adding guatiao-source-remote as a source: a server in a child
process serving a greeter written in Rust and one written in C, called as
any provider is; and two identical devices, each described by a manifest,
served under their names.

guatiao-source-remote is a workspace member: its library and its server are
looked for beside the other libraries (`GUATIAO_LIBRARY`), and these skip,
saying so, when the workspace was not built."""

from __future__ import annotations

import platform
import re
import subprocess
import sys
from pathlib import Path

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native

sys.path.insert(0, str(native.GREETER_KIND_PY))

from greeter_kind import Greeter  # noqa: E402

from guatiao import BadValue, Registry  # noqa: E402

_DIR = Path(native.library_dir())
_REMOTE = _DIR / native.library_name("guatiao_source_remote")
_SERVER = _DIR / ("guatiao-remote-server.exe" if platform.system() == "Windows" else "guatiao-remote-server")
_DERIVED = _DIR / native.library_name("derived_greeter")
_KINDS = native.REPO / "examples" / "greeter_kind" / "include" / "greeter_kind.kinds.json"

for _needed in (_REMOTE, _SERVER, _DERIVED):
    if not _needed.is_file():
        pytest.skip(f"{_needed.name} is not built beside the libraries", allow_module_level=True)


@pytest.fixture()
def reg():
    registry = Registry("py-remote-host", "0.1")
    yield registry
    for library in registry.libraries():
        registry.retire(library["key"])
    registry.collect()
    registry.close()


def test_a_remote_rust_and_a_remote_c_greeter_are_called(reg, tmp_path):
    c_dir = native.build_c_greeter(tmp_path / "c")
    if c_dir is None:
        pytest.skip("no C compiler for the C greeter")
    c_greeter = c_dir / native.library_name("c_greeter")
    child = [str(_SERVER), "--stdio", "--kinds", str(_KINDS), "--load", str(_DERIVED), "--load", str(c_greeter)]
    reg.add_source_file(_REMOTE, {"child": child})
    changed = reg.refresh()
    assert changed["refused"] == [], changed
    assert sorted(s["id"] for s in changed["loaded"]) == ["c_greeter_library", "derived_greeter"], changed

    rust = reg.bind("derived_greeter_hello", Greeter)
    assert rust.greet("ana").to_python() == {"greeting": "hello, ana"}
    c = reg.bind("c_greeter", Greeter)
    assert c.greet("bo").to_python() == {"greeting": "hello from C, bo"}
    with pytest.raises(BadValue, match="nobody to greet, says C"):
        c.greet("")


@pytest.fixture()
def devices(tmp_path):
    """Two identical devices on loopback, each a server of the Rust
    greeter, and the key both hold; answers their ports."""
    (tmp_path / "link.key").write_text("a key both devices hold")
    servers = []
    ports = []
    for _ in range(2):
        server = subprocess.Popen(
            [str(_SERVER), "--listen", "tcp://127.0.0.1:0", "--security", "none", "--range", "127.0.0.1",
             "--key-file", str(tmp_path / "link.key"), "--kinds", str(_KINDS), "--load", str(_DERIVED)],
            stderr=subprocess.PIPE, text=True,
        )
        servers.append(server)
        assert server.stderr is not None
        serving = server.stderr.readline()
        found = re.search(r"serving tcp://127\.0\.0\.1:(\d+)", serving)
        assert found, serving
        ports.append(int(found.group(1)))
    yield ports
    for server in servers:
        server.kill()
        server.wait()


def test_peers_described_by_manifests_are_served_under_their_names(reg, tmp_path, devices):
    peers = tmp_path / "peers"
    peers.mkdir()
    for name, port in zip(("dev-a", "dev-b"), devices):
        (peers / f"{name}.guatiao.toml").write_text(
            f'type = "remote"\nname = "{name}"\nconnect = "tcp://127.0.0.1:{port}"\n'
            f'security = "none"\nkey_file = "../link.key"\n'
        )
    (peers / "inline.guatiao.toml").write_text(
        'type = "remote"\nname = "inline"\nconnect = "tcp://127.0.0.1:1"\nsecurity = "none"\nkey = "here"\n'
    )
    reg.add_source_file(_REMOTE, {"dirs": [str(peers)]})
    changed = reg.refresh()
    assert sorted(s["key"] for s in changed["loaded"]) == ["dev-a/derived_greeter", "dev-b/derived_greeter"], changed
    [refused] = changed["refused"]
    assert refused["id"].endswith("inline.guatiao.toml") and "`key` is a secret" in refused["why"], refused

    for name in ("dev-a", "dev-b"):
        greeter = reg.bind(f"{name}/derived_greeter_hello", Greeter)
        assert greeter.greet(name).to_python() == {"greeting": f"hello, {name}"}
    assert reg.refresh() == {"loaded": [], "retired": [], "skipped": [], "refused": []}

    (peers / "dev-b.guatiao.toml").unlink()
    assert [s["key"] for s in reg.refresh()["retired"]] == ["dev-b/derived_greeter"]
