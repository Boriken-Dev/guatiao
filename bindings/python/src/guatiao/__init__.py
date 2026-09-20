# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""ctypes bindings over the guatiao C ABI.

Importing this package never touches a native library: every native call
is resolved lazily, on first use, so a consumer that only needs the
constants or wants to catch ``LibraryNotFound`` itself can import freely.
"""

from __future__ import annotations

from .errors import (
    AllocFailed,
    BadValue,
    Busy,
    Gone,
    GuatiaoError,
    Internal,
    NotFound,
    NullArgument,
    Unsupported,
    WrongKind,
)
from ._lib import LibraryNotFound, MissingSymbol
from .registry import Instance, Registry
from .value import ABSENT, List, Map, Ref, Value

__version__ = "0.1.0"

__all__ = [
    "__version__",
    "ABSENT",
    "Value",
    "Ref",
    "Map",
    "List",
    "Registry",
    "Instance",
    "GuatiaoError",
    "BadValue",
    "AllocFailed",
    "WrongKind",
    "NotFound",
    "NullArgument",
    "Gone",
    "Unsupported",
    "Busy",
    "Internal",
    "LibraryNotFound",
    "MissingSymbol",
]
