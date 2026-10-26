# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""`python -m guatiao.kinds DOCUMENT -o MODULE.py [--check]`: writes a
description's kinds as typed `Protocol` classes, or with `--check` says
whether the module already holds what it would write (exit 1 if not)."""

from __future__ import annotations

import argparse
import pathlib
import sys
from typing import Optional, Sequence

from ._description import generate


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        prog="python -m guatiao.kinds",
        description="Write the kinds of a description document as typed Protocol classes.",
    )
    parser.add_argument("document", help="the kinds document, <crate>.kinds.json")
    parser.add_argument("-o", "--out", required=True, help="the module to write")
    parser.add_argument("--check", action="store_true", help="compare instead of writing")
    given = parser.parse_args(argv)
    document = pathlib.Path(given.document).read_text(encoding="utf-8")
    command = f"python -m guatiao.kinds {given.document} -o {given.out}"
    text = generate(document, source=given.document, command=command)
    out = pathlib.Path(given.out)
    if given.check:
        current = out.read_text(encoding="utf-8") if out.exists() else None
        if current != text:
            print(f"{out} is not what {given.document} writes: regenerate it", file=sys.stderr)
            return 1
        return 0
    out.write_bytes(text.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main())
