"""Tests for `nestbox-display`, the display-layout tool of a desktop sandbox.

Run with `python3 -m unittest discover -s services/tool-images/desktop -p 'test_*.py'`
from the repository root, or `python3 -m unittest test_nestbox_display` in this
folder.

The fixture is the real `xrandr --query` and `xrandr --listmonitors` output of
`nestbox-sandbox-desktop-01` (TigerVNC 1.13.1, Xorg 21.1.11), captured on
2026-11-03. The two modeline cases are the ones the plan names; they are the
proof that the CVT arithmetic is the one `cvt -r` does.
"""
from __future__ import annotations

import importlib.machinery
import importlib.util
import json
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("nestbox-display")
_loader = importlib.machinery.SourceFileLoader("nestbox_display", str(SCRIPT))
_spec = importlib.util.spec_from_loader("nestbox_display", _loader)
display = importlib.util.module_from_spec(_spec)
_loader.exec_module(display)


XRANDR_QUERY = """\
Screen 0: minimum 32 x 32, current 1440 x 900, maximum 32768 x 32768
VNC-0 connected 1440x900+0+0 0mm x 0mm
   1440x900      60.00*+
   1920x1200     60.00  
   1920x1080     60.00  
   1600x1200     60.00  
   1680x1050     60.00  
   1400x1050     60.00  
   1360x768      60.00  
   1280x1024     60.00  
   1280x960      60.00  
   1280x800      60.00  
   1280x720      60.00  
   1024x768      60.00  
   800x600       60.00  
   640x480       60.00  
"""

XRANDR_MONITORS = """\
Monitors: 1
 0: +VNC-0 1440/381x900/238+0+0  VNC-0
"""

XRANDR_MONITORS_TWO = """\
Monitors: 2
 0: +NB-1 1440/0x900/0+0+0  VNC-0
 1: NB-2 1920/0x1080/0+1440+0 
"""


def layout_of(*monitors: dict, mode: str = "separate") -> dict:
    """A layout built from monitor dicts, in the documented shape."""
    return {"mode": mode, "monitors": list(monitors)}


def monitor(identifier: str, width: int, height: int, x: int, y: int, primary: bool = False) -> dict:
    """One monitor in the documented shape."""
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


DEFAULT = layout_of(monitor("m1", 1440, 900, 0, 0, primary=True))


class ParseTest(unittest.TestCase):
    def test_the_captured_query_gives_the_screen_size_and_the_modes(self) -> None:
        parsed = display.parse_xrandr(XRANDR_QUERY)
        self.assertEqual(parsed["screen"]["current"], [1440, 900])
        self.assertEqual(parsed["screen"]["minimum"], [32, 32])
        self.assertEqual(parsed["screen"]["maximum"], [32768, 32768])
        self.assertEqual(len(parsed["outputs"]), 1)
        output = parsed["outputs"][0]
        self.assertEqual(output["name"], "VNC-0")
        self.assertTrue(output["connected"])
        self.assertEqual((output["width"], output["height"]), (1440, 900))
        sizes = {(mode["width"], mode["height"]) for mode in output["modes"]}
        self.assertIn((1920, 1080), sizes)
        self.assertIn((1440, 900), sizes)
        self.assertEqual(len(output["modes"]), 14)

    def test_a_primary_output_is_still_one_output(self) -> None:
        parsed = display.parse_xrandr(
            "Screen 0: minimum 32 x 32, current 1366 x 768, maximum 32768 x 32768\n"
            "VNC-0 connected primary 1366x768+0+0 0mm x 0mm\n"
            "   NB-1366x768   70.00*+\n"
        )
        self.assertEqual(parsed["outputs"][0]["width"], 1366)
        self.assertEqual(parsed["outputs"][0]["modes"][0]["name"], "NB-1366x768")

    def test_parse_monitors_reads_name_size_position_and_owner(self) -> None:
        parsed = display.parse_monitors(XRANDR_MONITORS_TWO)
        self.assertEqual(len(parsed), 2)
        self.assertEqual(parsed[0]["name"], "NB-1")
        self.assertTrue(parsed[0]["primary"])
        self.assertEqual((parsed[0]["width"], parsed[0]["height"]), (1440, 900))
        self.assertEqual((parsed[0]["x"], parsed[0]["y"]), (0, 0))
        self.assertEqual(parsed[0]["output"], "VNC-0")
        self.assertEqual(parsed[1]["name"], "NB-2")
        self.assertFalse(parsed[1]["primary"])
        self.assertEqual((parsed[1]["x"], parsed[1]["y"]), (1440, 0))
        self.assertEqual(parsed[1]["output"], "")

    def test_parse_monitors_of_the_untouched_desktop(self) -> None:
        parsed = display.parse_monitors(XRANDR_MONITORS)
        self.assertEqual(len(parsed), 1)
        self.assertEqual(parsed[0]["name"], "VNC-0")

    def test_parse_layout_keeps_the_shape_and_drops_bookkeeping(self) -> None:
        text = json.dumps(
            {
                "mode": "separate",
                "monitors": [
                    {
                        "id": "m1",
                        "name": "Monitor 1",
                        "width": 1920,
                        "height": 1080,
                        "x": 0,
                        "y": 0,
                        "primary": True,
                    },
                    {"id": "m2", "name": "Monitor 2", "width": 2560, "height": 1440, "x": 1920, "y": 0},
                ],
                "applied_at": "2026-11-03T10:00:00Z",
                "applied_by": "admin",
            }
        )
        parsed = display.parse_layout(text)
        self.assertEqual(parsed["mode"], "separate")
        self.assertEqual(len(parsed["monitors"]), 2)
        self.assertEqual(parsed["monitors"][0]["primary"], True)
        self.assertNotIn("primary", parsed["monitors"][1])
        self.assertNotIn("applied_at", parsed)
        self.assertNotIn("applied_by", parsed)

    def test_parse_layout_rejects_text_that_is_not_a_layout(self) -> None:
        with self.assertRaises(ValueError):
            display.parse_layout("{not json")
        with self.assertRaises(ValueError):
            display.parse_layout("[1, 2, 3]")


class BoundingBoxTest(unittest.TestCase):
    def test_one_monitor_is_its_own_box(self) -> None:
        self.assertEqual(display.bounding_box(DEFAULT["monitors"]), (0, 0, 1440, 900))

    def test_side_by_side_monitors_add_up(self) -> None:
        monitors = [
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 2560, 1440, 1920, 0),
        ]
        self.assertEqual(display.bounding_box(monitors), (0, 0, 4480, 1440))

    def test_an_l_shape_takes_the_corner_into_the_box(self) -> None:
        monitors = [
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1280, 1024, 0, 1080),
        ]
        self.assertEqual(display.bounding_box(monitors), (0, 0, 1920, 2104))

    def test_normalize_moves_the_arrangement_to_the_origin(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 1920, 100),
            monitor("m2", 1280, 1024, 3840, 100),
        )
        normalized = display.normalize_layout(layout)
        self.assertEqual((normalized["monitors"][0]["x"], normalized["monitors"][0]["y"]), (0, 0))
        self.assertEqual((normalized["monitors"][1]["x"], normalized["monitors"][1]["y"]), (1920, 0))
        self.assertEqual(display.bounding_box(normalized["monitors"]), (0, 0, 3200, 1080))

    def test_normalize_names_a_monitor_that_arrives_without_one(self) -> None:
        normalized = display.normalize_layout(
            layout_of({"id": "m1", "width": 1440, "height": 900, "x": 0, "y": 0})
        )
        self.assertEqual(normalized["monitors"][0]["name"], "Monitor 1")


class ValidateTest(unittest.TestCase):
    def test_the_default_layout_is_valid(self) -> None:
        self.assertEqual(display.validate(DEFAULT), [])

    def test_two_monitors_side_by_side_are_valid(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 0, 0, primary=True),
            monitor("m2", 2560, 1440, 1920, 0),
        )
        self.assertEqual(display.validate(layout), [])

    def test_the_documented_example_is_valid(self) -> None:
        self.assertEqual(
            display.validate(
                display.parse_layout(
                    '{"mode":"separate","monitors":['
                    '{"id":"m1","name":"Monitor 1","width":1920,"height":1080,"x":0,"y":0,"primary":true},'
                    '{"id":"m2","name":"Monitor 2","width":2560,"height":1440,"x":1920,"y":0}]}'
                )
            ),
            [],
        )

    def test_a_layout_needs_between_one_and_eight_monitors(self) -> None:
        self.assertIn(
            "a layout needs 1 to 8 monitors, not 0",
            display.validate(layout_of()),
        )
        nine = [monitor(f"m{n}", 640, 480, (n - 1) * 640, 0) for n in range(1, 10)]
        self.assertIn(
            "a layout needs 1 to 8 monitors, not 9",
            display.validate(layout_of(*nine)),
        )

    def test_each_side_is_between_320_and_7680(self) -> None:
        self.assertIn(
            "m1: width must be a whole number from 320 to 7680",
            display.validate(layout_of(monitor("m1", 300, 900, 0, 0))),
        )
        self.assertIn(
            "m1: height must be a whole number from 320 to 7680",
            display.validate(layout_of(monitor("m1", 1440, 8000, 0, 0))),
        )
        self.assertEqual(display.validate(layout_of(monitor("m1", 320, 320, 0, 0))), [])
        self.assertEqual(display.validate(layout_of(monitor("m1", 7680, 4320, 0, 0))), [])

    def test_the_mode_has_to_be_known(self) -> None:
        self.assertIn(
            'mode must be "separate" or "stretched"',
            display.validate(layout_of(monitor("m1", 1440, 900, 0, 0), mode="mirror")),
        )

    def test_negative_positions_are_refused(self) -> None:
        self.assertIn(
            "m1: x must be 0 or more",
            display.validate(layout_of(monitor("m1", 1440, 900, -1, 0))),
        )
        self.assertIn(
            "m1: y must be 0 or more",
            display.validate(layout_of(monitor("m1", 1440, 900, 0, -1))),
        )

    def test_ids_are_required_and_unique(self) -> None:
        self.assertIn(
            "monitor 1 needs an id",
            display.validate(layout_of({"id": "", "width": 1440, "height": 900, "x": 0, "y": 0})),
        )
        self.assertIn(
            "monitor id m1 is used twice",
            display.validate(
                layout_of(
                    monitor("m1", 1440, 900, 0, 0),
                    monitor("m1", 1440, 900, 1440, 0),
                )
            ),
        )

    def test_only_one_monitor_can_be_primary(self) -> None:
        self.assertIn(
            "only one monitor can be primary",
            display.validate(
                layout_of(
                    monitor("m1", 1440, 900, 0, 0, primary=True),
                    monitor("m2", 1440, 900, 1440, 0, primary=True),
                )
            ),
        )

    def test_monitors_may_not_overlap(self) -> None:
        errors = display.validate(
            layout_of(
                monitor("m1", 1920, 1080, 0, 0),
                monitor("m2", 1280, 1024, 1000, 100),
            )
        )
        self.assertIn("monitors m1 and m2 overlap", errors)

    def test_a_monitor_has_to_touch_another_one(self) -> None:
        errors = display.validate(
            layout_of(
                monitor("m1", 1920, 1080, 0, 0),
                monitor("m2", 1280, 1024, 2000, 0),
            )
        )
        self.assertIn("monitor m2 must touch another monitor at an edge", errors)

    def test_a_corner_contact_is_not_a_touch(self) -> None:
        errors = display.validate(
            layout_of(
                monitor("m1", 1920, 1080, 0, 0),
                monitor("m2", 1280, 1024, 1920, 1080),
            )
        )
        self.assertIn("monitor m2 must touch another monitor at an edge", errors)

    def test_the_bounding_box_is_limited(self) -> None:
        errors = display.validate(
            layout_of(
                monitor("m1", 7680, 4320, 0, 0),
                monitor("m2", 7680, 4320, 7680, 0),
                monitor("m3", 7680, 4320, 15360, 0),
            )
        )
        self.assertTrue(any("above the 16384 x 8640 limit" in error for error in errors), errors)

    def test_the_total_area_is_limited(self) -> None:
        four_k = [monitor(f"m{n}", 3840, 2160, (n - 1) * 3840, 0) for n in range(1, 5)]
        self.assertEqual(display.validate(layout_of(*four_k)), [])
        five_k = four_k + [monitor("m5", 3840, 2160, 0, 2160)]
        errors = display.validate(layout_of(*five_k))
        self.assertTrue(any("above the 36000000 pixel limit" in error for error in errors), errors)

    def test_a_single_huge_monitor_hits_the_area_limit(self) -> None:
        errors = display.validate(layout_of(monitor("m1", 7680, 7680, 0, 0)))
        self.assertTrue(any("pixel limit" in error for error in errors), errors)


class CvtTest(unittest.TestCase):
    def test_full_hd_matches_cvt_dash_r(self) -> None:
        self.assertEqual(
            display.cvt_rb(1920, 1080),
            "138.50 1920 1968 2000 2080 1080 1083 1088 1111 +hsync -vsync",
        )

    def test_four_k_matches_cvt_dash_r(self) -> None:
        self.assertEqual(
            display.cvt_rb(3840, 2160),
            "533.25 3840 3888 3920 4000 2160 2163 2168 2222 +hsync -vsync",
        )

    def test_a_small_mode_keeps_its_exact_width(self) -> None:
        # 1366 is not a multiple of the character cell an ordinary CVT mode
        # rounds to; a monitor of 1366 pixels has to stay 1366 pixels wide.
        modeline = display.cvt_rb(1366, 768).split()
        self.assertEqual(modeline[1], "1366")
        self.assertEqual(modeline[4], "1526")
        self.assertEqual(modeline[6], "771")
        self.assertGreater(int(modeline[8]), 768)


class MonitorArgsTest(unittest.TestCase):
    def test_one_monitor_owns_the_output(self) -> None:
        self.assertEqual(
            display.monitor_args(DEFAULT),
            [["--setmonitor", "NB-1", "1440/0x900/0+0+0", "VNC-0"]],
        )

    def test_the_primary_monitor_owns_the_only_output(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1440, 900, 1920, 0, primary=True),
        )
        self.assertEqual(
            display.monitor_args(layout),
            [
                ["--setmonitor", "NB-1", "1920/0x1080/0+0+0", "none"],
                ["--setmonitor", "NB-2", "1440/0x900/0+1920+0", "VNC-0"],
            ],
        )

    def test_one_output_per_monitor_when_the_server_offers_them(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1280, 1024, 1920, 0),
        )
        self.assertEqual(
            display.monitor_args(layout, ("VNC-0", "VNC-1")),
            [
                ["--setmonitor", "NB-1", "1920/0x1080/0+0+0", "VNC-0"],
                ["--setmonitor", "NB-2", "1280/0x1024/0+1920+0", "VNC-1"],
            ],
        )

    def test_stretched_is_one_monitor_over_the_whole_box(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1280, 1024, 1920, 0),
            mode="stretched",
        )
        self.assertEqual(
            display.monitor_args(layout),
            [["--setmonitor", "NB-ALL", "3200/0x1080/0+0+0", "VNC-0"]],
        )


class AppendMonitorTest(unittest.TestCase):
    def test_right_appends_at_the_top_of_the_right_edge(self) -> None:
        layout = display.append_monitor(DEFAULT, 1920, 1080)
        self.assertEqual(len(layout["monitors"]), 2)
        added = layout["monitors"][1]
        self.assertEqual(added["id"], "m2")
        self.assertEqual(added["name"], "Monitor 2")
        self.assertEqual((added["x"], added["y"]), (1440, 0))
        self.assertEqual(display.validate(layout), [])

    def test_below_appends_at_the_left_of_the_bottom_edge(self) -> None:
        layout = display.append_monitor(DEFAULT, 1280, 1024, "below")
        added = layout["monitors"][1]
        self.assertEqual((added["x"], added["y"]), (0, 900))
        self.assertEqual(display.validate(layout), [])

    def test_two_adds_make_three_monitors_that_fit_a_single_screen(self) -> None:
        layout = display.append_monitor(display.append_monitor(DEFAULT, 1920, 1080), 1280, 1024)
        self.assertEqual(len(layout["monitors"]), 3)
        self.assertEqual(display.bounding_box(layout["monitors"]), (0, 0, 4640, 1080))
        self.assertEqual(display.validate(layout), [])

    def test_the_primary_monitor_stays_primary(self) -> None:
        layout = display.append_monitor(DEFAULT, 1920, 1080)
        self.assertEqual(layout["monitors"][0].get("primary"), True)
        self.assertNotIn("primary", layout["monitors"][1])

    def test_an_unknown_position_is_refused(self) -> None:
        with self.assertRaises(ValueError):
            display.append_monitor(DEFAULT, 1920, 1080, "left")

    def test_find_monitor_accepts_id_name_and_index(self) -> None:
        layout = layout_of(
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1440, 900, 1920, 0),
        )
        self.assertEqual(display.find_monitor(layout, "m2")["id"], "m2")
        self.assertEqual(display.find_monitor(layout, "Monitor 2")["id"], "m2")
        self.assertEqual(display.find_monitor(layout, "2")["id"], "m2")
        with self.assertRaises(display.Failure):
            display.find_monitor(layout, "m9")


if __name__ == "__main__":
    unittest.main()
