# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""An application's own library carrying the surfaces, mapped beside
plugins that link guatiao themselves.

Each of those files exports `guatiao_*`. Mapped with `RTLD_LOCAL` each
keeps its own; with `RTLD_GLOBAL` the application's become the process's
first definitions. Both must leave the host and the plugins working. Run
when `GUATIAO_LIBRARY` names the application's file (`examples/app_library`).
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

import pytest

import native

_APP = os.environ.get("GUATIAO_LIBRARY", "")

pytestmark = pytest.mark.skipif(
    os.path.isdir(_APP) or not os.path.basename(_APP).startswith(("myapp", "libmyapp")),
    reason="GUATIAO_LIBRARY does not name the application library (examples/app_library)",
)


@pytest.mark.skipif(sys.platform == "win32", reason="dlopen modes are a Unix loader's")
@pytest.mark.parametrize("mode", ["RTLD_LOCAL", "RTLD_GLOBAL"])
def test_the_application_library_and_plugins_share_a_process(mode):
    here = Path(__file__).resolve().parent
    env = dict(os.environ, PYTHONPATH=os.pathsep.join([str(here), os.environ.get("PYTHONPATH", "")]))
    done = subprocess.run(
        [sys.executable, str(here / "app_library_child.py"), mode],
        env=env, capture_output=True, text=True, timeout=120,
    )
    assert done.returncode == 0, done.stdout + done.stderr
    assert done.stdout.strip().endswith("ok")


def test_the_application_library_serves_the_core():
    from guatiao import _lib

    assert os.path.samefile(_lib.core().path, _APP)
    assert native.library_dir() == os.path.dirname(_APP)
