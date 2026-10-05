//! Cell and pixel geometry of the overlay panel.

use super::profile::{Anchor, Block, CellRect, Profile, CELL_PX};

/// Integer pixel rectangle (physical pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// The foreground window as seen by the window watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Foreground {
    pub pid: u32,
    pub hwnd: isize,
}

/// Geometry and state of a watched window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowGeometry {
    pub client: PxRect,
    pub monitor: PxRect,
    pub dpi: u32,
    pub minimized: bool,
    pub visible: bool,
}

/// Size of one layout cell in physical pixels.
pub fn cell_px(scale: f64, dpi: u32) -> f64 {
    CELL_PX * scale * f64::from(dpi) / 96.0
}

/// Union of the block rectangles, `None` without blocks.
pub fn footprint(blocks: &[Block]) -> Option<CellRect> {
    let mut it = blocks.iter().map(|b| b.rect);
    let first = it.next()?;
    let (mut x0, mut y0) = (first.x, first.y);
    let (mut x1, mut y1) = (first.x + first.w as i32, first.y + first.h as i32);
    for r in it {
        x0 = x0.min(r.x);
        y0 = y0.min(r.y);
        x1 = x1.max(r.x + r.w as i32);
        y1 = y1.max(r.y + r.h as i32);
    }
    Some(CellRect {
        x: x0,
        y: y0,
        w: (x1 - x0) as u32,
        h: (y1 - y0) as u32,
    })
}

/// Anchored top-left corner and size of a `w` x `h` box, before clamping.
fn anchored(profile: &Profile, area: PxRect, w: i32, h: i32, cell: f64) -> PxRect {
    let ox = (f64::from(profile.offset.x) * cell).round() as i32;
    let oy = (f64::from(profile.offset.y) * cell).round() as i32;
    use Anchor::*;
    let x = match profile.anchor {
        TopLeft | Left | BottomLeft => area.x.saturating_add(ox),
        Top | Center | Bottom => area.x + (area.w - w) / 2,
        TopRight | Right | BottomRight => area.x.saturating_add(area.w - w).saturating_sub(ox),
    };
    let y = match profile.anchor {
        TopLeft | Top | TopRight => area.y.saturating_add(oy),
        Left | Center | Right => area.y + (area.h - h) / 2,
        BottomLeft | Bottom | BottomRight => area.y.saturating_add(area.h - h).saturating_sub(oy),
    };
    PxRect { x, y, w, h }
}

fn clamp_rect(r: PxRect, area: PxRect) -> PxRect {
    PxRect {
        x: clamp_axis(r.x, r.w, area.x, area.w),
        y: clamp_axis(r.y, r.h, area.y, area.h),
        ..r
    }
}

fn panel_size(profile: &Profile, dpi: u32) -> Option<(i32, i32, f64)> {
    let f = footprint(&profile.blocks)?;
    let cell = cell_px(profile.scale, dpi);
    let pad = f64::from(profile.panel.padding);
    let w = ((f64::from(f.w) + 2.0 * pad) * cell).round() as i32;
    let h = ((f64::from(f.h) + 2.0 * pad) * cell).round() as i32;
    Some((w, h, cell))
}

/// Pixel rectangle of the whole panel (footprint plus padding on every side)
/// anchored in `area`, with the profile offset in cells pointing inward and
/// the result kept inside `area`. The offset is ignored on a centred axis.
pub fn place(profile: &Profile, area: PxRect, dpi: u32) -> Option<PxRect> {
    let (w, h, cell) = panel_size(profile, dpi)?;
    Some(clamp_rect(anchored(profile, area, w, h, cell), area))
}

/// Result of [`place_with_extra`]: the overlay window and the rectangles inside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub window: PxRect,
    pub profile: Option<PxRect>,
    pub extra: Option<PxRect>,
}

/// Like [`place`], with an optional extra box of `extra_cells` (the benchmark
/// box) attached to the profile: below it for top and middle anchors, above it
/// for bottom anchors, aligned to the anchor side. The window is the union of
/// both, kept inside `area` by moving everything together. Without blocks the
/// box alone is placed at the anchor point; `None` if there is nothing to place.
pub fn place_with_extra(
    profile: &Profile,
    area: PxRect,
    dpi: u32,
    extra_cells: Option<(u32, u32)>,
) -> Option<Placed> {
    let Some((ew, eh)) = extra_cells else {
        let r = place(profile, area, dpi)?;
        return Some(Placed {
            window: r,
            profile: Some(r),
            extra: None,
        });
    };
    let ecell = cell_px(profile.scale, dpi);
    let (ew, eh) = (
        (f64::from(ew) * ecell).round() as i32,
        (f64::from(eh) * ecell).round() as i32,
    );
    let Some((w, h, cell)) = panel_size(profile, dpi) else {
        let e = clamp_rect(anchored(profile, area, ew, eh, ecell), area);
        return Some(Placed {
            window: e,
            profile: None,
            extra: Some(e),
        });
    };
    let p = anchored(profile, area, w, h, cell);
    use Anchor::*;
    let x = match profile.anchor {
        TopLeft | Left | BottomLeft => p.x,
        Top | Center | Bottom => p.x + (p.w - ew) / 2,
        TopRight | Right | BottomRight => p.x + p.w - ew,
    };
    let y = match profile.anchor {
        BottomLeft | Bottom | BottomRight => p.y - eh,
        _ => p.y + p.h,
    };
    let e = PxRect { x, y, w: ew, h: eh };
    let (x0, y0) = (p.x.min(e.x), p.y.min(e.y));
    let (x1, y1) = ((p.x + p.w).max(e.x + e.w), (p.y + p.h).max(e.y + e.h));
    let u = PxRect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    };
    let window = clamp_rect(u, area);
    let (dx, dy) = (window.x - u.x, window.y - u.y);
    let shift = |r: PxRect| PxRect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    };
    Some(Placed {
        window,
        profile: Some(shift(p)),
        extra: Some(shift(e)),
    })
}

fn clamp_axis(pos: i32, len: i32, start: i32, span: i32) -> i32 {
    let max = start + (span - len).max(0);
    pos.clamp(start, max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::profile::{Anchor, Block, CellPoint, Profile};

    fn block(x: i32, y: i32, w: u32, h: u32) -> Block {
        serde_json::from_value(serde_json::json!({
            "id": format!("b{x}-{y}"),
            "rect": {"x": x, "y": y, "w": w, "h": h},
            "source": {"text": "t"},
            "kind": "text"
        }))
        .unwrap()
    }

    fn profile(anchor: Anchor, scale: f64, offset: (i32, i32), padding: u32) -> Profile {
        let mut p: Profile = serde_json::from_str(r#"{"format":1,"name":"t"}"#).unwrap();
        p.anchor = anchor;
        p.scale = scale;
        p.offset = CellPoint {
            x: offset.0,
            y: offset.1,
        };
        p.panel.padding = padding;
        p.blocks = vec![block(0, 0, 10, 4)];
        p
    }

    const AREA: PxRect = PxRect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    #[test]
    fn footprint_is_the_union_of_rects() {
        assert_eq!(footprint(&[]), None);
        let f = footprint(&[block(2, 3, 4, 2), block(10, 1, 5, 1), block(0, 8, 1, 1)]).unwrap();
        assert_eq!((f.x, f.y, f.w, f.h), (0, 1, 15, 8));
    }

    #[test]
    fn place_anchors_nine_points() {
        // 10x4 cells + 1 cell padding per side = 96x48 px at 96 dpi; offset 1 cell = 8 px.
        // Offset is ignored on a centred axis.
        let table = [
            (Anchor::TopLeft, (8, 8)),
            (Anchor::Top, (912, 8)),
            (Anchor::TopRight, (1816, 8)),
            (Anchor::Left, (8, 516)),
            (Anchor::Center, (912, 516)),
            (Anchor::Right, (1816, 516)),
            (Anchor::BottomLeft, (8, 1024)),
            (Anchor::Bottom, (912, 1024)),
            (Anchor::BottomRight, (1816, 1024)),
        ];
        for (anchor, (x, y)) in table {
            let r = place(&profile(anchor, 1.0, (1, 1), 1), AREA, 96).unwrap();
            assert_eq!(r, PxRect { x, y, w: 96, h: 48 }, "{anchor:?}");
        }
    }

    #[test]
    fn place_scales_with_scale_and_dpi() {
        assert_eq!(cell_px(1.5, 144), 18.0);
        let mut p = profile(Anchor::TopLeft, 1.5, (0, 0), 0);
        p.blocks = vec![block(0, 0, 1, 1)];
        let area = PxRect {
            x: 100,
            y: 50,
            w: 800,
            h: 600,
        };
        assert_eq!(
            place(&p, area, 144),
            Some(PxRect {
                x: 100,
                y: 50,
                w: 18,
                h: 18
            })
        );
    }

    #[test]
    fn place_clamps_inside_area() {
        // Offset of 1000 cells pushes far out; the panel stays inside.
        let r = place(&profile(Anchor::TopLeft, 1.0, (1000, 1000), 1), AREA, 96).unwrap();
        assert_eq!(
            r,
            PxRect {
                x: 1920 - 96,
                y: 1080 - 48,
                w: 96,
                h: 48
            }
        );
        let r = place(&profile(Anchor::TopLeft, 1.0, (-1000, -1000), 1), AREA, 96).unwrap();
        assert_eq!((r.x, r.y), (0, 0));
        // Panel larger than the area: pinned to the origin.
        let small = PxRect {
            x: 10,
            y: 20,
            w: 50,
            h: 30,
        };
        let r = place(&profile(Anchor::Center, 1.0, (0, 0), 1), small, 96).unwrap();
        assert_eq!((r.x, r.y), (10, 20));
        let mut p = profile(Anchor::Center, 1.0, (0, 0), 1);
        p.blocks.clear();
        assert_eq!(place(&p, AREA, 96), None);
    }

    fn extra(anchor: Anchor, offset: (i32, i32), cells: (u32, u32)) -> Placed {
        place_with_extra(&profile(anchor, 1.0, offset, 1), AREA, 96, Some(cells)).unwrap()
    }

    #[test]
    fn extra_box_goes_below_top_anchors_and_above_bottom_anchors() {
        for a in [
            Anchor::TopLeft,
            Anchor::Top,
            Anchor::TopRight,
            Anchor::Left,
            Anchor::Center,
            Anchor::Right,
        ] {
            let pl = extra(a, (1, 1), (10, 2));
            let (p, e) = (pl.profile.unwrap(), pl.extra.unwrap());
            assert_eq!(e.y, p.y + p.h, "{a:?}");
        }
        for a in [Anchor::BottomLeft, Anchor::Bottom, Anchor::BottomRight] {
            let pl = extra(a, (1, 1), (10, 2));
            let (p, e) = (pl.profile.unwrap(), pl.extra.unwrap());
            assert_eq!(e.y + e.h, p.y, "{a:?}");
        }
    }

    #[test]
    fn extra_box_aligns_with_the_anchor_side() {
        let l = extra(Anchor::TopLeft, (1, 1), (10, 2));
        assert_eq!(l.extra.unwrap().x, l.profile.unwrap().x);
        let c = extra(Anchor::Top, (1, 1), (10, 2));
        assert_eq!(c.extra.unwrap().x, c.profile.unwrap().x + (96 - 80) / 2);
        let r = extra(Anchor::TopRight, (1, 1), (10, 2));
        let (p, e) = (r.profile.unwrap(), r.extra.unwrap());
        assert_eq!(e.x + e.w, p.x + p.w);
    }

    #[test]
    fn window_is_the_union_kept_inside_the_area() {
        let area = PxRect {
            x: 0,
            y: 0,
            w: 200,
            h: 100,
        };
        let pl = place_with_extra(
            &profile(Anchor::Center, 1.0, (0, 0), 1),
            area,
            96,
            Some((10, 4)),
        )
        .unwrap();
        assert_eq!(
            pl.window,
            PxRect {
                x: 52,
                y: 20,
                w: 96,
                h: 80
            }
        );
        assert_eq!(pl.profile.unwrap().y, 20);
        assert_eq!(pl.extra.unwrap().y, 68);
        let none = place_with_extra(&profile(Anchor::Top, 1.0, (0, 0), 1), AREA, 96, None).unwrap();
        assert_eq!(Some(none.window), none.profile);
        assert_eq!(none.extra, None);
    }

    #[test]
    fn empty_profile_with_extra_places_the_box_alone() {
        let mut p = profile(Anchor::BottomRight, 1.0, (2, 3), 1);
        p.blocks.clear();
        assert_eq!(place_with_extra(&p, AREA, 96, None), None);
        let pl = place_with_extra(&p, AREA, 96, Some((10, 2))).unwrap();
        let e = PxRect {
            x: 1824,
            y: 1040,
            w: 80,
            h: 16,
        };
        assert_eq!((pl.window, pl.profile, pl.extra), (e, None, Some(e)));
    }

    #[test]
    fn geometry_cases_fixture_matches() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/overlay/geometry-cases.json"
        );
        let cases: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let rect = |v: &serde_json::Value| {
            (!v.is_null()).then(|| PxRect {
                x: v["x"].as_i64().unwrap() as i32,
                y: v["y"].as_i64().unwrap() as i32,
                w: v["w"].as_i64().unwrap() as i32,
                h: v["h"].as_i64().unwrap() as i32,
            })
        };
        let list = cases.as_array().unwrap();
        assert!(list.len() >= 14);
        for c in list {
            let name = c["name"].as_str().unwrap();
            let prof = crate::overlay::profile::parse_profile(&c["profile"].to_string()).unwrap();
            let cells = c["extraCells"]
                .as_array()
                .map(|a| (a[0].as_u64().unwrap() as u32, a[1].as_u64().unwrap() as u32));
            let got = place_with_extra(
                &prof,
                rect(&c["area"]).unwrap(),
                c["dpi"].as_u64().unwrap() as u32,
                cells,
            )
            .unwrap();
            let exp = &c["expected"];
            assert_eq!(Some(got.window), rect(&exp["window"]), "{name} window");
            assert_eq!(got.profile, rect(&exp["profile"]), "{name} profile");
            assert_eq!(got.extra, rect(&exp["extra"]), "{name} extra");
        }
    }
}
