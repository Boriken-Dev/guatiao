# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""ctypes bindings over the guatiao C ABI.

Importing this package never touches a native library: every native call
is resolved lazily, on first use, so a consumer that only needs the
constants or wants to catch ``LibraryNotFound`` itself can import freely.
"""

from __future__ import annotations

__version__ = "0.1.0"

__all__ = ["__version__"]
