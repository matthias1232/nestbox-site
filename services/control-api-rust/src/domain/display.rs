//! The display layout of a desktop sandbox: 1 to 8 monitors of any size, either
//! separate screens or one stretched screen (U09/U10a).
//!
//! This is the same document `nestbox-display` inside the sandbox reads, applies
//! and writes back, and the same one the console edits. The limits are kept here
//! as well, so a layout is refused with the sentence the operator will see
//! before it ever reaches the sandbox, and the Python tool, the Rust API and the
//! console cannot silently drift apart.
//!
//! The validator mirrors `validate()` of
//! `services/tool-images/desktop/nestbox-display` one to one: same rules, same
//! order, same messages, same tests. The only differences are the ones Rust
//! makes impossible to express — a monitor that is not an object, a name that is
//! not a string, `monitors` that is not a list: serde refuses those while
//! deserialising, so no sentence is needed here.

use serde::{Deserialize, Serialize};

/// The narrowest side a monitor may have, in pixels.
pub const MIN_SIDE: i64 = 320;

/// The widest side a monitor may have: 8K wide, beyond anything the appliance
/// drives, so the limit belongs to the layout rather than to today's hardware.
pub const MAX_SIDE: i64 = 7680;

/// How many monitors one layout may hold.
pub const MAX_MONITORS: usize = 8;

/// The widest framebuffer the appliance will transmit.
pub const MAX_BOUNDING_WIDTH: i64 = 16384;

/// The tallest framebuffer the appliance will transmit.
pub const MAX_BOUNDING_HEIGHT: i64 = 8640;

/// The largest total area the monitors may cover: four 4K screens.
pub const MAX_AREA: i64 = 36_000_000;

/// Each monitor is its own screen; the panel, maximize and fullscreen treat them
/// as separate displays.
pub const MODE_SEPARATE: &str = "separate";

/// One logical monitor over the whole bounding box: maximize and fullscreen fill
/// every monitor as one big screen.
pub const MODE_STRETCHED: &str = "stretched";

/// One monitor in the layout: where it sits and how large it is.
///
/// Positions are pixels from the top-left corner of the framebuffer; the sides
/// are pixels. `primary` marks the screen windows open on; at most one monitor
/// may carry it, and none has to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Monitor {
    /// Stable identifier an operator, an agent and the console all use (`m1`).
    #[serde(default)]
    pub id: String,
    /// The name the console shows (`Monitor 1`); it is not an identifier.
    #[serde(default)]
    pub name: String,
    /// Width in pixels.
    #[serde(default)]
    pub width: i64,
    /// Height in pixels.
    #[serde(default)]
    pub height: i64,
    /// Left edge, in pixels from the framebuffer's origin.
    #[serde(default)]
    pub x: i64,
    /// Top edge, in pixels from the framebuffer's origin.
    #[serde(default)]
    pub y: i64,
    /// Whether windows open on this screen. At most one monitor has it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub primary: bool,
}

/// Whether a flag is off, so it can be left out of the JSON.
///
/// The tool writes `primary` only for the primary monitor, and a layout that
/// gained `"primary": false` on every read would compare unequal to the layout
/// that was stored, which `expected_applied_at` reads as "somebody changed it".
fn is_false(value: &bool) -> bool {
    !*value
}

/// A whole display layout: the mode and the monitors it consists of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayLayout {
    /// `separate` or `stretched`.
    #[serde(default)]
    pub mode: String,
    /// The monitors, in the order the console shows them as tabs.
    #[serde(default)]
    pub monitors: Vec<Monitor>,
}

/// A layout as the sandbox reports it: the layout plus what was applied when.
///
/// `nestbox-display get` adds the framebuffer it transmits and the bookkeeping
/// of the last apply. `expected_applied_at` on a write compares against
/// `applied_at`, which is how a change somebody else made is noticed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedDisplay {
    /// The layout itself.
    #[serde(flatten)]
    pub layout: DisplayLayout,
    /// The framebuffer VNC transmits: `[width, height]`.
    #[serde(default)]
    pub framebuffer: Option<[i64; 2]>,
    /// When this layout was last applied, as the tool wrote it (ISO 8601, UTC).
    #[serde(default)]
    pub applied_at: Option<String>,
    /// Who applied it: a display name, `agent`, or `unknown`.
    #[serde(default)]
    pub applied_by: Option<String>,
}

/// Every reason this layout cannot be applied, in the order they are found.
///
/// `Ok(())` means it can be applied. A refusal is a list rather than a first
/// error so a console can show every problem at once, and the strings are the
/// same sentences `nestbox-display` prints for the same layout.
pub fn validate(layout: &DisplayLayout) -> Result<(), Vec<String>> {
    let mut errors: Vec<String> = Vec::new();

    if layout.mode != MODE_SEPARATE && layout.mode != MODE_STRETCHED {
        errors.push(format!("mode must be \"{MODE_SEPARATE}\" or \"{MODE_STRETCHED}\""));
    }

    if layout.monitors.is_empty() || layout.monitors.len() > MAX_MONITORS {
        errors.push(format!(
            "a layout needs 1 to {MAX_MONITORS} monitors, not {}",
            layout.monitors.len()
        ));
    }

    let mut seen: Vec<&str> = Vec::new();
    let mut primaries = 0_usize;
    let mut sizes_ok = true;
    for (index, monitor) in layout.monitors.iter().enumerate() {
        let label = if monitor.id.trim().is_empty() {
            errors.push(format!("monitor {} needs an id", index + 1));
            format!("monitor {}", index + 1)
        } else {
            if seen.contains(&monitor.id.as_str()) {
                errors.push(format!("monitor id {} is used twice", monitor.id));
            }
            seen.push(monitor.id.as_str());
            monitor.id.clone()
        };

        for (field, value) in [("width", monitor.width), ("height", monitor.height)] {
            if !(MIN_SIDE..=MAX_SIDE).contains(&value) {
                let message = format!(
                    "{label}: {field} must be a whole number from {MIN_SIDE} to {MAX_SIDE}"
                );
                errors.push(message);
                sizes_ok = false;
            }
        }

        for (field, value) in [("x", monitor.x), ("y", monitor.y)] {
            if value < 0 {
                errors.push(format!("{label}: {field} must be 0 or more"));
                sizes_ok = false;
            }
        }

        if monitor.primary {
            primaries += 1;
        }
    }

    if primaries > 1 {
        errors.push("only one monitor can be primary".to_string());
    }

    // Geometry needs every size and position to be valid; a half-checked layout
    // would report overlaps between monitors that do not exist. Any other error
    // keeps it out too, exactly as the tool decides it.
    if sizes_ok && errors.is_empty() {
        errors.extend(geometry_errors(&layout.monitors));
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Overlap, gaps, bounding box and area: what a display dialog refuses.
///
/// An overlap is reported alone: once two monitors share pixels, the touching
/// rule would report every neighbour of those two as well, and the operator
/// would read five sentences for one mistake.
fn geometry_errors(monitors: &[Monitor]) -> Vec<String> {
    let mut errors = Vec::new();
    for (index, first) in monitors.iter().enumerate() {
        for second in monitors.iter().skip(index + 1) {
            if overlaps(first, second) {
                errors.push(format!("monitors {} and {} overlap", first.id, second.id));
            }
        }
    }
    if !errors.is_empty() {
        return errors;
    }

    if monitors.len() > 1 {
        for monitor in monitors {
            let touches_another = monitors
                .iter()
                .filter(|other| !std::ptr::eq(*other, monitor))
                .any(|other| touches(monitor, other));
            if !touches_another {
                errors.push(format!(
                    "monitor {} must touch another monitor at an edge",
                    monitor.id
                ));
            }
        }
    }

    let (width, height) = bounding_box(monitors);
    if width > MAX_BOUNDING_WIDTH || height > MAX_BOUNDING_HEIGHT {
        errors.push(format!(
            "the layout is {width} x {height}, above the \
             {MAX_BOUNDING_WIDTH} x {MAX_BOUNDING_HEIGHT} limit"
        ));
    }

    let area: i64 = monitors
        .iter()
        .map(|monitor| monitor.width * monitor.height)
        .sum();
    if area > MAX_AREA {
        errors.push(format!(
            "the monitors cover {area} pixels, above the {MAX_AREA} pixel limit"
        ));
    }
    errors
}

/// The smallest rectangle around every monitor: `(width, height)`.
///
/// This is the framebuffer VNC transmits. Corners a layout does not cover (an L
/// shape) stay black.
pub fn bounding_box(monitors: &[Monitor]) -> (i64, i64) {
    let left = monitors.iter().map(|monitor| monitor.x).min().unwrap_or(0);
    let top = monitors.iter().map(|monitor| monitor.y).min().unwrap_or(0);
    let right = monitors
        .iter()
        .map(|monitor| monitor.x + monitor.width)
        .max()
        .unwrap_or(0);
    let bottom = monitors
        .iter()
        .map(|monitor| monitor.y + monitor.height)
        .max()
        .unwrap_or(0);
    (right - left, bottom - top)
}

/// True when the two rectangles share any pixel.
fn overlaps(first: &Monitor, second: &Monitor) -> bool {
    first.x < second.x + second.width
        && second.x < first.x + first.width
        && first.y < second.y + second.height
        && second.y < first.y + first.height
}

/// True when a whole edge of one rectangle meets the other, like a tiling.
///
/// A corner that only touches a corner is not a touch: the monitor in the corner
/// of an L-shaped arrangement cannot be reached by dragging a window across a
/// shared edge, so the layout is refused.
fn touches(first: &Monitor, second: &Monitor) -> bool {
    let vertical_overlap =
        (first.y + first.height).min(second.y + second.height) > first.y.max(second.y);
    let horizontal_overlap =
        (first.x + first.width).min(second.x + second.width) > first.x.max(second.x);
    let beside = vertical_overlap
        && (first.x + first.width == second.x || second.x + second.width == first.x);
    let above = horizontal_overlap
        && (first.y + first.height == second.y || second.y + second.height == first.y);
    beside || above
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One monitor in the documented shape.
    fn monitor(id: &str, width: i64, height: i64, x: i64, y: i64) -> Monitor {
        Monitor {
            id: id.to_string(),
            name: format!("Monitor {id}"),
            width,
            height,
            x,
            y,
            primary: false,
        }
    }

    /// The same monitor, marked primary.
    fn primary(id: &str, width: i64, height: i64, x: i64, y: i64) -> Monitor {
        Monitor {
            primary: true,
            ..monitor(id, width, height, x, y)
        }
    }

    /// A layout in the given mode.
    fn layout_of(mode: &str, monitors: Vec<Monitor>) -> DisplayLayout {
        DisplayLayout {
            mode: mode.to_string(),
            monitors,
        }
    }

    /// The default layout of every desktop sandbox: one 1440x900 monitor.
    fn default_layout() -> DisplayLayout {
        layout_of(MODE_SEPARATE, vec![primary("m1", 1440, 900, 0, 0)])
    }

    /// The validator's refusal, so a test can look for one sentence.
    fn errors(layout: &DisplayLayout) -> Vec<String> {
        validate(layout).err().unwrap_or_default()
    }

    /// Whether the refusal holds this exact sentence.
    fn has(errors: &[String], sentence: &str) -> bool {
        errors.iter().any(|error| error == sentence)
    }

    #[test]
    fn the_default_layout_is_valid() {
        assert_eq!(validate(&default_layout()), Ok(()));
    }

    #[test]
    fn two_monitors_side_by_side_are_valid() {
        let monitors = vec![
            primary("m1", 1920, 1080, 0, 0),
            monitor("m2", 2560, 1440, 1920, 0),
        ];
        assert_eq!(validate(&layout_of(MODE_SEPARATE, monitors)), Ok(()));
    }

    #[test]
    fn the_documented_example_is_valid() {
        let json = r#"{"mode":"separate","monitors":[
            {"id":"m1","name":"Monitor 1","width":1920,"height":1080,"x":0,"y":0,"primary":true},
            {"id":"m2","name":"Monitor 2","width":2560,"height":1440,"x":1920,"y":0}]}"#;
        let layout: DisplayLayout = serde_json::from_str(json).expect("the example parses");
        assert_eq!(validate(&layout), Ok(()));
    }

    #[test]
    fn a_layout_needs_between_one_and_eight_monitors() {
        let empty = layout_of(MODE_SEPARATE, Vec::new());
        assert!(has(&errors(&empty), "a layout needs 1 to 8 monitors, not 0"));

        let nine: Vec<Monitor> = (1..=9)
            .map(|n| monitor(&format!("m{n}"), 640, 480, (n - 1) * 640, 0))
            .collect();
        let found = errors(&layout_of(MODE_SEPARATE, nine));
        assert!(has(&found, "a layout needs 1 to 8 monitors, not 9"));
    }

    #[test]
    fn each_side_is_between_320_and_7680() {
        let narrow = layout_of(MODE_SEPARATE, vec![monitor("m1", 300, 900, 0, 0)]);
        assert!(has(&errors(&narrow), "m1: width must be a whole number from 320 to 7680"));

        let low = layout_of(MODE_SEPARATE, vec![monitor("m1", 1440, 8000, 0, 0)]);
        assert!(has(&errors(&low), "m1: height must be a whole number from 320 to 7680"));

        let smallest = layout_of(MODE_SEPARATE, vec![monitor("m1", 320, 320, 0, 0)]);
        assert_eq!(validate(&smallest), Ok(()));
        let wide = layout_of(MODE_SEPARATE, vec![monitor("m1", 7680, 4320, 0, 0)]);
        assert_eq!(validate(&wide), Ok(()));
    }

    #[test]
    fn the_mode_has_to_be_known() {
        let mirror = layout_of("mirror", vec![monitor("m1", 1440, 900, 0, 0)]);
        assert!(has(&errors(&mirror), "mode must be \"separate\" or \"stretched\""));
    }

    #[test]
    fn negative_positions_are_refused() {
        let left = layout_of(MODE_SEPARATE, vec![monitor("m1", 1440, 900, -1, 0)]);
        assert!(has(&errors(&left), "m1: x must be 0 or more"));
        let up = layout_of(MODE_SEPARATE, vec![monitor("m1", 1440, 900, 0, -1)]);
        assert!(has(&errors(&up), "m1: y must be 0 or more"));
    }

    #[test]
    fn ids_are_required_and_unique() {
        let nameless = layout_of(
            MODE_SEPARATE,
            vec![Monitor {
                id: " ".to_string(),
                name: String::new(),
                width: 1440,
                height: 900,
                x: 0,
                y: 0,
                primary: false,
            }],
        );
        assert!(has(&errors(&nameless), "monitor 1 needs an id"));

        let twice = layout_of(
            MODE_SEPARATE,
            vec![monitor("m1", 1440, 900, 0, 0), monitor("m1", 1440, 900, 1440, 0)],
        );
        assert!(has(&errors(&twice), "monitor id m1 is used twice"));
    }

    #[test]
    fn only_one_monitor_can_be_primary() {
        let both = layout_of(
            MODE_SEPARATE,
            vec![primary("m1", 1440, 900, 0, 0), primary("m2", 1440, 900, 1440, 0)],
        );
        assert!(has(&errors(&both), "only one monitor can be primary"));
    }

    #[test]
    fn monitors_may_not_overlap() {
        let stacked = layout_of(
            MODE_SEPARATE,
            vec![monitor("m1", 1920, 1080, 0, 0), monitor("m2", 1280, 1024, 1000, 100)],
        );
        assert!(has(&errors(&stacked), "monitors m1 and m2 overlap"));
    }

    #[test]
    fn a_monitor_has_to_touch_another_one() {
        let gap = layout_of(
            MODE_SEPARATE,
            vec![monitor("m1", 1920, 1080, 0, 0), monitor("m2", 1280, 1024, 2000, 0)],
        );
        let found = errors(&gap);
        assert!(has(&found, "monitor m2 must touch another monitor at an edge"));
    }

    #[test]
    fn a_corner_contact_is_not_a_touch() {
        let corner = layout_of(
            MODE_SEPARATE,
            vec![monitor("m1", 1920, 1080, 0, 0), monitor("m2", 1280, 1024, 1920, 1080)],
        );
        let found = errors(&corner);
        assert!(has(&found, "monitor m2 must touch another monitor at an edge"));
    }

    #[test]
    fn the_bounding_box_is_limited() {
        let too_wide = layout_of(
            MODE_SEPARATE,
            vec![
                monitor("m1", 7680, 4320, 0, 0),
                monitor("m2", 7680, 4320, 7680, 0),
                monitor("m3", 7680, 4320, 15360, 0),
            ],
        );
        let found = errors(&too_wide);
        let named = found
            .iter()
            .any(|error| error.contains("above the 16384 x 8640 limit"));
        assert!(named, "{found:?}");
    }

    #[test]
    fn the_total_area_is_limited() {
        let four_k: Vec<Monitor> = (1..=4)
            .map(|n| monitor(&format!("m{n}"), 3840, 2160, (n - 1) * 3840, 0))
            .collect();
        assert_eq!(validate(&layout_of(MODE_SEPARATE, four_k.clone())), Ok(()));

        let mut five_k = four_k;
        five_k.push(monitor("m5", 3840, 2160, 0, 2160));
        let found = errors(&layout_of(MODE_SEPARATE, five_k));
        let named = found
            .iter()
            .any(|error| error.contains("above the 36000000 pixel limit"));
        assert!(named, "{found:?}");
    }

    #[test]
    fn a_single_huge_monitor_hits_the_area_limit() {
        let huge = layout_of(MODE_SEPARATE, vec![monitor("m1", 7680, 7680, 0, 0)]);
        let found = errors(&huge);
        assert!(found.iter().any(|error| error.contains("pixel limit")), "{found:?}");
    }

    #[test]
    fn the_box_of_an_l_shape_takes_the_corner_into_the_box() {
        let monitors = vec![
            monitor("m1", 1920, 1080, 0, 0),
            monitor("m2", 1280, 1024, 0, 1080),
        ];
        assert_eq!(bounding_box(&monitors), (1920, 2104));
    }

    #[test]
    fn a_layout_read_from_the_tool_reports_the_framebuffer() {
        let json = r#"{"mode":"stretched","monitors":[{"id":"m1","name":"Monitor 1",
            "width":3840,"height":1080,"x":0,"y":0,"primary":true}],
            "framebuffer":[3840,1080],"applied_at":"2026-11-03T10:00:00Z","applied_by":"admin"}"#;
        let applied: AppliedDisplay = serde_json::from_str(json).expect("a tool answer parses");
        assert_eq!(applied.layout.mode, MODE_STRETCHED);
        assert_eq!(applied.framebuffer, Some([3840, 1080]));
        assert_eq!(applied.applied_at.as_deref(), Some("2026-11-03T10:00:00Z"));
        assert!(applied.layout.monitors[0].primary);
    }

    #[test]
    fn a_layout_without_primary_is_written_without_the_key() {
        let plain = layout_of(MODE_SEPARATE, vec![monitor("m1", 1440, 900, 0, 0)]);
        let written = serde_json::to_string(&plain).expect("a layout serialises");
        assert!(!written.contains("primary"), "{written}");
    }
}
