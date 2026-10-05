//! Text with DirectWrite (spec §5.2). Plain text is drawn with
//! `DrawTextLayout`; text with an outline or a shadow (DP5) becomes one
//! Direct2D geometry built by a custom `IDWriteTextRenderer`, which turns
//! every glyph run into its outline (`GetGlyphRunOutline`). The geometry is
//! kept until the text or its style change: the shadow is the same geometry
//! translated, the outline a stroke of `2 × width` under the fill.

use std::cell::RefCell;

use oma_core::overlay::{Rgba, TextStyle};
use windows::core::{w, Result, BOOL, PCWSTR};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, E_UNEXPECTED};
use windows::Win32::Graphics::Direct2D::Common::D2D1_FILL_MODE_WINDING;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Factory, ID2D1Geometry, ID2D1RenderTarget, D2D1_DRAW_TEXT_OPTIONS_NONE,
};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFontCollection, IDWriteInlineObject, IDWritePixelSnapping_Impl,
    IDWriteTextFormat, IDWriteTextLayout, IDWriteTextRenderer, IDWriteTextRenderer_Impl,
    DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_ITALIC, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_GLYPH_RUN, DWRITE_GLYPH_RUN_DESCRIPTION, DWRITE_LINE_METRICS,
    DWRITE_MATRIX, DWRITE_MEASURING_MODE, DWRITE_STRIKETHROUGH, DWRITE_TEXT_METRICS,
    DWRITE_UNDERLINE, DWRITE_WORD_WRAPPING_NO_WRAP,
};
use windows_core::{implement, ComObject, IUnknown, Ref};
use windows_numerics::{Matrix3x2, Vector2};

use super::layout::{style_px, TextBox};

/// The longest side DirectWrite lays a text out in; the texts are one line.
const MAX_EXTENT: f32 = 16384.0;

/// A text format for `style` at `px` pixels.
pub fn create_format(
    dwrite: &IDWriteFactory,
    style: &TextStyle,
    px: f32,
) -> Result<IDWriteTextFormat> {
    let family: Vec<u16> = style.font.encode_utf16().chain(Some(0)).collect();
    let italic = if style.italic {
        DWRITE_FONT_STYLE_ITALIC
    } else {
        DWRITE_FONT_STYLE_NORMAL
    };
    // SAFETY: `family` is NUL-terminated and outlives the call; no custom
    // collection (system fonts, §5.2); the format is returned owned.
    unsafe {
        dwrite.CreateTextFormat(
            PCWSTR(family.as_ptr()),
            None::<&IDWriteFontCollection>,
            DWRITE_FONT_WEIGHT(i32::from(style.weight)),
            italic,
            DWRITE_FONT_STRETCH_NORMAL,
            px.max(1.0),
            w!(""),
        )
    }
}

/// The metrics of the first line of `layout`. A text with line breaks has
/// more than one: the call is repeated with room for all of them.
fn first_line(layout: &IDWriteTextLayout) -> Result<DWRITE_LINE_METRICS> {
    let mut lines = vec![DWRITE_LINE_METRICS::default()];
    let mut count = 0u32;
    // SAFETY: `lines` is a live buffer of the length passed; `count` is a
    // local out-parameter.
    let first = unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count) };
    if let Err(e) = first {
        if e.code() != ERROR_INSUFFICIENT_BUFFER.to_hresult() || count == 0 {
            return Err(e);
        }
        lines.resize(count as usize, DWRITE_LINE_METRICS::default());
        // SAFETY: as above, with room for `count` lines.
        unsafe { layout.GetLineMetrics(Some(&mut lines), &mut count)? };
    }
    Ok(lines[0])
}

/// Collects the outline of every glyph run of a layout as geometries placed
/// at their baseline origin. Used only inside one `IDWriteTextLayout::Draw`
/// call on this thread, so the `RefCell` is never borrowed twice.
#[implement(IDWriteTextRenderer)]
struct OutlineCollector {
    factory: ID2D1Factory,
    geometries: RefCell<Vec<Option<ID2D1Geometry>>>,
}

impl IDWritePixelSnapping_Impl for OutlineCollector_Impl {
    fn IsPixelSnappingDisabled(&self, _context: *const core::ffi::c_void) -> Result<BOOL> {
        // A geometry is not snapped to pixels.
        Ok(true.into())
    }

    fn GetCurrentTransform(
        &self,
        _context: *const core::ffi::c_void,
        transform: *mut DWRITE_MATRIX,
    ) -> Result<()> {
        if transform.is_null() {
            return Err(E_UNEXPECTED.into());
        }
        // SAFETY: DirectWrite passes a valid out-pointer (checked non-null).
        unsafe {
            transform.write(DWRITE_MATRIX {
                m11: 1.0,
                m12: 0.0,
                m21: 0.0,
                m22: 1.0,
                dx: 0.0,
                dy: 0.0,
            })
        };
        Ok(())
    }

    fn GetPixelsPerDip(&self, _context: *const core::ffi::c_void) -> Result<f32> {
        // Layouts are built in pixels.
        Ok(1.0)
    }
}

impl IDWriteTextRenderer_Impl for OutlineCollector_Impl {
    fn DrawGlyphRun(
        &self,
        _context: *const core::ffi::c_void,
        x: f32,
        y: f32,
        _mode: DWRITE_MEASURING_MODE,
        run: *const DWRITE_GLYPH_RUN,
        _description: *const DWRITE_GLYPH_RUN_DESCRIPTION,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        // SAFETY: DirectWrite passes a glyph run valid for this call; it is
        // only read here, and nothing of it is kept after the call.
        let run =
            unsafe { run.as_ref() }.ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
        // Borrowed: the `ManuallyDrop` is neither dropped nor released.
        let face = run
            .fontFace
            .as_ref()
            .ok_or_else(|| windows::core::Error::from(E_UNEXPECTED))?;
        if run.glyphCount == 0 {
            return Ok(());
        }
        // SAFETY: the factory is the render target's; the glyph arrays are
        // the run's, `glyphCount` long (advances always, offsets may be
        // null), valid for this call; the sink is ours and closed before the
        // path is used. The outline is relative to the baseline origin, so
        // the path is translated there.
        unsafe {
            let path = self.factory.CreatePathGeometry()?;
            let sink = path.Open()?;
            let offsets = (!run.glyphOffsets.is_null()).then_some(run.glyphOffsets);
            let outlined = face.GetGlyphRunOutline(
                run.fontEmSize,
                run.glyphIndices,
                Some(run.glyphAdvances),
                offsets,
                run.glyphCount,
                run.isSideways.as_bool(),
                run.bidiLevel % 2 == 1,
                &sink,
            );
            // The sink is closed in any case; the first error wins.
            let closed = sink.Close();
            outlined.and(closed)?;
            let placed = self
                .factory
                .CreateTransformedGeometry(&path, &Matrix3x2::translation(x, y))?;
            self.geometries.borrow_mut().push(Some(placed.into()));
        }
        Ok(())
    }

    fn DrawUnderline(
        &self,
        _context: *const core::ffi::c_void,
        _x: f32,
        _y: f32,
        _underline: *const DWRITE_UNDERLINE,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawStrikethrough(
        &self,
        _context: *const core::ffi::c_void,
        _x: f32,
        _y: f32,
        _strikethrough: *const DWRITE_STRIKETHROUGH,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }

    fn DrawInlineObject(
        &self,
        _context: *const core::ffi::c_void,
        _x: f32,
        _y: f32,
        _object: Ref<IDWriteInlineObject>,
        _sideways: BOOL,
        _rtl: BOOL,
        _effect: Ref<IUnknown>,
    ) -> Result<()> {
        Ok(())
    }
}

/// The outline of `layout` as one geometry, with the layout's top-left
/// corner at (0, 0); `None` for a text without glyphs.
pub fn outline_geometry(
    factory: &ID2D1Factory,
    layout: &IDWriteTextLayout,
) -> Result<Option<ID2D1Geometry>> {
    let collector = ComObject::new(OutlineCollector {
        factory: factory.clone(),
        geometries: RefCell::new(Vec::new()),
    });
    let renderer: IDWriteTextRenderer = collector.to_interface();
    // SAFETY: `Draw` calls the renderer synchronously on this thread and
    // returns before we read what it collected; no drawing context.
    unsafe { layout.Draw(None, &renderer, 0.0, 0.0)? };
    let geometries = collector.geometries.take();
    if geometries.is_empty() {
        return Ok(None);
    }
    // SAFETY: the geometries come from `factory`, as the group requires.
    let group = unsafe { factory.CreateGeometryGroup(D2D1_FILL_MODE_WINDING, &geometries)? };
    Ok(Some(group.into()))
}

/// One text part of a block: its layout and size, and its outline geometry
/// when the style has an outline or a shadow. Rebuilt only when the text,
/// the style or the cell size change.
#[derive(Default)]
pub struct TextItem {
    text: String,
    style: Option<TextStyle>,
    cell: f64,
    layout: Option<IDWriteTextLayout>,
    geometry: Option<ID2D1Geometry>,
    pub size: TextBox,
}

impl TextItem {
    /// Whether the item already shows `text` in `style` for `cell`.
    pub fn is(&self, text: &str, style: &TextStyle, cell: f64) -> bool {
        self.text == text && self.cell == cell && self.style.as_ref() == Some(style)
    }

    /// Lays out `text` again if it, `style` or `cell` changed.
    pub fn update(
        &mut self,
        text: &str,
        style: &TextStyle,
        cell: f64,
        dwrite: &IDWriteFactory,
        format: impl FnOnce() -> Result<IDWriteTextFormat>,
        factory: &ID2D1Factory,
    ) -> Result<()> {
        if self.is(text, style, cell) {
            return Ok(());
        }
        // Forgotten first, and recorded only once built: a failed layout
        // is tried again at the next update.
        self.text.clear();
        self.style = None;
        self.layout = None;
        self.geometry = None;
        self.size = TextBox::default();
        if !text.is_empty() {
            let utf16: Vec<u16> = text.encode_utf16().collect();
            let format = format()?;
            // SAFETY: `utf16` and the format outlive the call; the layout is
            // returned owned; the getters write into locals.
            let (layout, metrics, line) = unsafe {
                let layout = dwrite.CreateTextLayout(&utf16, &format, MAX_EXTENT, MAX_EXTENT)?;
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut metrics)?;
                let first = first_line(&layout)?;
                (layout, metrics, first)
            };
            if style.outline.is_some() || style.shadow.is_some() {
                self.geometry = outline_geometry(factory, &layout)?;
            }
            self.size = TextBox {
                w: metrics.widthIncludingTrailingWhitespace,
                ascent: line.baseline,
                descent: (metrics.height - line.baseline).max(0.0),
            };
            self.layout = Some(layout);
        }
        self.text.push_str(text);
        self.style = Some(style.clone());
        self.cell = cell;
        Ok(())
    }

    /// Draws the text with its top-left corner at `at`, filled with `fill`
    /// (the style's colour or a threshold's); `brush` gives a brush per colour.
    pub fn draw(
        &self,
        rt: &ID2D1RenderTarget,
        at: (f32, f32),
        fill: Rgba,
        brush: &mut dyn FnMut(
            Rgba,
        )
            -> Result<windows::Win32::Graphics::Direct2D::ID2D1SolidColorBrush>,
    ) -> Result<()> {
        let (Some(layout), Some(style)) = (&self.layout, &self.style) else {
            return Ok(());
        };
        let fill_brush = brush(fill)?;
        let Some(geometry) = &self.geometry else {
            // SAFETY: drawing on the target between its BeginDraw and
            // EndDraw, from the thread that owns it.
            unsafe {
                rt.DrawTextLayout(
                    Vector2::new(at.0, at.1),
                    layout,
                    &fill_brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                )
            };
            return Ok(());
        };
        let outline = style
            .outline
            .map(|o| (style_px(f64::from(o.width), self.cell) * 2.0, o.color));
        // Every brush first: nothing below can fail with a transform set.
        let shadow = match style.shadow {
            Some(sh) => Some((
                style_px(f64::from(sh.dx), self.cell),
                style_px(f64::from(sh.dy), self.cell),
                brush(sh.color)?,
            )),
            None => None,
        };
        let outline = match outline {
            Some((width, color)) => Some((width, brush(color)?)),
            None => None,
        };
        // SAFETY: as above; the geometry comes from the target's factory
        // (the cache drops it with the device), and the transform is reset
        // to identity before the block ends.
        unsafe {
            if let Some((dx, dy, b)) = &shadow {
                rt.SetTransform(&Matrix3x2::translation(at.0 + dx, at.1 + dy));
                if let Some((width, _)) = &outline {
                    rt.DrawGeometry(geometry, b, *width, None);
                }
                rt.FillGeometry(geometry, b, None);
            }
            rt.SetTransform(&Matrix3x2::translation(at.0, at.1));
            if let Some((width, b)) = &outline {
                rt.DrawGeometry(geometry, b, *width, None);
            }
            rt.FillGeometry(geometry, &fill_brush, None);
            rt.SetTransform(&Matrix3x2::identity());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateFactory, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    };
    use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, DWRITE_FACTORY_TYPE_SHARED};

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn outline_renderer_builds_a_geometry() {
        // SAFETY: plain factory creation; both are returned owned.
        let (d2d, dwrite): (ID2D1Factory1, IDWriteFactory) = unsafe {
            (
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).expect("d2d"),
                DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).expect("dwrite"),
            )
        };
        let factory: ID2D1Factory = d2d.into();
        let style = TextStyle::default();
        let format = create_format(&dwrite, &style, 16.0).expect("format");
        let text: Vec<u16> = "144 FPS".encode_utf16().collect();
        // SAFETY: owned locals outlive the calls.
        let layout =
            unsafe { dwrite.CreateTextLayout(&text, &format, 1000.0, 100.0) }.expect("layout");
        let geometry = outline_geometry(&factory, &layout)
            .expect("outline")
            .expect("glyphs");
        // SAFETY: a getter on a live geometry; no transform.
        let bounds = unsafe { geometry.GetBounds(None) }.expect("bounds");
        // Glyphs sit below the layout's top and to the right of its left.
        assert!(bounds.right - bounds.left > 20.0, "{bounds:?}");
        assert!(bounds.bottom - bounds.top > 5.0, "{bounds:?}");
        assert!(bounds.top >= 0.0 && bounds.left >= 0.0, "{bounds:?}");

        // The item lays out once and keeps its geometry until the text changes.
        let outlined = TextStyle {
            outline: Some(oma_core::overlay::Outline {
                width: 1.0,
                color: Rgba::BLACK,
            }),
            ..TextStyle::default()
        };
        let mut item = TextItem::default();
        let fmt = || create_format(&dwrite, &outlined, 16.0);
        item.update("60", &outlined, 8.0, &dwrite, fmt, &factory)
            .expect("update");
        assert!(item.geometry.is_some() && item.size.w > 0.0 && item.size.ascent > 0.0);
        assert!(item.is("60", &outlined, 8.0));
        assert!(!item.is("61", &outlined, 8.0));
        // Two lines: the metrics of the first line still give the baseline.
        let fmt = || create_format(&dwrite, &outlined, 16.0);
        item.update(
            "1
2", &outlined, 8.0, &dwrite, fmt, &factory,
        )
        .expect("update");
        assert!(item.size.ascent > 0.0, "{:?}", item.size);
        assert!(item.size.descent > item.size.ascent, "{:?}", item.size);
        // An empty text has neither layout nor geometry.
        let fmt = || create_format(&dwrite, &outlined, 16.0);
        item.update("", &outlined, 8.0, &dwrite, fmt, &factory)
            .expect("update");
        assert!(item.layout.is_none() && item.geometry.is_none());
        assert_eq!(item.size, TextBox::default());
    }
}
