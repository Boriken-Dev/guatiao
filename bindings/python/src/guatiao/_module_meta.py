# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""What a module library says about itself, as its `meta` map.

Three sources, merged in this order, each key of a later one winning:

1. the metadata of the installed distribution the module belongs to:
   `summary`, `license`, `authors`, `urls`, and a `notices` entry for each
   licence file it carries (the notices convention every host reads);
2. the module's conventional attributes: `description` (the first
   paragraph of its docstring), `authors` (`__author__`), `license`
   (`__license__`), `copyright` (`__copyright__`);
3. `_meta_`, a mapping of plain data the module may define.
"""

from __future__ import annotations

import importlib.metadata as metadata
import inspect
import types
from collections.abc import Mapping
from typing import Any, Dict, List, Optional


def _top_level(dist: Any) -> List[str]:
    found: List[str] = []
    text = dist.read_text("top_level.txt")
    if text:
        found += [line.strip() for line in text.splitlines() if line.strip()]
    for file in dist.files or []:
        parts = file.parts
        if len(parts) > 1 and not parts[0].endswith((".dist-info", ".egg-info")):
            found.append(parts[0])
        elif len(parts) == 1 and parts[0].endswith(".py"):
            found.append(parts[0][:-3])
    return found


def _distribution(top: str) -> Optional[Any]:
    """The installed distribution whose top-level names include `top`."""
    by_package = getattr(metadata, "packages_distributions", None)
    if by_package is not None:
        names = by_package().get(top, [])
        for name in names:
            try:
                return metadata.distribution(name)
            except metadata.PackageNotFoundError:
                continue
        return None
    for dist in metadata.distributions():
        if top in _top_level(dist):
            return dist
    return None


def _from_distribution(dist: Any) -> Dict[str, Any]:
    md = dist.metadata
    meta: Dict[str, Any] = {}
    name = md.get("Name") or ""
    if md.get("Summary"):
        meta["summary"] = md["Summary"]
    licence = md.get("License-Expression") or md.get("License")
    if licence:
        meta["license"] = licence
    authors = [a for a in (md.get("Author"), md.get("Author-email")) if a]
    if authors:
        meta["authors"] = authors
    urls: Dict[str, str] = {}
    for entry in md.get_all("Project-URL") or []:
        label, _, url = entry.partition(",")
        if url.strip():
            urls[label.strip()] = url.strip()
    if md.get("Home-page"):
        urls.setdefault("Home-page", md["Home-page"])
    if urls:
        meta["urls"] = urls
    notices = []
    for file in md.get_all("License-File") or []:
        text = dist.read_text(f"licenses/{file}") or dist.read_text(file)
        if text and text.strip():
            notices.append({"component": name, "text": text, "format": "text"})
    if notices:
        meta["notices"] = notices
    return meta


def _from_attributes(module: types.ModuleType) -> Dict[str, Any]:
    meta: Dict[str, Any] = {}
    if module.__doc__:
        first = inspect.cleandoc(module.__doc__).split("\n\n")[0]
        meta["description"] = " ".join(first.split())
    author = getattr(module, "__author__", None)
    if isinstance(author, str):
        meta["authors"] = [author]
    elif isinstance(author, (list, tuple)) and all(isinstance(a, str) for a in author):
        meta["authors"] = list(author)
    for attr, key in (("__license__", "license"), ("__copyright__", "copyright")):
        value = getattr(module, attr, None)
        if isinstance(value, str):
            meta[key] = value
    return meta


def _plain(value: Any) -> bool:
    if value is None or isinstance(value, (bool, int, float, str)):
        return True
    if isinstance(value, (list, tuple)):
        return all(_plain(v) for v in value)
    if isinstance(value, Mapping):
        return all(isinstance(k, str) and _plain(v) for k, v in value.items())
    return False


def meta_of(module: types.ModuleType) -> Dict[str, Any]:
    """The `meta` map of the library `module` is. `TypeError` when its
    `_meta_` is not a mapping of plain data."""
    meta: Dict[str, Any] = {}
    dist = _distribution(module.__name__.split(".")[0])
    if dist is not None:
        meta.update(_from_distribution(dist))
    meta.update(_from_attributes(module))
    own = getattr(module, "_meta_", None)
    if own is not None:
        if not isinstance(own, Mapping) or not _plain(own):
            raise TypeError(f"{module.__name__}._meta_ is a mapping of plain data (text keys)")
        meta.update(own)
    return meta
