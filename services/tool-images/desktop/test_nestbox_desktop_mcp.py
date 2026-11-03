"""Tests for `nestbox-desktop-mcp`, the desktop-control MCP server.

Run from the repository root with
`python3 -m unittest discover -s services/tool-images/desktop -p 'test_*.py'`,
or in this folder with `python3 -m unittest test_nestbox_desktop_mcp`.

Two kinds of test, and both matter:

* the pure helpers, with the monitor arrangement a two-screen desktop has, so
  "relative to monitor 2" is proven to add the right offset and nothing else;
* one JSON-RPC round trip over the real `serve()` loop, because a protocol
  server that answers its own function tests is not a protocol server. The
  commands behind the tools are replaced with a recorder, so these tests need
  no X display and no `xdotool`.

The module skips itself when the server file is absent, so this test file stays
runnable on its own while the desktop pack is being changed elsewhere.
"""
from __future__ import annotations

import base64
import importlib.machinery
import importlib.util
import io
import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SCRIPT = Path(__file__).with_name("nestbox-desktop-mcp")

try:
    _loader = importlib.machinery.SourceFileLoader("nestbox_desktop_mcp", str(SCRIPT))
    _spec = importlib.util.spec_from_loader("nestbox_desktop_mcp", _loader)
    if _spec is None:  # pragma: no cover - only when the file cannot be loaded
        raise unittest.SkipTest("nestbox-desktop-mcp is not loadable")
    mcp = importlib.util.module_from_spec(_spec)
    _loader.exec_module(mcp)
except (FileNotFoundError, OSError, ImportError) as _missing:  # pragma: no cover
    raise unittest.SkipTest(f"nestbox-desktop-mcp is not present: {_missing}") from _missing


def monitor(identifier: str, width: int, height: int, x: int, y: int, primary: bool = False) -> dict:
    """One monitor in the shape `nestbox-display get` reports."""
    entry = {
        "id": identifier,
        "name": f"Monitor {identifier[1:]}",
        "width": width,
        "height": height,
        "x": x,
        "y": y,
    }
    if primary:
        entry["primary"] = True
    return entry


# The arrangement the U09 pack produces for two screens: 1440x900 primary on the
# left, 2560x1440 on the right, and the framebuffer around both.
LAYOUT = {
    "mode": "separate",
    "monitors": [monitor("m1", 1440, 900, 0, 0, primary=True), monitor("m2", 2560, 1440, 1440, 0)],
    "framebuffer": [4000, 1440],
    "applied_at": "2026-11-03T10:00:00Z",
    "applied_by": "admin",
}

# The L shape: the second monitor sits below the first, so (2000, 200) is a
# black corner that belongs to no monitor at all.
LAYOUT_L = {
    "mode": "separate",
    "monitors": [monitor("m1", 1920, 1080, 0, 0), monitor("m2", 1280, 1024, 0, 1080)],
    "framebuffer": [1920, 2104],
}

WMCTRL = """\
0x01000007 -1 0    0    1440 900  xfce4-panel.Xfce4-panel  nestbox-sandbox-desktop-01 xfce4-panel
0x02a00003  0 0    24   1400 826  google-chrome.Google-chrome  nestbox-sandbox-desktop-01 New Tab - Google Chrome
0x02e00004  1 1440 0    1280 1024 xfce4-terminal.Xfce4-terminal  nestbox-sandbox-desktop-01 Nestbox
not a window line
"""


class MonitorRefTest(unittest.TestCase):
    def test_an_id_points_at_that_monitor(self) -> None:
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, "m2")["x"], 1440)

    def test_a_one_based_number_points_at_that_monitor(self) -> None:
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, 2)["id"], "m2")
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, "1")["id"], "m1")

    def test_a_name_points_at_that_monitor_whatever_its_case(self) -> None:
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, "monitor 2")["id"], "m2")

    def test_all_is_the_framebuffer(self) -> None:
        whole = mcp.monitor_by_ref(LAYOUT, "all")
        self.assertEqual(whole["id"], "all")
        self.assertEqual((whole["x"], whole["y"]), (0, 0))
        self.assertEqual((whole["width"], whole["height"]), (4000, 1440))

    def test_the_default_is_all(self) -> None:
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, None)["id"], "all")
        self.assertEqual(mcp.monitor_by_ref(LAYOUT, "")["id"], "all")

    def test_an_unknown_reference_names_the_monitors_that_exist(self) -> None:
        with self.assertRaises(mcp.Failure) as raised:
            mcp.monitor_by_ref(LAYOUT, "m9")
        self.assertIn("m1", str(raised.exception))
        self.assertIn("m2", str(raised.exception))

    def test_index_zero_is_not_a_monitor(self) -> None:
        with self.assertRaises(mcp.Failure):
            mcp.monitor_by_ref(LAYOUT, "0")

    def test_an_empty_layout_is_a_failure_not_an_empty_box(self) -> None:
        with self.assertRaises(mcp.Failure):
            mcp.monitor_by_ref({"monitors": []}, "all")


class AbsolutePointTest(unittest.TestCase):
    def test_a_point_on_the_first_monitor_stays_where_it_is(self) -> None:
        self.assertEqual(mcp.to_absolute(10, 20, mcp.monitor_by_ref(LAYOUT, "m1")), (10, 20))

    def test_a_point_on_the_second_monitor_gains_its_offset(self) -> None:
        self.assertEqual(mcp.to_absolute(10, 20, mcp.monitor_by_ref(LAYOUT, "m2")), (1450, 20))

    def test_the_last_pixel_is_inside_the_monitor(self) -> None:
        self.assertEqual(mcp.to_absolute(2560, 1440, mcp.monitor_by_ref(LAYOUT, "m2")), (3999, 1439))

    def test_a_point_past_the_edge_is_clamped_not_moved_to_a_neighbour(self) -> None:
        self.assertEqual(mcp.to_absolute(5000, -10, mcp.monitor_by_ref(LAYOUT, "m2")), (3999, 0))

    def test_coordinates_must_be_whole_numbers(self) -> None:
        with self.assertRaises(mcp.Failure):
            mcp.to_absolute(1.5, 2, mcp.monitor_by_ref(LAYOUT, "m1"))
        with self.assertRaises(mcp.Failure):
            mcp.to_absolute(True, 2, mcp.monitor_by_ref(LAYOUT, "m1"))
        with self.assertRaises(mcp.Failure):
            mcp.to_absolute("12", 2, mcp.monitor_by_ref(LAYOUT, "m1"))


class MonitorOfTest(unittest.TestCase):
    def test_a_point_is_on_the_monitor_that_covers_it(self) -> None:
        self.assertEqual(mcp.monitor_of(100, 100, LAYOUT["monitors"])["id"], "m1")
        self.assertEqual(mcp.monitor_of(1500, 100, LAYOUT["monitors"])["id"], "m2")

    def test_the_black_corner_of_an_l_shape_belongs_to_no_monitor(self) -> None:
        self.assertIsNone(mcp.monitor_of(2000, 200, LAYOUT_L["monitors"]))

    def test_the_edge_between_two_monitors_belongs_to_the_second(self) -> None:
        self.assertEqual(mcp.monitor_of(1440, 0, LAYOUT["monitors"])["id"], "m2")


class WmctrlParseTest(unittest.TestCase):
    def test_every_window_keeps_its_id_geometry_class_and_title(self) -> None:
        windows = mcp.parse_wmctrl(WMCTRL)
        self.assertEqual(len(windows), 3)
        chrome = windows[1]
        self.assertEqual(chrome["id"], "0x02a00003")
        self.assertEqual((chrome["x"], chrome["y"]), (0, 24))
        self.assertEqual((chrome["width"], chrome["height"]), (1400, 826))
        self.assertEqual(chrome["class"], "google-chrome.Google-chrome")
        self.assertEqual(chrome["title"], "New Tab - Google Chrome")

    def test_the_host_is_not_part_of_the_title(self) -> None:
        windows = mcp.parse_wmctrl(WMCTRL)
        self.assertNotIn("nestbox-sandbox-desktop-01", windows[2]["title"])
        self.assertEqual(windows[2]["title"], "Nestbox")

    def test_a_line_that_is_not_a_window_is_dropped(self) -> None:
        windows = mcp.parse_wmctrl(WMCTRL)
        self.assertEqual([window["id"] for window in windows], ["0x01000007", "0x02a00003", "0x02e00004"])

    def test_empty_output_is_no_window(self) -> None:
        self.assertEqual(mcp.parse_wmctrl(""), [])


class KeyComboTest(unittest.TestCase):
    def test_one_chord_is_one_chord(self) -> None:
        self.assertEqual(mcp.key_combo("ctrl+l"), ["ctrl+l"])

    def test_modifier_spellings_are_folded_but_the_key_keeps_its_case(self) -> None:
        self.assertEqual(mcp.key_combo("Control+L"), ["ctrl+L"])
        self.assertEqual(mcp.key_combo("Win+e"), ["super+e"])
        self.assertEqual(mcp.key_combo("alt+F4"), ["alt+F4"])

    def test_a_comma_is_a_sequence(self) -> None:
        self.assertEqual(mcp.key_combo("ctrl+a, Delete"), ["ctrl+a", "Delete"])

    def test_an_empty_spec_is_a_failure(self) -> None:
        for text in ("", "  ", ",", "ctrl+,", "ctrl++a"):
            with self.assertRaises(mcp.Failure, msg=text):
                mcp.key_combo(text)
        with self.assertRaises(mcp.Failure):
            mcp.key_combo(None)


class ScreenshotHelpersTest(unittest.TestCase):
    def test_the_scale_becomes_a_percentage(self) -> None:
        self.assertEqual(mcp.thumbnail_percent(0.5), 50)
        self.assertEqual(mcp.thumbnail_percent(1), 100)

    def test_the_percentage_is_held_to_the_documented_range(self) -> None:
        self.assertEqual(mcp.thumbnail_percent(0.1), 25)
        self.assertEqual(mcp.thumbnail_percent(4), 100)

    def test_a_scale_that_is_not_a_number_is_a_failure(self) -> None:
        for value in ("0.5", True, None):
            with self.assertRaises(mcp.Failure, msg=str(value)):
                mcp.thumbnail_percent(value)

    def test_a_png_header_gives_its_pixel_size(self) -> None:
        png = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + (1280).to_bytes(4, "big") + (720).to_bytes(4, "big")
        self.assertEqual(mcp.png_size(png), (1280, 720))

    def test_something_that_is_not_a_png_has_no_size(self) -> None:
        self.assertIsNone(mcp.png_size(b"not an image at all"))
        self.assertIsNone(mcp.png_size(b""))


def roundtrip(*requests: object) -> list[dict]:
    """Feed requests through the real stdio loop and return the answers."""
    stdin = io.StringIO("".join(json.dumps(request) + "\n" for request in requests))
    stdout = io.StringIO()
    mcp.serve(stdin, stdout)
    return [json.loads(line) for line in stdout.getvalue().splitlines() if line.strip()]


class ProtocolTest(unittest.TestCase):
    def test_initialize_answers_the_protocol_and_the_server_name(self) -> None:
        answers = roundtrip({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}})
        self.assertEqual(len(answers), 1)
        result = answers[0]["result"]
        self.assertEqual(result["protocolVersion"], "2025-03-26")
        self.assertEqual(result["serverInfo"]["name"], "nestbox-desktop")
        self.assertIn("tools", result["capabilities"])

    def test_an_unknown_protocol_version_falls_back_to_ours(self) -> None:
        answers = roundtrip({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}})
        self.assertEqual(answers[0]["result"]["protocolVersion"], "2025-03-26")

    def test_the_initialized_notification_gets_no_answer(self) -> None:
        answers = roundtrip({"jsonrpc": "2.0", "method": "notifications/initialized"})
        self.assertEqual(answers, [])

    def test_ping_answers_an_empty_result(self) -> None:
        answers = roundtrip({"jsonrpc": "2.0", "id": 7, "method": "ping"})
        self.assertEqual(answers[0], {"jsonrpc": "2.0", "id": 7, "result": {}})

    def test_tools_list_names_every_tool_with_a_schema(self) -> None:
        answers = roundtrip(
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list"},
        )
        tools = answers[1]["result"]["tools"]
        names = [tool["name"] for tool in tools]
        self.assertEqual(len(names), 19)
        self.assertEqual(len(set(names)), 19)
        for name in (
            "desktop_layout",
            "desktop_set_layout",
            "desktop_screenshot",
            "desktop_click",
            "desktop_drag",
            "desktop_type",
            "desktop_key",
            "desktop_windows",
            "desktop_place",
            "desktop_span",
            "desktop_launch",
        ):
            self.assertIn(name, names)
        for tool in tools:
            self.assertEqual(tool["inputSchema"]["type"], "object")
            self.assertTrue(tool["description"].strip())

    def test_every_advertised_tool_has_a_handler(self) -> None:
        self.assertEqual({tool["name"] for tool in mcp.TOOLS}, set(mcp.HANDLERS))

    def test_an_unknown_method_is_a_protocol_error(self) -> None:
        answers = roundtrip({"jsonrpc": "2.0", "id": 3, "method": "resources/list"})
        self.assertEqual(answers[0]["error"]["code"], -32601)

    def test_a_line_that_is_not_json_is_a_parse_error(self) -> None:
        stdin = io.StringIO("{not json\n")
        stdout = io.StringIO()
        mcp.serve(stdin, stdout)
        self.assertEqual(json.loads(stdout.getvalue())["error"]["code"], -32700)

    def test_an_unknown_tool_is_a_protocol_error(self) -> None:
        answers = roundtrip(
            {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "desktop_nope", "arguments": {}}}
        )
        self.assertEqual(answers[0]["error"]["code"], -32602)

    def test_desktop_layout_returns_the_layout_as_readable_json(self) -> None:
        with mock.patch.object(mcp, "current_layout", return_value=LAYOUT):
            answers = roundtrip({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {"name": "desktop_layout"}})
        result = answers[0]["result"]
        self.assertFalse(result["isError"])
        layout = json.loads(result["content"][0]["text"])
        self.assertEqual([entry["id"] for entry in layout["monitors"]], ["m1", "m2"])

    def test_a_tool_that_cannot_run_reports_an_error_result_not_a_crash(self) -> None:
        with mock.patch.object(mcp, "run", side_effect=mcp.Failure("xdotool is not installed")):
            answers = roundtrip(
                {
                    "jsonrpc": "2.0",
                    "id": 6,
                    "method": "tools/call",
                    "params": {"name": "desktop_click", "arguments": {"x": 1, "y": 2}},
                }
            )
        result = answers[0]["result"]
        self.assertTrue(result["isError"])
        self.assertIn("xdotool", result["content"][0]["text"])


class ToolCommandTest(unittest.TestCase):
    def test_a_click_on_the_second_monitor_is_one_absolute_xdotool_call(self) -> None:
        commands: list[list[str]] = []

        def record(command: list[str]) -> str:
            commands.append(command)
            return ""

        with mock.patch.object(mcp, "current_layout", return_value=LAYOUT), mock.patch.object(mcp, "run", record):
            content = mcp.tool_click({"x": 10, "y": 20, "monitor": "m2"})

        self.assertEqual(commands, [["xdotool", "mousemove", "--sync", "1450", "20", "click", "1"]])
        answer = json.loads(content[0]["text"])
        self.assertEqual(answer["absolute"], {"x": 1450, "y": 20})
        self.assertEqual(answer["relative"], {"x": 10, "y": 20})
        self.assertEqual(answer["monitor"], "m2")

    def test_a_double_right_click_repeats_the_button(self) -> None:
        commands: list[list[str]] = []

        def record(command: list[str]) -> str:
            commands.append(command)
            return ""

        with mock.patch.object(mcp, "current_layout", return_value=LAYOUT), mock.patch.object(mcp, "run", record):
            mcp.tool_click({"x": 0, "y": 0, "button": "right", "double": True})

        self.assertEqual(
            commands[0],
            ["xdotool", "mousemove", "--sync", "0", "0", "click", "--repeat", "2", "--delay", "80", "3"],
        )

    def test_a_drag_presses_moves_and_releases_across_monitors(self) -> None:
        commands: list[list[str]] = []

        def record(command: list[str]) -> str:
            commands.append(command)
            return ""

        with mock.patch.object(mcp, "current_layout", return_value=LAYOUT), mock.patch.object(mcp, "run", record), mock.patch.object(mcp.time, "sleep"):
            content = mcp.tool_drag(
                {"from": {"x": 10, "y": 10, "monitor": "m1"}, "to": {"x": 20, "y": 30, "monitor": "m2"}}
            )

        self.assertEqual(commands[0], ["xdotool", "mousemove", "--sync", "10", "10", "mousedown", "1"])
        self.assertEqual(commands[-1], ["xdotool", "mouseup", "1"])
        self.assertEqual(commands[-2], ["xdotool", "mousemove", "--sync", "1460", "30"])
        answer = json.loads(content[0]["text"])
        self.assertEqual(answer["from"]["monitor"], "m1")
        self.assertEqual(answer["to"]["absolute"], {"x": 1460, "y": 30})

    def test_a_screenshot_crops_to_the_monitor_and_returns_an_image(self) -> None:
        png = b"\x89PNG\r\n\x1a\n" + b"\x00\x00\x00\rIHDR" + (1280).to_bytes(4, "big") + (720).to_bytes(4, "big")
        commands: list[list[str]] = []

        with tempfile.TemporaryDirectory() as directory:
            shot = Path(directory) / "shot.png"
            shot.write_bytes(png)

            def record(command: list[str]) -> str:
                commands.append(command)
                return ""

            with mock.patch.object(mcp, "current_layout", return_value=LAYOUT), mock.patch.object(mcp, "run", record), mock.patch.object(mcp, "SHOT_PATH", str(shot)):
                content = mcp.tool_screenshot({"monitor": "m2", "scale": 0.5})

        self.assertEqual(commands[0], ["scrot", "--overwrite", "-a", "1440,0,2560,1440", "-t", "50", str(shot)])
        self.assertEqual(content[1]["type"], "image")
        self.assertEqual(content[1]["mimeType"], "image/png")
        self.assertEqual(base64.b64decode(content[1]["data"]), png)
        summary = json.loads(content[0]["text"])
        self.assertEqual(summary["monitor"], "m2")
        self.assertEqual(summary["monitor_geometry"], {"x": 1440, "y": 0, "width": 2560, "height": 1440})
        self.assertEqual(summary["image_pixels"], {"width": 1280, "height": 720})

    def test_a_window_is_reported_with_the_monitor_it_is_on(self) -> None:
        with mock.patch.object(mcp, "current_layout", return_value=LAYOUT), mock.patch.object(
            mcp, "run", return_value=WMCTRL
        ):
            content = mcp.tool_windows({})
        windows = json.loads(content[0]["text"])["windows"]
        self.assertEqual(windows[1]["monitor"], "m1")
        self.assertEqual(windows[2]["monitor"], "m2")

    def test_launching_something_outside_the_allowlist_is_refused(self) -> None:
        with self.assertRaises(mcp.Failure):
            mcp.tool_launch({"app": "bash"})

    def test_launching_a_terminal_spawns_the_allowlisted_program(self) -> None:
        with mock.patch.object(mcp, "spawn") as spawn:
            content = mcp.tool_launch({"app": "terminal"})
        spawn.assert_called_once_with(["xfce4-terminal"])
        self.assertEqual(json.loads(content[0]["text"])["launched"], "terminal")

    def test_spawn_starts_the_program_detached_without_a_shell(self) -> None:
        with mock.patch.object(mcp.subprocess, "Popen") as popen:
            mcp.spawn(["xfce4-terminal"])
        command = popen.call_args.args[0]
        self.assertEqual(command, ["xfce4-terminal"])
        self.assertTrue(popen.call_args.kwargs["start_new_session"])
        self.assertNotIn("shell", popen.call_args.kwargs)
        self.assertEqual(popen.call_args.kwargs["env"]["DISPLAY"], mcp.DISPLAY)


if __name__ == "__main__":
    unittest.main()
