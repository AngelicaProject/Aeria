//! Fitting outlines to the pixel grid of a game font's own Latin.
//!
//! The game draws its fonts as 4-bit bitmaps at fixed sizes, so the
//! position of a stem inside its pixels decides how it looks: the native
//! Latin of a size has every vertical stem equally wide and starting at the
//! same fraction of a pixel, which makes a line look even. A glyph is fitted
//! the same way: its vertical stems (pairs of straight vertical edges) are
//! moved to the native stem width and phase, its horizontal bars to the
//! native bar thickness on whole pixels, and the baseline, x-height, and
//! cap height to whole pixels. Everything between the moved edges is
//! stretched linearly, so outlines stay continuous.

use swash::zeno::{Command, Vector};

use crate::targets::PixelGrid;

/// Where the zones of a glyph are before and after fitting, in pixels above
/// the baseline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Zones {
    pub x_height: (f32, f32),
    pub cap_height: (f32, f32),
}

/// A straight edge along one axis: its position on that axis, its extent on
/// the other, and its direction along it.
#[derive(Clone, Copy, Debug)]
struct Edge {
    at: f32,
    from: f32,
    to: f32,
    rising: bool,
}

/// Straight edges of `commands`. With `vertical`, edges along y (stems);
/// otherwise along x (bars). An edge is at least `min_length` long and
/// leans by less than a tenth of its length.
fn edges(commands: &[Command], vertical: bool, min_length: f32) -> Vec<Edge> {
    let mut found = Vec::new();
    let mut current = Vector::new(0.0, 0.0);
    let mut start = current;
    for command in commands {
        let end = match *command {
            Command::MoveTo(point) => {
                current = point;
                start = point;
                continue;
            }
            Command::QuadTo(_, point) | Command::CurveTo(_, _, point) => {
                current = point;
                continue;
            }
            Command::LineTo(point) => point,
            Command::Close => start,
        };
        let (across_a, across_b, along_a, along_b) = if vertical {
            (current.x, end.x, current.y, end.y)
        } else {
            (current.y, end.y, current.x, end.x)
        };
        let length = (along_b - along_a).abs();
        if length >= min_length && (across_b - across_a).abs() < 0.1 * length {
            found.push(Edge {
                at: f32::midpoint(across_a, across_b),
                from: along_a.min(along_b),
                to: along_a.max(along_b),
                rising: along_b > along_a,
            });
        }
        current = end;
    }
    found
}

/// Whether the outer contours of `commands` run clockwise (with y up), as
/// TrueType outlines do; then ink lies right of the direction of travel.
/// The sign of the area enclosed by the on-curve points decides.
fn clockwise(commands: &[Command]) -> bool {
    let mut twice_area = 0.0;
    let mut current = Vector::new(0.0, 0.0);
    let mut start = current;
    for command in commands {
        let end = match *command {
            Command::MoveTo(point) => {
                current = point;
                start = point;
                continue;
            }
            Command::LineTo(point) | Command::QuadTo(_, point) | Command::CurveTo(_, _, point) => {
                point
            }
            Command::Close => start,
        };
        twice_area += (end.x - current.x) * (end.y + current.y);
        current = end;
    }
    twice_area > 0.0
}

/// Pairs of opposite edges no more than `max_width` apart that overlap by at
/// least half of the shorter one and enclose ink: the two sides of a stem or
/// bar, low side first. The low side of ink runs up (`low_rising`) or down;
/// two edges with a gap between them are a counter, not a stroke. Each edge
/// pairs with the nearest such edge above it.
fn strokes(mut edges: Vec<Edge>, max_width: f32, low_rising: bool) -> Vec<(f32, f32)> {
    edges.sort_by(|a, b| a.at.total_cmp(&b.at));
    let mut used = vec![false; edges.len()];
    let mut pairs = Vec::new();
    for low in 0..edges.len() {
        if used[low] || edges[low].rising != low_rising {
            continue;
        }
        let partner = (low + 1..edges.len()).find(|&high| {
            let (a, b) = (edges[low], edges[high]);
            let width = b.at - a.at;
            let overlap = a.to.min(b.to) - a.from.max(b.from);
            !used[high]
                && a.rising != b.rising
                && width > 0.2
                && width <= max_width
                && overlap >= 0.5 * (a.to - a.from).min(b.to - b.from)
        });
        if let Some(high) = partner {
            used[low] = true;
            used[high] = true;
            pairs.push((edges[low].at, edges[high].at));
        }
    }
    pairs
}

/// Sorted anchors `(before, after)` that keep the order on both sides; an
/// anchor that would cross or touch an earlier one is dropped.
fn monotonic(mut anchors: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    anchors.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut kept: Vec<(f32, f32)> = Vec::with_capacity(anchors.len());
    for anchor in anchors {
        if kept
            .last()
            .is_none_or(|last| anchor.0 > last.0 + 1e-3 && anchor.1 > last.1 + 1e-3)
        {
            kept.push(anchor);
        }
    }
    kept
}

/// Moves `value` with the anchors: linearly between two of them, and with
/// the nearest one outside them.
fn warp(value: f32, anchors: &[(f32, f32)]) -> f32 {
    let (Some(first), Some(last)) = (anchors.first(), anchors.last()) else {
        return value;
    };
    if value <= first.0 {
        return value + first.1 - first.0;
    }
    if value >= last.0 {
        return value + last.1 - last.0;
    }
    for pair in anchors.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if value <= x1 {
            return y0 + (value - x0) * (y1 - y0) / (x1 - x0);
        }
    }
    value
}

/// Horizontal anchors: every vertical stem gets the native width (a stem
/// half again as wide keeps its own) and starts at the native phase.
fn horizontal_anchors(commands: &[Command], grid: &PixelGrid, zones: &Zones) -> Vec<(f32, f32)> {
    let stem = f32::from(grid.stem_centi) / 100.0;
    let phase = f32::from(grid.stem_phase_centi) / 100.0;
    let vertical = edges(commands, true, 0.3 * zones.x_height.0);
    let mut anchors = Vec::new();
    // Ink right of an edge running up: the left side of a stem.
    for (left, right) in strokes(vertical, 2.5 * stem, clockwise(commands)) {
        let width = if right - left < 1.5 * stem {
            stem
        } else {
            right - left
        };
        let centre = f32::midpoint(left, right);
        let new_left = (centre - width / 2.0 - phase).round() + phase;
        anchors.push((left, new_left));
        anchors.push((right, new_left + width));
    }
    monotonic(anchors)
}

/// Vertical anchors: the baseline, x-height, and cap height, and every
/// horizontal bar with the native thickness, resting on a zone it touches or
/// on a whole pixel.
fn vertical_anchors(commands: &[Command], grid: &PixelGrid, zones: &Zones) -> Vec<(f32, f32)> {
    let bar = f32::from(grid.bar_centi) / 100.0;
    let mut anchors = vec![(0.0, 0.0), zones.x_height, zones.cap_height];
    let horizontal = edges(commands, false, 0.25 * zones.x_height.0);
    let near = |value: f32, zone: f32| (value - zone).abs() < 0.5;
    // Ink above an edge running left: the bottom of a bar.
    for (bottom, top) in strokes(horizontal, 2.5 * bar, !clockwise(commands)) {
        let width = if top - bottom < 1.5 * bar {
            bar
        } else {
            top - bottom
        };
        let (new_bottom, new_top) = if near(top, zones.x_height.0) {
            (zones.x_height.1 - width, zones.x_height.1)
        } else if near(top, zones.cap_height.0) {
            (zones.cap_height.1 - width, zones.cap_height.1)
        } else if near(bottom, 0.0) {
            (0.0, width)
        } else {
            let new_bottom = (f32::midpoint(bottom, top) - width / 2.0).round();
            (new_bottom, new_bottom + width)
        };
        anchors.push((bottom, new_bottom));
        anchors.push((top, new_top));
    }
    monotonic(anchors)
}

/// Fits `commands` (pixels, y up, origin on the pen and baseline) to the
/// grid in place. Returns how far the right side moved, which the advance
/// follows so the space after the glyph stays as designed.
pub(crate) fn fit(commands: &mut [Command], grid: &PixelGrid, zones: &Zones, advance: f32) -> f32 {
    let across = horizontal_anchors(commands, grid, zones);
    let up = vertical_anchors(commands, grid, zones);
    let move_point = |point: Vector| Vector::new(warp(point.x, &across), warp(point.y, &up));
    for command in commands.iter_mut() {
        *command = map_points(*command, move_point);
    }
    warp(advance, &across) - advance
}

/// Applies `f` to every point of a path command.
pub(crate) fn map_points(command: Command, f: impl Fn(Vector) -> Vector) -> Command {
    match command {
        Command::MoveTo(point) => Command::MoveTo(f(point)),
        Command::LineTo(point) => Command::LineTo(f(point)),
        Command::QuadTo(control, point) => Command::QuadTo(f(control), f(point)),
        Command::CurveTo(first, second, point) => Command::CurveTo(f(first), f(second), f(point)),
        Command::Close => Command::Close,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(left: f32, bottom: f32, right: f32, top: f32) -> Vec<Command> {
        // Clockwise, as TrueType outer contours are.
        vec![
            Command::MoveTo(Vector::new(left, bottom)),
            Command::LineTo(Vector::new(left, top)),
            Command::LineTo(Vector::new(right, top)),
            Command::LineTo(Vector::new(right, bottom)),
            Command::Close,
        ]
    }

    fn points(commands: &[Command]) -> Vec<Vector> {
        commands
            .iter()
            .filter_map(|command| match *command {
                Command::MoveTo(point) | Command::LineTo(point) => Some(point),
                _ => None,
            })
            .collect()
    }

    const GRID: PixelGrid = PixelGrid {
        x_height: 8,
        stem_centi: 140,
        stem_phase_centi: 40,
        bar_centi: 100,
        bearing_split_centi: 0,
    };
    const ZONES: Zones = Zones {
        x_height: (8.3, 8.0),
        cap_height: (11.0, 11.0),
    };

    #[test]
    fn a_stem_takes_the_native_width_and_phase() {
        let mut stem = rectangle(2.15, 0.0, 3.35, 8.3);
        let moved = fit(&mut stem, &GRID, &ZONES, 5.0);
        let xs: Vec<f32> = points(&stem).iter().map(|point| point.x).collect();
        let (left, right) = (xs[0], xs[2]);
        assert!((left - 2.4).abs() < 1e-4, "{left}");
        assert!((right - 3.8).abs() < 1e-4, "{right}");
        assert!((moved - 0.45).abs() < 1e-4, "{moved}");
        // The top follows the x-height.
        assert!((points(&stem)[1].y - 8.0).abs() < 1e-4);
    }

    #[test]
    fn a_bar_rests_on_a_whole_pixel() {
        let mut bar = rectangle(0.0, 3.7, 6.0, 4.6);
        fit(&mut bar, &GRID, &ZONES, 6.0);
        let ys: Vec<f32> = points(&bar).iter().map(|point| point.y).collect();
        assert!(
            (ys[0] - 4.0).abs() < 1e-4 && (ys[1] - 5.0).abs() < 1e-4,
            "{ys:?}"
        );
    }

    #[test]
    fn the_gap_between_two_stems_is_not_a_stem() {
        // Two stems 1.2 px wide with a 2 px counter between them.
        let mut stems = rectangle(1.0, 0.0, 2.2, 8.3);
        stems.extend(rectangle(4.2, 0.0, 5.4, 8.3));
        fit(&mut stems, &GRID, &ZONES, 7.0);
        let xs: Vec<f32> = points(&stems).iter().map(|point| point.x).collect();
        let widths = [xs[2] - xs[0], xs[6] - xs[4]];
        assert!(
            widths.iter().all(|width| (width - 1.4).abs() < 1e-4),
            "{xs:?}"
        );
        assert!(
            xs.iter()
                .step_by(4)
                .all(|left| (left.fract() - 0.4).abs() < 1e-4),
            "{xs:?}"
        );
    }

    #[test]
    fn outlines_without_straight_edges_only_follow_the_zones() {
        let mut curve = vec![
            Command::MoveTo(Vector::new(1.0, 0.0)),
            Command::QuadTo(Vector::new(5.0, 4.0), Vector::new(1.0, 8.3)),
            Command::Close,
        ];
        fit(&mut curve, &GRID, &ZONES, 4.0);
        let Command::QuadTo(_, end) = curve[1] else {
            panic!("quad");
        };
        assert!((end.x - 1.0).abs() < 1e-4 && (end.y - 8.0).abs() < 1e-4);
    }
}
