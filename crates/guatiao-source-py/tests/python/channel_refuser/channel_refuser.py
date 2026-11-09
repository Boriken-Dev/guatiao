# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at https://mozilla.org/MPL/2.0/.

"""A provider of the channel echo kind whose `start` always raises: the
sink it was handed is kept, not destroyed, so the binding must."""

from channel_echo_kind import Channel, ChannelEcho, ChannelSink

from guatiao import Map, provide
from guatiao.errors import BadValue


__version__ = "1.0.0"


class Refuser(provide.Provider, ChannelEcho, display_name="Refuser, in Python"):
    kept: list = []

    def start(self, config: Map, sink: ChannelSink) -> Channel:
        self.kept.append(sink)
        raise BadValue("not today")
