# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A plugin host written in Python.

Scans the directories it is given for libraries declaring the `greeter`
kind, whatever language they were written in, and calls each greeter
through the kind's table: a map back, a map lent and changed in place, a
map handed over, an object handed in and one handed back, an object lent
for one call. A provider that builds instances from a configuration is
given one its schema accepts, after one it refuses. Everything it opened
is closed before it exits, and it prints what it did.

With `--python-source BRIDGE`, it also adds the guatiao-source-py bridge as a
source of Python libraries described under each `--python-dir`, and calls
the Python greeters it serves beside the rest.

    python host.py DIR [DIR ...] [--python-source BRIDGE --python-dir DIR ...]
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any, Callable, Dict, List, Optional

# The kind's description lives beside the kind, as its C header does.
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "greeter_kind" / "python"))

from greeter_kind import Greeter  # noqa: E402

from guatiao import GuatiaoError, Registry, Value  # noqa: E402
from guatiao.kinds import FloorMismatch, Remote, live_objects  # noqa: E402

Say = Callable[[str], None]


class Ear:
    """A listener: what a conversation tells, kept."""

    def __init__(self) -> None:
        self.heard_so_far: List[str] = []

    def heard(self, what: str) -> None:
        self.heard_so_far.append(what)


def config_for(schema: Dict[str, Any], fill: str) -> Dict[str, str]:
    """A configuration naming every required text field of `schema`."""
    properties = schema.get("properties", {})
    return {
        name: fill
        for name in schema.get("required", [])
        if properties.get(name, {}).get("type") == "string"
    }


def exercise(greeter: Remote, say: Say) -> None:
    """Every slot of the greeter kind this table carries."""
    say(f"greet: {greeter.greet('ana').to_python()}")
    try:
        greeter.greet("")
    except GuatiaoError as refused:
        say(f"greet refused: {type(refused).__name__}: {refused.message}")
    if greeter.has("shout"):
        say(f"shout: {greeter.shout('ana')}")
    if greeter.has("check"):
        with Value({"name": "bo"}) as config:
            usable = greeter.check(config)
            say(f"check: {usable}, lent map now {config.to_python()}")
    if greeter.has("greet_from"):
        say(f"greet_from: {greeter.greet_from({'name': 'cy'}).to_python()}")
    if greeter.has("greet_into"):
        ear = Ear()
        greeter.greet_into("dee", ear)
        say(f"greet_into: the lent listener heard {ear.heard_so_far}")
    if greeter.has("start"):
        ear = Ear()
        try:
            conversation = greeter.start(ear)
        except GuatiaoError as refused:
            say(f"start refused: {refused.message}")
        else:
            with conversation:
                conversation.say("hi")
                conversation.say("again")
                transcript = bytearray(64)
                n = conversation.read(transcript)
                say(
                    f"conversation: {conversation.turns()} turns, "
                    f"transcript {bytes(transcript[:n])!r}, listener heard {ear.heard_so_far}"
                )


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("dirs", nargs="+", help="directories holding greeter libraries")
    parser.add_argument("--python-source", help="the guatiao-source-py bridge, to serve Python libraries")
    parser.add_argument(
        "--python-dir", action="append", default=[], help="where Python library manifests are"
    )
    args = parser.parse_args(argv)

    with Registry("python-host", "1.0") as reg:
        for directory in args.dirs:
            report = reg.scan_dir(directory, rules="kind=greeter")
            for path in report["loaded"]:
                print(f"loaded {Path(path).name}")
        if args.python_source:
            # This interpreter is the bridge's: what it imported, the
            # libraries import too.
            reg.add_source_file(args.python_source, {"dirs": args.python_dir})
            changed = reg.refresh()
            for served in changed["loaded"]:
                print(f"served {served['id']} from Python")
            for refused in changed["refused"]:
                print(f"refused {refused.get('id', '')}: {refused['why']}")

        for provider in reg.providers("greeter"):
            key = provider["key"]
            print(f"\n{key} ({provider['library']} {provider['version']})")

            def say(line: str) -> None:
                print(f"  {line}")

            try:
                greeter = reg.bind(key, Greeter)
            except FloorMismatch as mismatch:
                say(f"not this greeter kind: {mismatch}")
                continue
            if not provider["has_config"]:
                exercise(greeter, say)
                continue
            # Built from a configuration: one the schema refuses, then one
            # it accepts. The registry checks it before the provider sees it.
            declared = reg.provider_config(key)
            schema = declared.to_python() if declared is not None else {}
            try:
                reg.create(key, {})
            except GuatiaoError as refused:
                say(f"an empty configuration is refused: {refused.message}")
            config = config_for(schema, "hey")
            with reg.create(key, config) as instance:
                say(f"instance from {config}")
                exercise(instance.bind(Greeter), say)

        # Unmapped where the library agrees; a library with no say in it
        # (the C greeter) is retired instead, and stays mapped.
        print()
        for library in reg.libraries():
            key = library["key"]
            try:
                reg.unload(key)
                print(f"unloaded {key}")
            except GuatiaoError as refused:
                reg.retire(key)
                print(f"retired {key}, still mapped: unload said {type(refused).__name__}")
    print(f"python objects a library can still reach: {live_objects()}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
