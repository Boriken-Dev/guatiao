# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""Points the test suite at a workspace build when nothing else does."""

from __future__ import annotations

import os
from pathlib import Path

_REPO_ROOT = Path(__file__).resolve().parents[3]


def _default_library_dir() -> Path:
    return _REPO_ROOT / "target" / "debug"


os.environ.setdefault("GUATIAO_LIBRARY", str(_default_library_dir()))
