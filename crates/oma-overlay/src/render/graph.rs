//! Chart geometries (§5.2): built from the chart's points only when its data
//! changes, then replayed every frame.

use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN,
};
use windows::Win32::Graphics::Direct2D::{ID2D1Factory, ID2D1Geometry, ID2D1GeometrySink};
use windows_numerics::Vector2;

use super::layout::RectF;

fn v((x, y): (f32, f32)) -> Vector2 {
    Vector2 { X: x, Y: y }
}

/// A path filled by `build`, closed and returned as a geometry.
fn path(factory: &ID2D1Factory, build: impl FnOnce(&ID2D1GeometrySink)) -> Result<ID2D1Geometry> {
    // SAFETY: a path of the target's factory; the sink is used only inside
    // `build` and closed before the geometry is returned.
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        build(&sink);
        sink.Close()?;
        Ok(path.into())
    }
}

/// The polyline through `points`; `None` with fewer than two.
pub fn line(factory: &ID2D1Factory, points: &[(f32, f32)]) -> Result<Option<ID2D1Geometry>> {
    let [first, rest @ ..] = points else {
        return Ok(None);
    };
    if rest.is_empty() {
        return Ok(None);
    }
    let rest: Vec<Vector2> = rest.iter().copied().map(v).collect();
    path(factory, |sink| {
        // SAFETY: an open sink; `rest` outlives the calls.
        unsafe {
            sink.BeginFigure(v(*first), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddLines(&rest);
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
        }
    })
    .map(Some)
}

/// The area under the polyline down to `bottom`; `None` with fewer than two
/// points.
pub fn area(
    factory: &ID2D1Factory,
    points: &[(f32, f32)],
    bottom: f32,
) -> Result<Option<ID2D1Geometry>> {
    let (Some(&first), Some(&last)) = (points.first(), points.last()) else {
        return Ok(None);
    };
    if points.len() < 2 {
        return Ok(None);
    }
    let mut outline: Vec<Vector2> = points.iter().copied().map(v).collect();
    outline.push(v((last.0, bottom)));
    outline.push(v((first.0, bottom)));
    path(factory, |sink| {
        // SAFETY: an open sink; `outline` outlives the calls.
        unsafe {
            sink.BeginFigure(outline[0], D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&outline[1..]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
    })
    .map(Some)
}

/// One filled rectangle per bar; `None` without bars.
pub fn bars(factory: &ID2D1Factory, bars: &[RectF]) -> Result<Option<ID2D1Geometry>> {
    if bars.is_empty() {
        return Ok(None);
    }
    path(factory, |sink| {
        for b in bars.iter().filter(|b| b.w > 0.0 && b.h > 0.0) {
            let corners = [
                v((b.right(), b.y)),
                v((b.right(), b.bottom())),
                v((b.x, b.bottom())),
            ];
            // SAFETY: an open sink; `corners` outlives the calls.
            unsafe {
                sink.BeginFigure(v((b.x, b.y)), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddLines(&corners);
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
        }
    })
    .map(Some)
}

/// The bars of a `bars` chart: each from the previous point to its own, from
/// its value down to the bottom of `rect`, one pixel apart when they fit.
pub fn value_bars(points: &[(f32, f32)], rect: RectF) -> Vec<RectF> {
    points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let x0 = match i {
                0 => (x - 2.0).max(rect.x),
                _ => points[i - 1].0,
            };
            let w = x - x0;
            let w = if w > 2.0 { w - 1.0 } else { w };
            RectF {
                x: x - w,
                y,
                w,
                h: rect.bottom() - y,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_bars_span_from_the_previous_point_to_the_bottom() {
        let r = RectF {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 50.0,
        };
        let b = value_bars(&[(10.0, 40.0), (20.0, 10.0)], r);
        assert_eq!(b.len(), 2);
        assert_eq!(
            b[0],
            RectF {
                x: 8.0,
                y: 40.0,
                w: 2.0,
                h: 10.0
            }
        );
        // 10 px from the previous point, one left as a gap.
        assert_eq!(
            b[1],
            RectF {
                x: 11.0,
                y: 10.0,
                w: 9.0,
                h: 40.0
            }
        );
    }
}
