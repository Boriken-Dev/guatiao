# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The repository's Python examples, run as their READMEs say."""

from __future__ import annotations

import os
import subprocess
import sys

import pytest

from guatiao import _lib

try:
    _lib.core()
except _lib.LibraryNotFound:
    pytest.skip("no guatiao library built", allow_module_level=True)

import native

_TARGET_DEBUG = os.environ["GUATIAO_LIBRARY"]
_SRC = native.REPO / "bindings" / "python" / "src"


def _run(script, *args) -> str:
    env = dict(os.environ)
    env["PYTHONPATH"] = os.pathsep.join([str(_SRC), env.get("PYTHONPATH", "")])
    done = subprocess.run(
        [sys.executable, str(script), *map(str, args)],
        capture_output=True,
        text=True,
        env=env,
        timeout=120,
    )
    assert done.returncode == 0, f"stdout:\n{done.stdout}\nstderr:\n{done.stderr}"
    return done.stdout


def test_the_python_host_calls_every_greeter_it_finds(tmp_path):
    host = native.REPO / "examples" / "python_host" / "host.py"
    dirs = [_TARGET_DEBUG]
    c_greeter = native.build_c_greeter(tmp_path / "c-greeter")
    if c_greeter is not None:
        dirs.append(c_greeter)
    out = _run(host, *dirs)
    assert "greet: {'greeting': 'hello, ana'}" in out
    assert "conversation: 2 turns" in out
    assert "an empty configuration is refused" in out
    assert "greet: {'greeting': 'hey ANA'}" in out
    assert "not this greeter kind" in out
    assert "unloaded derived_greeter" in out
    assert out.rstrip().endswith("python objects a library can still reach: 0")
    if c_greeter is not None:
        assert "greet: {'greeting': 'hello from C, ana'}" in out
        assert "retired c_greeter_library, still mapped" in out
