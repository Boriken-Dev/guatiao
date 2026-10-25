# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""The Python half of guatiao-source-py: imports the library a manifest names,
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


def _module_file(module_name, paths):
    """Where `module_name`'s source is under `paths`, found without
    importing anything."""
    from importlib.machinery import PathFinder

    parts = module_name.split(".")
    search = list(paths)
    spec = None
    for at in range(len(parts)):
        spec = PathFinder.find_spec(".".join(parts[: at + 1]), search)
        if spec is None:
            return None
        search = list(spec.submodule_search_locations or [])
    return spec.origin if spec is not None else None


def literal_version(module_name, paths):
    """`__version__` as a literal in the module's source, read with `ast`
    and not imported."""
    import ast

    origin = _module_file(module_name, paths)
    if not origin or not origin.endswith(".py"):
        raise LookupError(f"no source for `{module_name}` under {', '.join(paths)}")
    with open(origin, "rb") as source:
        tree = ast.parse(source.read(), origin)
    for node in tree.body:
        if isinstance(node, ast.Assign) and any(
            isinstance(t, ast.Name) and t.id == "__version__" for t in node.targets
        ):
            if isinstance(node.value, ast.Constant) and isinstance(node.value.value, str):
                return node.value.value
            break
    raise ValueError(
        f"`{module_name}.__version__` is not a literal to read without importing it; "
        "put `version` in the manifest"
    )


def open_library(module_name, version, kinds, paths):
    """Imports `module_name` afresh from `paths`, the module being the
    library, and answers the address of its library descriptor and a
    handle that `close_library` takes."""
    roots = [_real(p) for p in paths]
    top = module_name.split(".")[0]
    held = sys.modules.get(top)
    if held is not None:
        where = getattr(held, "__file__", None)
        if not _under(where, roots):
            raise ImportError(
                f"`{top}` is already imported from {where or 'the interpreter itself'}, "
                f"and the library would import it from {', '.join(paths)}"
            )
    # A fresh import: what an open library imported keeps its own modules,
    # and a new one gets new ones.
    _drop_modules(roots)
    _use_paths(paths)
    importlib.invalidate_caches()
    try:
        from guatiao.provide import library_of

        library = library_of(importlib.import_module(module_name))
        if library.version != version:
            raise ValueError(
                f"the manifest says version `{version}`, and `{module_name}.__version__` is `{library.version}`"
            )
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
