# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A kind read from its description: the document a kind crate commits
beside its header (`<crate>.kinds.json`) or a provider carries, loaded at
run time (`kinds_of`) or written out as typed `Protocol` classes
(`generate`)."""

from __future__ import annotations

import ctypes
import json
import re
from collections.abc import Iterable, Mapping
from typing import Any, Callable, Union

from .. import _abi, _lib
from ..errors import raise_for_status
from ..value import _make_str, _str_or
from . import _shapes as shapes
from ._protocol import kind_of
from ._spec import Kind, Method

#: A document as plain data, or its JSON text.
Document = Union[Mapping[str, Any], str]

_SCALARS: "dict[str, shapes.Shape]" = {
    "bool": shapes.BOOL,
    "i8": shapes.I8,
    "i16": shapes.I16,
    "i32": shapes.I32,
    "i64": shapes.I64,
    "u8": shapes.U8,
    "u16": shapes.U16,
    "u32": shapes.U32,
    "u64": shapes.U64,
    "isize": shapes.ISIZE,
    "usize": shapes.USIZE,
    "f32": shapes.F32,
    "f64": shapes.F64,
}

_TREES = {"value": shapes.VALUE, "map": shapes.MAP, "list": shapes.LIST}


def _document(document: Document) -> "list[Mapping[str, Any]]":
    given = json.loads(document) if isinstance(document, str) else document
    if not isinstance(given, Mapping) or "kinds" not in given:
        raise TypeError("a kinds document is {\"kinds\": [...]}")
    found = given.get("kinds")
    if not isinstance(found, list) or not all(isinstance(k, Mapping) for k in found):
        raise TypeError("a kinds document's `kinds` is a list of kinds")
    return list(found)


def _ordered(kinds: "list[Mapping[str, Any]]", outside: Callable[[str], bool]) -> "list[Mapping[str, Any]]":
    """The kinds with every object kind a slot names before the kind
    naming it, otherwise in document order: the order a module defines
    them in. A kind found neither here nor `outside` is refused, and so
    is a cycle."""
    by_name: "dict[str, Mapping[str, Any]]" = {}
    for k in kinds:
        name = k.get("name")
        if not isinstance(name, str) or not name:
            raise TypeError("a kind without a name")
        if name in by_name:
            raise TypeError(f"the document describes {name} twice")
        by_name[name] = k
    out: "list[Mapping[str, Any]]" = []
    done: "set[str]" = set()
    making: "list[str]" = []

    def visit(name: str) -> None:
        if name in done:
            return
        if name in making:
            raise TypeError(f"{name} refers to itself through its slots, which a description cannot load here")
        making.append(name)
        for named in _named_kinds(by_name[name]):
            if named in by_name:
                visit(named)
            elif not outside(named):
                raise TypeError(f"{name}: no kind {named} in the document or among the known kinds")
        making.pop()
        done.add(name)
        out.append(by_name[name])

    for k in kinds:
        visit(k["name"])
    return out


def _named_kinds(kind: Mapping[str, Any]) -> "list[str]":
    """Every kind an argument or an answer of `kind` names, in order."""
    named = []
    for slot in kind.get("slots", []):
        for one in list(slot.get("args", [])) + [slot.get("answer") or {}]:
            if one.get("type") == "object" and isinstance(one.get("kind"), str):
                named.append(one["kind"])
    return named


def _arg(arg: Mapping[str, Any], kinds: Mapping[str, Kind], where: str) -> shapes.Shape:
    ty, how = arg.get("type"), arg.get("pass")
    if ty in _SCALARS:
        return _SCALARS[ty]
    if ty == "str":
        return shapes.STR
    if ty == "bytes":
        return shapes.BYTES
    if ty == "bytes_mut":
        return shapes.BYTES_MUT
    if ty in _TREES:
        tree = _TREES[ty]
        if how == "borrowed":
            if ty == "list":
                raise TypeError(f"{where}: a list is not borrowed, but taken or lent")
            return tree
        if how == "taken":
            return shapes.taken(tree)
        if how == "lent":
            return shapes.lent(tree)
        raise TypeError(f"{where}: a {ty} is passed borrowed, taken or lent, not {how!r}")
    if ty == "object":
        kind = kinds[arg["kind"]]
        if how == "taken":
            return shapes.taken(kind)
        if how == "lent":
            return shapes.lent(kind)
        raise TypeError(f"{where}: an object is passed taken or lent, not {how!r}")
    raise TypeError(f"{where}: {ty!r} is no argument type")


def _answer(answer: "Mapping[str, Any] | None", kinds: Mapping[str, Kind], where: str) -> Any:
    if answer is None:
        return None
    ty = answer.get("type")
    if ty in _SCALARS:
        return _SCALARS[ty]
    if ty == "text":
        return shapes.TEXT
    if ty in _TREES:
        return _TREES[ty]
    if ty == "object":
        return kinds[answer["kind"]]
    raise TypeError(f"{where}: an answer cannot be {ty!r}")


def kinds_of(document: Document, *, known: Iterable[Any] = ()) -> "dict[str, Kind]":
    """Each kind of a description document, by name, as a `Kind` ready to
    `bind` a table or serve one: what a kind class would build, read from
    the document a kind crate commits (`<crate>.kinds.json`, its text or
    its parsed object) or the one `Registry.kind_descriptions` answers.

    An object kind a slot names is found in the document, else among
    `known` (kind classes or `Kind`s). `TypeError` names the kind and the
    slot for a document that is not one, a type it does not know, or kinds
    that refer to each other in a cycle.
    """
    outside = {k.name: k for k in (kind_of(one) for one in known)}
    built: "dict[str, Kind]" = dict(outside)
    out: "dict[str, Kind]" = {}
    for description in _ordered(_document(document), lambda name: name in outside):
        name = description["name"]
        methods = []
        for i, slot in enumerate(description.get("slots", [])):
            where = f"{name}.{slot.get('name', i)}"
            methods.append(
                Method(
                    str(slot["name"]),
                    [_arg(a, built, f"{where} argument {j}") for j, a in enumerate(slot.get("args", []))],
                    _answer(slot.get("answer"), built, f"{where} answer"),
                    fallible=bool(slot.get("fallible")),
                    required=bool(slot.get("required")),
                )
            )
        kind = Kind(
            name,
            floor_hash=int(description["floor_hash"]),
            methods=methods,
            is_object=bool(description.get("object")),
        )
        built[name] = kind
        out[name] = kind
    return {d["name"]: out[d["name"]] for d in _document(document)}


# --- a kind written as data ---------------------------------------------------

_SCALAR_NAMES = {shape: name for name, shape in _SCALARS.items()}


def _tree_name(tag: Any) -> str:
    from .. import _abi

    return {None: "value", _abi.Tag.MAP: "map", _abi.Tag.LIST: "list"}[tag]


def _arg_description(name: str, shape: shapes.Shape) -> "dict[str, Any]":
    out: "dict[str, Any]" = {"name": name}
    if shape in _SCALAR_NAMES:
        out["type"] = _SCALAR_NAMES[shape]
    elif shape is shapes.STR:
        out["type"] = "str"
    elif shape is shapes.BYTES:
        out["type"] = "bytes"
    elif shape is shapes.BYTES_MUT:
        out["type"] = "bytes_mut"
    elif isinstance(shape, shapes._ObjectShape):
        out.update(type="object", kind=shape.kind.name, **{"pass": "taken" if shape.mode == "take" else "lent"})
    elif isinstance(shape, shapes._Tree):
        how = {"borrow": "borrowed", "take": "taken", "lend": "lent"}[shape.mode]
        out.update(type=_tree_name(shape.tag), **{"pass": how})
    else:
        raise TypeError(f"{name}: {shape!r} has no description")
    return out


def _answer_description(shape: Any) -> "dict[str, Any] | None":
    if shape is None:
        return None
    if shape in _SCALAR_NAMES:
        return {"type": _SCALAR_NAMES[shape]}
    if shape is shapes.TEXT:
        return {"type": "text"}
    if isinstance(shape, shapes._ObjectShape):
        return {"type": "object", "kind": shape.kind.name}
    if isinstance(shape, shapes._Tree):
        return {"type": _tree_name(shape.tag)}
    raise TypeError(f"{shape!r} has no description")


def slot_description(method: Method) -> "dict[str, Any]":
    """One slot as a description says it."""
    out: "dict[str, Any]" = {
        "name": method.name,
        "required": method.required,
        "fallible": method.fallible,
        "args": [_arg_description(n, s) for n, s in zip(method.arg_names, method.args)],
    }
    answer = _answer_description(method.returns)
    if answer is not None:
        out["answer"] = answer
    if method.doc:
        out["doc"] = method.doc
    return out


def floor_hash_of(is_object: bool, slots: "list[dict[str, Any]]") -> int:
    """The floor hash a kind born outside Rust gets, which a trait declaring
    the same slots also has: the library's `guatiao_kind_floor_hash`, FNV-1a
    (32-bit) of the shape and every slot in table order spelled as
    `#[guatiao::kind]` spells them (`docs/encodings.md`, "A kind,
    described"). Raises `BadValue` for slots that are not a description."""
    text = json.dumps({"object": is_object, "slots": slots}).encode("utf-8")
    view, _buf = _make_str(text)
    core = _lib.core()
    out, why = ctypes.c_uint32(), _abi.Value()
    status = core.guatiao_kind_floor_hash(view, ctypes.byref(core.alloc), ctypes.byref(out), ctypes.byref(why))
    if status != _abi.Status.OK:
        message = _str_or(why, "the slots are not a kind's")
        core.guatiao_value_free(ctypes.byref(why))
        raise_for_status(status, message)
    return out.value


def describe_kinds(*kinds: Any) -> "dict[str, Any]":
    """The kinds given (kind classes or `Kind`s), each followed by the object
    kinds its slots take or answer, each once, as the description document a
    kind crate commits beside its header (`{"kinds": [...]}`):
    what `kinds_of`, `python -m guatiao.kinds`, Node's `kindsOf` and Dart's
    `dart run guatiao:kind` read. A kind class's docstrings are the docs."""
    out: "list[dict[str, Any]]" = []
    seen: "set[str]" = set()

    def describe(kind: Kind) -> None:
        if kind.name in seen:
            return
        seen.add(kind.name)
        entry: "dict[str, Any]" = {"name": kind.name, "floor_hash": kind.floor_hash, "object": kind.is_object}
        if kind.doc:
            entry["doc"] = kind.doc
        entry["slots"] = [slot_description(m) for m in kind.methods]
        out.append(entry)
        for m in kind.methods:
            for shape in [*m.args, m.returns]:
                if isinstance(shape, shapes._ObjectShape):
                    describe(shape.kind)

    for one in kinds:
        describe(kind_of(one))
    return {"kinds": out}


# --- written out as classes -------------------------------------------------


def _class_name(kind: str) -> str:
    return "".join(part[:1].upper() + part[1:] for part in kind.split("_") if part)


_SCALAR_ANNOTATIONS = {
    "bool": "bool",
    "f64": "float",
    "i8": "I8",
    "i16": "I16",
    "i32": "I32",
    "i64": "I64",
    "u8": "U8",
    "u16": "U16",
    "u32": "U32",
    "u64": "U64",
    "isize": "ISIZE",
    "usize": "USIZE",
    "f32": "F32",
}

_TREE_TYPES = {"value": ("Value", "Ref"), "map": ("Map", "Map"), "list": ("List", "List")}


class _Writer:
    """The names a generated module uses, gathered as its annotations are
    written, so it imports exactly those."""

    def __init__(self, classes: Mapping[str, str]) -> None:
        self.classes = classes
        self.guatiao: "set[str]" = set()
        self.kinds: "set[str]" = set()
        self.optional = False

    def arg(self, arg: Mapping[str, Any]) -> str:
        ty, how = arg.get("type"), arg.get("pass")
        if ty in _SCALAR_ANNOTATIONS:
            return self._scalar(ty)
        if ty == "str":
            return "str"
        if ty == "bytes":
            return "bytes"
        if ty == "bytes_mut":
            return "bytearray"
        if ty == "object":
            mark = "Taken" if how == "taken" else "Lent"
            self.kinds.add(mark)
            return f"{mark}[{self.classes[arg['kind']]}]"
        taken_type, borrowed_type = _TREE_TYPES[str(ty)]
        if how == "borrowed":
            self.guatiao.add(borrowed_type)
            if arg.get("nullable"):
                self.optional = True
                return f"Optional[{borrowed_type}]"
            return borrowed_type
        mark = "Taken" if how == "taken" else "Lent"
        inner = taken_type if how == "taken" else borrowed_type
        self.kinds.add(mark)
        self.guatiao.add(inner)
        return f"{mark}[{inner}]"

    def answer(self, answer: "Mapping[str, Any] | None") -> str:
        if answer is None:
            return "None"
        ty = answer.get("type")
        if ty in _SCALAR_ANNOTATIONS:
            return self._scalar(ty)
        if ty == "text":
            return "str"
        if ty == "object":
            return self.classes[answer["kind"]]
        name = {"value": "ValueAnswer", "map": "MapAnswer", "list": "ListAnswer"}[str(ty)]
        self.kinds.add(name)
        return name

    def _scalar(self, ty: str) -> str:
        name = _SCALAR_ANNOTATIONS[ty]
        if name not in ("bool", "float"):
            self.kinds.add(name)
        return name


def _docstring(text: "str | None", indent: str) -> "list[str]":
    if not text:
        return []
    body = text.replace("\\", "\\\\").replace('"""', '\\"\\"\\"')
    lines = body.split("\n")
    if body.endswith('"'):
        lines[-1] += " "
    if len(lines) == 1:
        return [f'{indent}"""{lines[0]}"""']
    out = [f'{indent}"""{lines[0]}']
    out += [f"{indent}{line}" if line else "" for line in lines[1:]]
    out.append(f'{indent}"""')
    return out


def generate(document: Document, *, source: str = "", command: str = "") -> str:
    """The module of a description's kinds as typed `Protocol` classes, one
    per kind, each object kind before the kinds naming it: the same
    classes a kind written by hand would be, docs included. `source` and
    `command` go in the module's first lines, saying where it came from
    and how to write it again."""
    kinds = _ordered(_document(document), lambda name: False)
    classes = {k["name"]: _class_name(k["name"]) for k in kinds}
    if len(set(classes.values())) != len(classes):
        raise TypeError("two kinds would be the same class name")
    writer = _Writer(classes)
    body: "list[str]" = []
    for kind in kinds:
        cls = classes[kind["name"]]
        opening = f'class {cls}(KindProtocol, Protocol, kind="{kind["name"]}", floor_hash={int(kind["floor_hash"])}'
        opening += ", is_object=True):" if kind.get("object") else "):"
        body += ["", "", opening]
        doc = kind.get("doc") or f"The `{kind['name']}` kind."
        body += _docstring(doc, "    ")
        for slot in kind.get("slots", []):
            name = str(slot["name"])
            if not re.fullmatch(r"[a-z_][a-z0-9_]*", name):
                raise TypeError(f"{kind['name']}: `{name}` is no Python method name")
            params = "".join(f", {a['name']}: {writer.arg(a)}" for a in slot.get("args", []))
            answer = writer.answer(slot.get("answer"))
            body.append("")
            if slot.get("required"):
                body.append("    @required")
            if slot.get("fallible"):
                body.append("    @fallible")
            if slot.get("doc"):
                body.append(f"    def {name}(self{params}) -> {answer}:")
                body += _docstring(slot["doc"], "        ")
                body.append("        ...")
            else:
                body.append(f"    def {name}(self{params}) -> {answer}: ...")
    writer.kinds.update({"KindProtocol"})
    if any(s.get("required") for k in kinds for s in k.get("slots", [])):
        writer.kinds.add("required")
    if any(s.get("fallible") for k in kinds for s in k.get("slots", [])):
        writer.kinds.add("fallible")
    head: "list[str]" = []
    if source:
        head.append(f"# GENERATED by `python -m guatiao.kinds` from {source}. DO NOT EDIT.")
    if command:
        head.append(f"# Regenerate: {command}")
    if head:
        head.append("")
    head += [
        f'"""The kinds of {source or "a description"}, as typed `Protocol` classes."""',
        "",
        "from __future__ import annotations",
        "",
        "from typing import " + ", ".join(sorted({"Protocol"} | ({"Optional"} if writer.optional else set()))),
        "",
    ]
    if writer.guatiao:
        head.append("from guatiao import " + ", ".join(sorted(writer.guatiao)))
    head.append("from guatiao.kinds import " + ", ".join(sorted(writer.kinds, key=_import_order)))
    tail = ["", "", "__all__ = [" + ", ".join(f'"{classes[k["name"]]}"' for k in kinds) + "]", ""]
    return "\n".join(head + body + tail)


def _import_order(name: str) -> "tuple[int, str]":
    """Upper-case names first, as isort orders them."""
    return (0 if name.isupper() else 1 if name[:1].isupper() else 2, name)
