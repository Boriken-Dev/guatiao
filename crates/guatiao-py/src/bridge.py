# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The Python half of guatiao-py: imports the library a manifest names,
checks it is the one the manifest describes, and lets it go again.

Runs inside the one interpreter the bridge keeps for the program. Every
function here is called holding the GIL.
"""

import importlib
import os
import sys

_open = {}
_path_users = {}
_next = [1]


def _real(path):
    return os.path.normcase(os.path.realpath(path))


def _under(file, roots):
    if not file:
        return False
    real = _real(file)
    return any(real == root or real.startswith(root + os.sep) for root in roots)


def _use_paths(paths):
    for path in reversed(paths):
        users = _path_users.get(path, 0)
        if users == 0 and path not in sys.path:
            sys.path.insert(0, path)
        _path_users[path] = users + 1


def _release_paths(paths):
    for path in paths:
        users = _path_users.get(path, 0) - 1
        if users <= 0:
            _path_users.pop(path, None)
            if path in sys.path:
                sys.path.remove(path)
        else:
            _path_users[path] = users


def _drop_modules(roots, only=None):
    for name, module in list(sys.modules.items()):
        if only is not None and only.get(name) is not module:
            continue
        if _under(getattr(module, "__file__", None), roots):
            del sys.modules[name]


def setup(paths, bridge):
    """Puts the source's shared paths first on `sys.path`, and points the
    binding at the bridge's own exports unless the host chose a library."""
    for path in reversed(paths):
        if path not in sys.path:
            sys.path.insert(0, path)
    import guatiao

    if bridge and "GUATIAO_LIBRARY" not in os.environ:
        try:
            guatiao.use_library(bridge)
        except RuntimeError:
            # The host is Python and its binding already has a library.
            pass


def open_library(library_id, kinds, entry, paths):
    """Imports `entry` afresh from `paths` and answers the address of its
    library descriptor and a handle that `close_library` takes."""
    module_name, _, attr = entry.partition(":")
    if not module_name or not attr:
        raise ValueError(f"the entry `{entry}` is not `module:attribute`")
    roots = [_real(p) for p in paths]
    top = module_name.split(".")[0]
    held = sys.modules.get(top)
    if held is not None:
        where = getattr(held, "__file__", None)
        if not _under(where, roots):
            raise ImportError(
                f"`{top}` is already imported from {where or 'the interpreter itself'}, "
                f"and {library_id} would import it from {', '.join(paths)}"
            )
    # A fresh import: what an open library imported keeps its own modules,
    # and a new one gets new ones.
    _drop_modules(roots)
    _use_paths(paths)
    importlib.invalidate_caches()
    try:
        library = importlib.import_module(module_name)
        for part in attr.split("."):
            library = getattr(library, part)
        from guatiao.provide import Library

        if not isinstance(library, Library):
            raise TypeError(f"`{entry}` is a {type(library).__name__}, not a guatiao.provide Library")
        if library.id != library_id:
            raise ValueError(f"the manifest names `{library_id}`, and `{entry}` is `{library.id}`")
        if sorted(set(kinds)) != sorted(set(library.kinds)):
            raise ValueError(
                f"the manifest's kinds {sorted(set(kinds))} are not the library's {sorted(set(library.kinds))}"
            )
    except BaseException:
        _drop_modules(roots)
        _release_paths(paths)
        raise
    modules = {
        name: module
        for name, module in sys.modules.items()
        if _under(getattr(module, "__file__", None), roots)
    }
    handle = _next[0]
    _next[0] += 1
    _open[handle] = (library, modules, roots, paths)
    return library.info_address, handle


def close_library(handle):
    """Lets the library go: its own instances, the modules its open
    imported (never a newer copy), and its paths once nothing else uses
    them."""
    library, modules, roots, paths = _open.pop(handle)
    try:
        library.close()
    finally:
        _drop_modules(roots, only=modules)
        _release_paths(paths)
