//! Panels, meter bars, gauge arcs and grid lines.

use oma_core::overlay::{Orientation, Rgba};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_OPEN, D2D_RECT_F, D2D_SIZE_F,
};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Factory, ID2D1Geometry, ID2D1RenderTarget, ID2D1SolidColorBrush, D2D1_ARC_SEGMENT,
    D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL, D2D1_ROUNDED_RECT, D2D1_SWEEP_DIRECTION_CLOCKWISE,
};
use windows_numerics::Vector2;

use super::layout::{arc_point, RectF};

/// Where a gauge's arc starts: bottom left, so the 270° arc is open at the
/// bottom.
pub const GAUGE_START_DEG: f32 = 135.0;

pub fn d2d_rect(r: RectF) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x,
        top: r.y,
        right: r.right(),
        bottom: r.bottom(),
    }
}

/// `color` with its alpha multiplied by `opacity` (0–1).
pub fn with_opacity(color: Rgba, opacity: f64) -> Rgba {
    let a = (f64::from(color.a) * opacity.clamp(0.0, 1.0)).round() as u8;
    Rgba { a, ..color }
}

/// Fills a rounded rectangle.
pub fn fill_panel(rt: &ID2D1RenderTarget, rect: RectF, radius: f32, brush: &ID2D1SolidColorBrush) {
    let rounded = D2D1_ROUNDED_RECT {
        rect: d2d_rect(rect),
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: drawing on the target between BeginDraw and EndDraw, from its
    // thread; `rounded` outlives the call.
    unsafe { rt.FillRoundedRectangle(&rounded, brush) };
}

/// The filled part of a meter in `shape`: from the left, or from the
/// bottom when vertical.
pub fn meter_bar(shape: RectF, fraction: f32, orientation: Orientation) -> RectF {
    let f = fraction.clamp(0.0, 1.0);
    match orientation {
        Orientation::Horizontal => RectF {
            w: shape.w * f,
            ..shape
        },
        Orientation::Vertical => RectF {
            y: shape.bottom() - shape.h * f,
            h: shape.h * f,
            ..shape
        },
    }
}

/// The centre, radius and stroke width of a gauge in its square `shape`.
pub fn gauge_circle(shape: RectF) -> ((f32, f32), f32, f32) {
    let stroke = (shape.w * 0.1).max(1.0);
    let center = (shape.x + shape.w / 2.0, shape.y + shape.h / 2.0);
    (center, (shape.w / 2.0 - stroke / 2.0).max(0.0), stroke)
}

/// An arc of `sweep` degrees from `start`, clockwise; `None` when empty.
pub fn arc(
    factory: &ID2D1Factory,
    center: (f32, f32),
    radius: f32,
    start: f32,
    sweep: f32,
) -> Result<Option<ID2D1Geometry>> {
    if sweep <= 0.0 || radius <= 0.0 {
        return Ok(None);
    }
    let (sx, sy) = arc_point(center, radius, start);
    let (ex, ey) = arc_point(center, radius, start + sweep);
    let segment = D2D1_ARC_SEGMENT {
        point: Vector2 { X: ex, Y: ey },
        size: D2D_SIZE_F {
            width: radius,
            height: radius,
        },
        rotationAngle: 0.0,
        sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
        arcSize: if sweep > 180.0 {
            D2D1_ARC_SIZE_LARGE
        } else {
            D2D1_ARC_SIZE_SMALL
        },
    };
    // SAFETY: a path of the target's factory; the sink is closed before the
    // geometry is returned; `segment` outlives the call.
    unsafe {
        let path = factory.CreatePathGeometry()?;
        let sink = path.Open()?;
        sink.BeginFigure(Vector2 { X: sx, Y: sy }, D2D1_FIGURE_BEGIN_HOLLOW);
        sink.AddArc(&segment);
        sink.EndFigure(D2D1_FIGURE_END_OPEN);
        sink.Close()?;
        Ok(Some(path.into()))
    }
}

/// `count` horizontal lines splitting `rect` into equal bands.
pub fn grid_lines(rt: &ID2D1RenderTarget, rect: RectF, count: u8, brush: &ID2D1SolidColorBrush) {
    let n = f32::from(count);
    for i in 1..=count {
        let y = (rect.y + rect.h * f32::from(i) / (n + 1.0)).round() + 0.5;
        // SAFETY: drawing on the target between BeginDraw and EndDraw.
        unsafe {
            rt.DrawLine(
                Vector2 { X: rect.x, Y: y },
                Vector2 {
                    X: rect.right(),
                    Y: y,
                },
                brush,
                1.0,
                None,
            )
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_bar_fills_from_left_or_bottom() {
        let s = RectF {
            x: 10.0,
            y: 0.0,
            w: 100.0,
            h: 40.0,
        };
        assert_eq!(
            meter_bar(s, 0.25, Orientation::Horizontal),
            RectF { w: 25.0, ..s }
        );
        assert_eq!(
            meter_bar(s, 0.25, Orientation::Vertical),
            RectF {
                y: 30.0,
                h: 10.0,
                ..s
            }
        );
        assert_eq!(meter_bar(s, 2.0, Orientation::Horizontal), s);
    }

    #[test]
    fn opacity_scales_alpha() {
        assert_eq!(with_opacity(Rgba::BLACK, 0.5).a, 128);
        assert_eq!(with_opacity(Rgba::rgb(1, 2, 3), 2.0), Rgba::rgb(1, 2, 3));
    }
}
