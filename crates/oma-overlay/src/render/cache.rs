//! What the renderer keeps between frames (§11): one brush per colour, one
//! text format per font, size, weight and slant, and per block its texts,
//! their layouts and outlines, and its chart geometries. A frame without
//! changes only replays these.
//!
//! Brushes and geometries belong to one render target and its factory:
//! binding another target (a new device after a loss) drops them all.

use std::collections::HashMap;

use oma_core::overlay::{Rgba, TextStyle};
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::Common::D2D1_COLOR_F;
use windows::Win32::Graphics::Direct2D::{
    ID2D1Factory, ID2D1Geometry, ID2D1RenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED,
};

use super::layout::RectF;
use super::text::{create_format, TextItem};

/// Most brushes and text formats kept; past this the map starts over, so a
/// profile with many colours or fonts cannot grow it without bound.
const MAX_ENTRIES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FormatKey {
    font: String,
    px_bits: u32,
    weight: u16,
    italic: bool,
}

/// Device-bound resources: the target they were made for and its brushes.
struct Device {
    target: ID2D1RenderTarget,
    factory: ID2D1Factory,
    brushes: HashMap<u32, ID2D1SolidColorBrush>,
}

/// The part of the cache texts need while a block is borrowed.
pub struct TextRes<'a> {
    pub dwrite: &'a IDWriteFactory,
    pub factory: &'a ID2D1Factory,
    formats: &'a mut HashMap<FormatKey, IDWriteTextFormat>,
}

impl TextRes<'_> {
    /// Lays out `text` in `item` with `style` at `px` pixels, if changed.
    pub fn update(
        &mut self,
        item: &mut TextItem,
        text: &str,
        style: &TextStyle,
        px: f32,
        cell: f64,
    ) -> Result<()> {
        if item.is(text, style, cell) {
            return Ok(());
        }
        let (dwrite, formats) = (self.dwrite, &mut *self.formats);
        let format = || {
            let key = FormatKey {
                font: style.font.clone(),
                px_bits: px.to_bits(),
                weight: style.weight,
                italic: style.italic,
            };
            if let Some(f) = formats.get(&key) {
                return Ok(f.clone());
            }
            if formats.len() >= MAX_ENTRIES {
                formats.clear();
            }
            let f = create_format(dwrite, style, px)?;
            formats.insert(key, f.clone());
            Ok(f)
        };
        item.update(text, style, cell, dwrite, format, self.factory)
    }
}

/// The drawing state of one block, refreshed at the text and chart rates.
#[derive(Default)]
pub struct BlockCache {
    pub visible: bool,
    pub rect: RectF,
    /// The colours of the first true threshold per target.
    pub value_color: Option<Rgba>,
    pub graph_color: Option<Rgba>,
    pub panel_color: Option<Rgba>,
    /// Label, value and unit, and where each is drawn.
    pub texts: [TextItem; 3],
    pub at: [(f32, f32); 3],
    /// A graph's min / avg / max line, and where it is drawn.
    pub stats: TextItem,
    pub stats_at: (f32, f32),
    /// The chart's line (stroked) and area or bars (filled).
    pub line: Option<ID2D1Geometry>,
    pub fill: Option<ID2D1Geometry>,
    /// The data generation the chart was built from; `None` to build it.
    pub chart_stamp: Option<u64>,
    /// The highest value seen: the automatic top of a meter or gauge.
    pub peak: f64,
    /// A meter's fullness, 0–1.
    pub fraction: f32,
    /// A gauge's arcs: the whole track and the value's sweep.
    pub track: Option<ID2D1Geometry>,
    pub sweep_arc: Option<ID2D1Geometry>,
    pub sweep: f32,
}

#[derive(Default)]
pub struct RenderCache {
    dwrite: Option<IDWriteFactory>,
    device: Option<Device>,
    formats: HashMap<FormatKey, IDWriteTextFormat>,
    pub blocks: Vec<BlockCache>,
    /// Block indices in drawing order (`z`, then profile order).
    pub order: Vec<usize>,
    /// False after a new profile, placement or settings: rebuild all.
    pub valid: bool,
    /// Reused for the samples of one chart.
    pub samples: Vec<(f64, f64)>,
}

fn color_key(c: Rgba) -> u32 {
    u32::from_be_bytes([c.r, c.g, c.b, c.a])
}

impl RenderCache {
    /// Everything per block is rebuilt at the next frame (new profile,
    /// placement or drawing settings); brushes and formats stay.
    pub fn invalidate(&mut self) {
        self.valid = false;
    }

    /// Lets go of every device-bound resource (lost device, or the device
    /// released while hidden), so nothing keeps the old device alive.
    pub fn release_device(&mut self) {
        self.device = None;
        self.blocks.clear();
        self.valid = false;
    }

    /// Makes `target` the one the cached resources belong to; a different
    /// target drops them.
    pub fn bind(&mut self, target: &ID2D1RenderTarget) -> Result<()> {
        if self.dwrite.is_none() {
            // SAFETY: plain factory creation; returned owned.
            self.dwrite = Some(unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? });
        }
        if self.device.as_ref().is_some_and(|d| &d.target == target) {
            return Ok(());
        }
        self.release_device();
        // SAFETY: a getter on a live target; the factory is returned owned.
        let factory = unsafe { target.GetFactory()? };
        self.device = Some(Device {
            target: target.clone(),
            factory,
            brushes: HashMap::new(),
        });
        Ok(())
    }

    /// The parts of the cache apart, so blocks can be updated and drawn
    /// while brushes and text resources are borrowed too.
    pub fn split(&mut self) -> Result<Parts<'_>> {
        let device = self.device.as_mut().ok_or_else(not_bound)?;
        let dwrite = self.dwrite.as_ref().ok_or_else(not_bound)?;
        Ok(Parts {
            brushes: BrushRes {
                brushes: &mut device.brushes,
                target: &device.target,
            },
            text: TextRes {
                dwrite,
                factory: &device.factory,
                formats: &mut self.formats,
            },
            blocks: &mut self.blocks,
            order: &self.order,
            samples: &mut self.samples,
        })
    }
}

/// The cache split for one frame.
pub struct Parts<'a> {
    pub brushes: BrushRes<'a>,
    pub text: TextRes<'a>,
    pub blocks: &'a mut Vec<BlockCache>,
    pub order: &'a [usize],
    pub samples: &'a mut Vec<(f64, f64)>,
}

/// The brushes while the blocks are borrowed.
pub struct BrushRes<'a> {
    brushes: &'a mut HashMap<u32, ID2D1SolidColorBrush>,
    target: &'a ID2D1RenderTarget,
}

impl BrushRes<'_> {
    pub fn get(&mut self, c: Rgba) -> Result<ID2D1SolidColorBrush> {
        get_brush(self.brushes, self.target, c)
    }
}

fn get_brush(
    brushes: &mut HashMap<u32, ID2D1SolidColorBrush>,
    target: &ID2D1RenderTarget,
    c: Rgba,
) -> Result<ID2D1SolidColorBrush> {
    let key = color_key(c);
    if let Some(b) = brushes.get(&key) {
        return Ok(b.clone());
    }
    if brushes.len() >= MAX_ENTRIES {
        brushes.clear();
    }
    let color = D2D1_COLOR_F {
        r: f32::from(c.r) / 255.0,
        g: f32::from(c.g) / 255.0,
        b: f32::from(c.b) / 255.0,
        a: f32::from(c.a) / 255.0,
    };
    // SAFETY: a live target of this thread; `color` outlives the call.
    let b = unsafe { target.CreateSolidColorBrush(&color, None)? };
    brushes.insert(key, b.clone());
    Ok(b)
}

fn not_bound() -> windows::core::Error {
    windows::core::Error::from(windows::Win32::Foundation::E_UNEXPECTED)
}
