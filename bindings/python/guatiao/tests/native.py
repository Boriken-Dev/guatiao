# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Builds the repository's C greeter into a directory of its own, for the
tests that call a library written in C."""

from __future__ import annotations

import platform
import shutil
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
GREETER_KIND_PY = REPO / "examples" / "greeter_kind" / "python"


def compiler() -> "str | None":
    """The first C compiler on this machine, as the Rust tests look for one."""
    for name in ("clang", r"C:\Program Files\LLVM\bin\clang.exe", "cc", "gcc"):
        found = shutil.which(name) if not Path(name).is_absolute() else (name if Path(name).exists() else None)
        if found:
            return found
    return None


def library_name(base: str) -> str:
    system = platform.system()
    if system == "Windows":
        return f"{base}.dll"
    if system == "Darwin":
        return f"lib{base}.dylib"
    return f"lib{base}.so"


def _build(source: str, into: Path, name: str) -> "Path | None":
    """`source` (under the crate's C tests) built into `into` as `name`, or
    `None` without a compiler. A compile error fails loudly: the header
    changed under it."""
    cc = compiler()
    if cc is None:
        return None
    into.mkdir(parents=True, exist_ok=True)
    out = into / library_name(name)
    command = [cc, "-shared", "-std=c11", "-Wall", "-Wextra", "-Werror"]
    if platform.system() != "Windows":
        command.append("-fPIC")
    command += [
        "-I", str(REPO / "crates" / "guatiao" / "include"),
        "-I", str(REPO / "examples" / "greeter_kind" / "include"),
        "-o", str(out),
        str(REPO / "crates" / "guatiao" / "tests" / "c_consumer" / source),
    ]
    done = subprocess.run(command, capture_output=True, text=True)
    if done.returncode != 0:
        raise RuntimeError(f"{source} did not compile:\n{done.stderr}")
    return out


def build_c_greeter(into: Path) -> "Path | None":
    """The greeter written in C, in a directory of its own."""
    built = _build("greeter_in_c.c", into, "c_greeter")
    return None if built is None else into


def build_c_source(into: Path, name: str = "c_shelf") -> "Path | None":
    """The source written in C (a shelf of counters), as a file."""
    return _build("source_in_c.c", into, name)
