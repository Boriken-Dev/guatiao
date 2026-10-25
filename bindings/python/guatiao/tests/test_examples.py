# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The repository's Python examples, run as their READMEs say."""

from __future__ import annotations

import os
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

_TARGET_DEBUG = native.library_dir()
_SRC = native.REPO / "bindings" / "python" / "guatiao" / "src"


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


def test_the_python_host_serves_python_greeters_through_the_bridge() -> None:
    bridge = Path(
        os.environ.get("GUATIAO_PY_BRIDGE")
        or native.REPO / "crates" / "guatiao-source-py" / "target" / "debug" / native.library_name("guatiao_source_py")
    )
    if not bridge.is_file():
        pytest.skip(f"no guatiao-source-py build at {bridge}")
    host = native.REPO / "examples" / "python_host" / "host.py"
    out = _run(
        host,
        _TARGET_DEBUG,
        "--python-source",
        bridge,
        "--python-dir",
        native.REPO / "examples" / "python_provider",
    )
    assert "served py_greeter from Python" in out
    assert "py_greeter_hello (py_greeter 1.0.0)" in out
    assert "conversation: 2 turns" in out
    assert out.rstrip().endswith("python objects a library can still reach: 0")


def test_the_python_host_calls_greeters_written_in_cpp() -> None:
    cpp = os.environ.get("GUATIAO_CPP_LIBRARY")
    if not cpp:
        pytest.skip("GUATIAO_CPP_LIBRARY names no directory holding the C++ greeters")
    host = native.REPO / "examples" / "python_host" / "host.py"
    out = _run(host, cpp)
    assert "cpp_greeter_hello (cpp_greeter 1.0.0)" in out
    assert "greet: {'greeting': 'hello, ana'}" in out
    assert "greet refused: BadValue: nobody to greet" in out
    assert "shout: HELLO, ANA" in out
    assert "check: True, lent map now {'name': 'bo', 'greeting': 'hello, bo'}" in out
    assert "greet_from: {'greeting': 'hello, cy'}" in out
    assert "greet_into: the lent listener heard ['hello, dee']" in out
    assert "conversation: 2 turns, transcript b'hi\\nagain', listener heard ['hi', 'again']" in out
    assert "an empty configuration is refused" in out
    assert "greet: {'greeting': 'hey ANA'}" in out
    # A C++ library agrees to be unmapped once nothing it served is alive.
    assert "unloaded cpp_greeter" in out
    assert out.rstrip().endswith("python objects a library can still reach: 0")


def test_the_python_host_calls_greeters_written_in_fortran() -> None:
    fortran = os.environ.get("GUATIAO_FORTRAN_LIBRARY")
    if not fortran:
        pytest.skip("GUATIAO_FORTRAN_LIBRARY names no directory holding the Fortran greeters")
    host = native.REPO / "examples" / "python_host" / "host.py"
    out = _run(host, fortran)
    assert "fortran_greeter_hello (fortran_greeter 1.0.0)" in out
    assert "greet: {'greeting': 'hello, ana'}" in out
    assert "shout: HELLO, ANA" in out
    assert "check: True, lent map now {'name': 'bo', 'greeting': 'hello, bo'}" in out
    assert "greet_from: {'greeting': 'hello, cy'}" in out
    assert "greet_into: the lent listener heard ['hello, dee']" in out
    assert "conversation: 2 turns, transcript b'hi\\nagain', listener heard ['hi', 'again']" in out
    assert "greet: {'greeting': 'hey ANA'}" in out
    # A Fortran library is never unmapped: retired instead.
    assert "retired fortran_greeter, still mapped" in out
    assert out.rstrip().endswith("python objects a library can still reach: 0")
