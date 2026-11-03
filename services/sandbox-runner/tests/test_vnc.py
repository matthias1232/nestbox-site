"""Tests for the view-only filter of the desktop stream, without FastAPI.

The filter is the part of a view-only desktop that can go wrong quietly: it has
to let the RFB handshake and the framebuffer requests through, and it has to
drop every message that carries input. So it is tested directly, with byte
strings shaped like the real client messages.

`app.vnc` imports FastAPI at module scope (through the router and the terminal
helpers), so the import below installs minimal stand-ins first — the same
approach as `test_terminal.py`. The module under test is the real file.
"""
from __future__ import annotations

import os
import sys
import types
import unittest

# --- framework stand-ins ---------------------------------------------------


class _WebSocket:
    """Structural stand-in; only the class is needed for the import."""


class _WebSocketDisconnect(Exception):
    """Stand-in for the exception the router would import."""


class _APIRouter:
    """Enough of the router for the module import: a decorator per verb."""

    def _decorator(self, *_args, **_kwargs):
        return lambda handler: handler

    get = post = put = patch = delete = websocket = _decorator


_fastapi = types.ModuleType("fastapi")
_fastapi.APIRouter = _APIRouter
_fastapi.WebSocket = _WebSocket
_fastapi.WebSocketDisconnect = _WebSocketDisconnect
sys.modules.setdefault("fastapi", _fastapi)

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from app import vnc  # noqa: E402
from app.vnc import RfbClientFilter  # noqa: E402

# --- message shapes -------------------------------------------------------

HANDSHAKE = bytes(range(14))
KEY_EVENT = bytes([4, 1, 0, 0, 0, 0, 0, 65])
POINTER_EVENT = bytes([5, 1, 0, 10, 0, 20])
FRAMEBUFFER_REQUEST = bytes([3, 0, 0, 0, 0, 0, 0, 10, 0, 10])
# kind 6, three padding bytes, length 3 (signed 32 bit), then the text.
CLIENT_CUT_TEXT = bytes([6, 0, 0, 0, 0, 0, 0, 3, 65, 66, 67])
UNKNOWN = bytes([99, 0, 0])


class PassthroughTest(unittest.TestCase):
    """A desktop that may be driven sees exactly what the browser sent."""

    def test_a_writable_desktop_forwards_every_byte(self) -> None:
        flt = RfbClientFilter(False)
        self.assertEqual(flt.feed(HANDSHAKE), HANDSHAKE)
        self.assertEqual(flt.feed(KEY_EVENT), KEY_EVENT)
        self.assertEqual(flt.feed(UNKNOWN), UNKNOWN)
        self.assertFalse(flt.broken)


class ViewOnlyTest(unittest.TestCase):
    """A viewer's stream keeps the protocol alive and drops every input."""

    def test_the_handshake_passes_unchanged(self) -> None:
        flt = RfbClientFilter(True)
        self.assertEqual(flt.feed(HANDSHAKE), HANDSHAKE)

    def test_a_key_event_is_dropped(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        self.assertEqual(flt.feed(KEY_EVENT), b"")

    def test_a_framebuffer_update_request_passes(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        self.assertEqual(flt.feed(FRAMEBUFFER_REQUEST), FRAMEBUFFER_REQUEST)

    def test_a_pointer_event_split_over_two_feeds_is_dropped(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        self.assertEqual(flt.feed(POINTER_EVENT[:3]), b"")
        self.assertEqual(flt.feed(POINTER_EVENT[3:]), b"")

    def test_cut_text_is_dropped_and_the_next_request_passes(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        self.assertEqual(flt.feed(CLIENT_CUT_TEXT), b"")
        self.assertEqual(flt.feed(FRAMEBUFFER_REQUEST), FRAMEBUFFER_REQUEST)

    def test_handshake_and_first_message_in_one_frame(self) -> None:
        flt = RfbClientFilter(True)
        self.assertEqual(flt.feed(HANDSHAKE + FRAMEBUFFER_REQUEST), HANDSHAKE + FRAMEBUFFER_REQUEST)

    def test_a_dropped_message_between_two_requests_keeps_the_order(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        forwarded = flt.feed(FRAMEBUFFER_REQUEST + KEY_EVENT + FRAMEBUFFER_REQUEST)
        self.assertEqual(forwarded, FRAMEBUFFER_REQUEST + FRAMEBUFFER_REQUEST)

    def test_an_unknown_message_marks_the_stream_broken(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        flt.feed(UNKNOWN)
        self.assertTrue(flt.broken)

    def test_a_broken_filter_forwards_nothing_further(self) -> None:
        flt = RfbClientFilter(True)
        flt.feed(HANDSHAKE)
        flt.feed(UNKNOWN)
        self.assertEqual(flt.feed(FRAMEBUFFER_REQUEST), b"")


if __name__ == "__main__":
    unittest.main()

class BridgeTagTest(unittest.TestCase):
    def test_each_bridge_carries_its_own_tag(self) -> None:
        argv = vnc.bridge_command("abc123")
        self.assertEqual(argv[:2], ["python3", "-c"])
        self.assertIn("nestbox-vnc-bridge-abc123", argv[-1])

    def test_the_kill_command_names_exactly_one_tag(self) -> None:
        self.assertEqual(vnc.bridge_kill_command("abc123"), ["pkill", "-f", "nestbox-vnc-bridge-abc123"])
