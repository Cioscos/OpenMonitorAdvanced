//! Drawing of the profile's blocks (C12, spec §5.2 and §6.2).
//!
//! Every presented frame is drawn whole (the swapchain's buffers do not
//! keep the previous frame), but only from the cache: the texts are laid
//! out again only when [`Due::text`], the chart geometries rebuilt only when
//! [`Due::charts`], and each only if its input changed. Order: the
//! profile's panel, then each block by `z` (then profile order) with its
//! own panel, its chart or shape, and its texts on top.

#[cfg(windows)]
pub mod cache;
#[cfg(windows)]
pub mod graph;
pub mod layout;
#[cfg(windows)]
pub mod shapes;
#[cfg(windows)]
pub mod text;

#[cfg(windows)]
pub use cache::RenderCache;

#[cfg(windows)]
use oma_core::overlay::{
    fg_active, is_visible, threshold_color, Align, Block, FrameMetric, GraphMode, Kind,
    Orientation, Rgba, Source, Stat, StatOp, ThresholdTarget,
};
#[cfg(windows)]
use windows::core::Result;
#[cfg(windows)]
use windows::Win32::Graphics::Direct2D::{
    ID2D1Factory, ID2D1RenderTarget, D2D1_ANTIALIAS_MODE_ALIASED,
    D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
#[cfg(windows)]
use windows_numerics::Matrix3x2;

#[cfg(windows)]
use crate::cadence::Due;
#[cfg(windows)]
use crate::state::{OverlayState, SourceKey};
#[cfg(windows)]
use cache::{BlockCache, BrushRes, TextRes};
#[cfg(windows)]
use layout::{
    areas, block_px, block_value, format_block_value, frametime_bars, gauge_sweep, graph_points,
    merge_bars, meter_fraction, panel_radius_px, profile_frame, row_positions, source_value,
    style_px, text_parts, value_range, Frame, RectF, TextBox,
};

/// The colour of a chart's grid lines.
#[cfg(windows)]
const GRID: Rgba = Rgba {
    r: 255,
    g: 255,
    b: 255,
    a: 0x30,
};

/// Draws the profile of `state` on `rt`, between its `BeginDraw` and
/// `EndDraw`; `due` says which cached parts may be refreshed.
#[cfg(windows)]
pub fn draw(
    rt: &ID2D1RenderTarget,
    cache: &mut RenderCache,
    state: &OverlayState,
    due: Due,
) -> Result<()> {
    cache.bind(rt)?;
    let (Some(profile), Some((_, dpi))) = (&state.profile, &state.placement) else {
        return Ok(());
    };
    let Some(frame) = profile_frame(profile, *dpi) else {
        return Ok(());
    };
    let n = profile.blocks.len();
    let rebuild = !cache.valid || cache.blocks.len() != n;
    if rebuild {
        cache.blocks.clear();
        cache.blocks.resize_with(n, BlockCache::default);
        cache.order = (0..n).collect();
        // Stable: equal `z` keeps the profile order.
        cache.order.sort_by_key(|&i| profile.blocks[i].z);
        cache.valid = true;
    }
    let mut parts = cache.split()?;
    let fg = state
        .metrics
        .as_ref()
        .filter(|m| m.state == "running")
        .is_some_and(|m| fg_active(m.fg_multiplier, m.fg_suspected));
    if due.text || rebuild {
        for (block, bc) in profile.blocks.iter().zip(parts.blocks.iter_mut()) {
            update_texts(bc, block, state, &frame, fg, &mut parts.text)?;
        }
    }
    if due.charts || rebuild {
        let factory = parts.text.factory;
        for (block, bc) in profile.blocks.iter().zip(parts.blocks.iter_mut()) {
            update_chart(bc, block, state, &frame, factory, parts.samples)?;
        }
    }

    // SAFETY: drawing on the target between BeginDraw and EndDraw, from its
    // thread. Grayscale: ClearType needs an opaque background.
    unsafe {
        rt.SetTransform(&Matrix3x2::identity());
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    }
    let brushes = &mut parts.brushes;
    let p = &profile.panel;
    let (w, h) = frame.size;
    let whole = RectF {
        x: 0.0,
        y: 0.0,
        w,
        h,
    };
    let panel_brush = brushes.get(shapes::with_opacity(p.color, p.opacity))?;
    let radius = panel_radius_px(p.radius, frame.cell, w, h);
    shapes::fill_panel(rt, whole, radius, &panel_brush);
    for &i in parts.order {
        let (block, bc) = (&profile.blocks[i], &parts.blocks[i]);
        if !bc.visible {
            continue;
        }
        if block.panel.is_some() || bc.panel_color.is_some() {
            let bp = block.panel.as_ref().unwrap_or(p);
            let color = shapes::with_opacity(bc.panel_color.unwrap_or(bp.color), bp.opacity);
            let radius = panel_radius_px(bp.radius, frame.cell, bc.rect.w, bc.rect.h);
            shapes::fill_panel(rt, bc.rect, radius, &brushes.get(color)?);
        }
        let clip = shapes::d2d_rect(bc.rect);
        // SAFETY: as above; every push is popped below, also on an error.
        unsafe { rt.PushAxisAlignedClip(&clip, D2D1_ANTIALIAS_MODE_ALIASED) };
        let drawn = paint_block(rt, brushes, block, bc, &frame);
        // SAFETY: pops the clip pushed above.
        unsafe { rt.PopAxisAlignedClip() };
        drawn?;
    }
    Ok(())
}

#[cfg(windows)]
fn orientation(block: &Block) -> Orientation {
    match block.kind {
        Kind::Gauge => block.style.gauge.orientation,
        _ => block.style.meter.orientation,
    }
}

#[cfg(windows)]
fn is_percent(block: &Block, state: &OverlayState) -> bool {
    match &block.source {
        Source::Sensor(id) => state.sensors.get(id).is_some_and(|s| s.unit == "percent"),
        _ => false,
    }
}

/// A `frametime` graph of a `frametime-*` source: one bar per frame.
#[cfg(windows)]
fn is_frametime_chart(block: &Block) -> bool {
    block.kind == Kind::Graph
        && block.style.graph.mode == GraphMode::Frametime
        && matches!(
            block.source,
            Source::Frames(FrameMetric::FrametimeDisplayed | FrameMetric::FrametimeApp)
        )
}

/// The frametime of a frame for `block`'s source.
#[cfg(windows)]
fn frame_ms(block: &Block, f: &oma_ipc::overlay::WireFrameTime) -> Option<f64> {
    match block.source {
        Source::Frames(FrameMetric::FrametimeApp) => f.app_ms,
        _ => f.displayed_ms,
    }
}

/// A graph's minimum, average and maximum over its range, as
/// `min / avg / max unit`; empty without data.
#[cfg(windows)]
fn stats_text(block: &Block, state: &OverlayState) -> String {
    let range = block.style.graph.range_s;
    let (min, avg, max) = if is_frametime_chart(block) {
        let newest = state.frame_times.back().map_or(0.0, |f| f.t_s);
        let from = newest - f64::from(range);
        let (mut lo, mut hi, mut sum, mut n) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0u32);
        for ms in state
            .frame_times
            .iter()
            .filter(|f| f.t_s >= from)
            .filter_map(|f| frame_ms(block, f))
        {
            lo = lo.min(ms);
            hi = hi.max(ms);
            sum += ms;
            n += 1;
        }
        if n == 0 {
            return String::new();
        }
        (Some(lo), Some(sum / f64::from(n)), Some(hi))
    } else {
        let stat = |op| {
            let s = Stat {
                op,
                window: range,
                ..block.stat
            };
            source_value(&block.source, &s, state)
        };
        (stat(StatOp::Min), stat(StatOp::Avg), stat(StatOp::Max))
    };
    if avg.is_none() {
        return String::new();
    }
    let (lo, _) = format_block_value(block, state, min);
    let (mid, _) = format_block_value(block, state, avg);
    let (hi, unit) = format_block_value(block, state, max);
    oma_core::format::join(&format!("{lo} / {mid} / {hi}"), &unit)
}

/// Refreshes visibility, value, threshold colours and texts of a block.
#[cfg(windows)]
fn update_texts(
    bc: &mut BlockCache,
    block: &Block,
    state: &OverlayState,
    frame: &Frame,
    fg: bool,
    text: &mut TextRes<'_>,
) -> Result<()> {
    let cell = frame.cell;
    let rect = block_px(block, frame.origin, cell);
    bc.rect = rect;
    bc.visible = is_visible(
        block.visible_if.as_ref(),
        &|source, stat| source_value(source, stat, state),
        fg,
    );
    if !bc.visible {
        return Ok(());
    }
    let value = block_value(block, state);
    let color = |target| threshold_color(&block.thresholds, target, value);
    bc.value_color = color(ThresholdTarget::Value);
    bc.graph_color = color(ThresholdTarget::Graph);
    bc.panel_color = color(ThresholdTarget::Panel);

    let st = &block.style;
    let mut parts = text_parts(block, state);
    if block.kind == Kind::Graph && !st.graph.show_value {
        parts.value.clear();
        parts.unit.clear();
    }
    let px = |size: f32| style_px(f64::from(size), cell);
    let styles = [&st.label_style, &st.value_style, &st.unit_style];
    let strings = [&parts.label, &parts.value, &parts.unit];
    for ((item, s), style) in bc.texts.iter_mut().zip(strings).zip(styles) {
        text.update(item, s, style, px(style.size), cell)?;
    }
    let sizes = [bc.texts[0].size, bc.texts[1].size, bc.texts[2].size];
    let gap = (cell / 2.0) as f32;
    let unit_gap = if parts.unit == "%" {
        0.0
    } else {
        px(st.value_style.size) * 0.25
    };
    let area = areas(block.kind, orientation(block), rect, cell as f32);
    bc.at = match block.kind {
        Kind::Gauge => {
            // Value and unit in the middle, the label at the bottom.
            let mid = [TextBox::default(), sizes[1], sizes[2]];
            let at = row_positions(area.shape, mid, Align::Center, 0.0, unit_gap);
            let l = sizes[0];
            let label = (
                area.shape.x + (area.shape.w - l.w) / 2.0,
                area.shape.bottom() - l.ascent - l.descent,
            );
            [label, at[1], at[2]]
        }
        Kind::Graph => {
            // Label and value on a band at the top of the chart.
            let band_h = sizes
                .iter()
                .map(|b| b.ascent + b.descent)
                .fold(0.0, f32::max);
            let band = RectF {
                x: rect.x + gap / 2.0,
                w: (rect.w - gap).max(0.0),
                h: band_h,
                ..rect
            };
            row_positions(band, sizes, Align::Right, gap, unit_gap)
        }
        _ => row_positions(area.text, sizes, st.align, gap, unit_gap),
    };

    let stats = if block.kind == Kind::Graph && st.graph.show_min_avg_max {
        stats_text(block, state)
    } else {
        String::new()
    };
    let label_px = px(st.label_style.size);
    text.update(&mut bc.stats, &stats, &st.label_style, label_px, cell)?;
    let s = bc.stats.size;
    bc.stats_at = (rect.x + gap / 2.0, rect.bottom() - s.ascent - s.descent);

    let percent = is_percent(block, state);
    match block.kind {
        Kind::Meter => {
            let (lo, hi) = value_range(st.meter.min, st.meter.max, percent, value);
            bc.fraction = meter_fraction(value.unwrap_or(f64::NAN), lo, hi);
        }
        Kind::Gauge => {
            let (lo, hi) = value_range(st.gauge.min, st.gauge.max, percent, value);
            let sweep = gauge_sweep(value.unwrap_or(f64::NAN), lo, hi);
            let (center, radius, _) = shapes::gauge_circle(area.shape);
            let start = shapes::GAUGE_START_DEG;
            if bc.track.is_none() {
                bc.track = shapes::arc(text.factory, center, radius, start, 270.0)?;
            }
            if bc.sweep_arc.is_none() || bc.sweep != sweep {
                bc.sweep = sweep;
                bc.sweep_arc = shapes::arc(text.factory, center, radius, start, sweep)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Rebuilds the chart geometry of a visible `graph` or `sparkline`.
#[cfg(windows)]
fn update_chart(
    bc: &mut BlockCache,
    block: &Block,
    state: &OverlayState,
    frame: &Frame,
    factory: &ID2D1Factory,
    samples: &mut Vec<(f64, f64)>,
) -> Result<()> {
    if !bc.visible || !matches!(block.kind, Kind::Graph | Kind::Sparkline) {
        return Ok(());
    }
    let shape = areas(block.kind, orientation(block), bc.rect, frame.cell as f32).shape;
    let g = &block.style.graph;
    let range = f64::from(g.range_s);
    samples.clear();
    bc.line = None;
    bc.fill = None;
    if is_frametime_chart(block) {
        samples.extend(
            state
                .frame_times
                .iter()
                .filter_map(|f| frame_ms(block, f).map(|ms| (f.t_s, ms))),
        );
        let bars = merge_bars(&frametime_bars(samples, shape, range));
        bc.fill = graph::bars(factory, &bars)?;
        return Ok(());
    }
    let Some(ring) = SourceKey::of(&block.source).and_then(|k| state.rings.get(&k)) else {
        return Ok(());
    };
    samples.extend(ring.samples());
    // Anchored to the newest sample (DP12).
    let now = samples.last().map_or(0.0, |s| s.0);
    let points = graph_points(samples, shape, range, &g.y, now);
    let mode = if block.kind == Kind::Sparkline {
        GraphMode::Line
    } else {
        g.mode
    };
    match mode {
        GraphMode::Area => {
            bc.fill = graph::area(factory, &points, shape.bottom())?;
            bc.line = graph::line(factory, &points)?;
        }
        GraphMode::Bars => {
            bc.fill = graph::bars(factory, &graph::value_bars(&points, shape))?;
        }
        GraphMode::Line | GraphMode::Frametime => bc.line = graph::line(factory, &points)?,
    }
    Ok(())
}

/// Draws a block's chart or shape and its texts from the cache.
#[cfg(windows)]
fn paint_block(
    rt: &ID2D1RenderTarget,
    brushes: &mut BrushRes<'_>,
    block: &Block,
    bc: &BlockCache,
    frame: &Frame,
) -> Result<()> {
    let g = &block.style.graph;
    let line_color = bc.graph_color.unwrap_or(g.line.color);
    let fill_color = shapes::with_opacity(bc.graph_color.unwrap_or(g.fill.color), g.fill.alpha);
    let shape = areas(block.kind, orientation(block), bc.rect, frame.cell as f32).shape;
    // SAFETY: drawing on the target between BeginDraw and EndDraw, from its
    // thread; the geometries come from its factory (dropped with it).
    unsafe {
        match block.kind {
            Kind::Graph | Kind::Sparkline => {
                if block.kind == Kind::Graph && g.grid_lines > 0 {
                    shapes::grid_lines(rt, shape, g.grid_lines, &brushes.get(GRID)?);
                }
                if let Some(fill) = &bc.fill {
                    let solid = is_frametime_chart(block) || g.mode == GraphMode::Bars;
                    let c = if solid { line_color } else { fill_color };
                    rt.FillGeometry(fill, &brushes.get(c)?, None);
                }
                if let Some(line) = &bc.line {
                    let width = style_px(f64::from(g.line.width), frame.cell);
                    rt.DrawGeometry(line, &brushes.get(line_color)?, width, None);
                }
            }
            Kind::Meter => {
                rt.FillRectangle(&shapes::d2d_rect(shape), &brushes.get(fill_color)?);
                let bar = shapes::meter_bar(shape, bc.fraction, orientation(block));
                rt.FillRectangle(&shapes::d2d_rect(bar), &brushes.get(line_color)?);
            }
            Kind::Gauge => {
                let (_, _, stroke) = shapes::gauge_circle(shape);
                if let Some(track) = &bc.track {
                    rt.DrawGeometry(track, &brushes.get(fill_color)?, stroke, None);
                }
                if let Some(arc) = &bc.sweep_arc {
                    rt.DrawGeometry(arc, &brushes.get(line_color)?, stroke, None);
                }
            }
            Kind::Text => {}
        }
    }
    let st = &block.style;
    let colors = [
        st.label_style.color,
        bc.value_color.unwrap_or(st.value_style.color),
        st.unit_style.color,
    ];
    let mut brush = |c| brushes.get(c);
    for ((item, &at), color) in bc.texts.iter().zip(&bc.at).zip(colors) {
        item.draw(rt, at, color, &mut brush)?;
    }
    bc.stats
        .draw(rt, bc.stats_at, st.label_style.color, &mut brush)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use oma_core::model::Schema;
    use oma_core::overlay::{builtin_profile, BuiltinId, Source};
    use oma_ipc::overlay::{
        FrameMetrics, FrameTimes, OverlayMessage, PxArea, SensorInfo, SetPlacement, SetProfile,
        Values, WireFrameTime, WireValue,
    };
    use windows::Win32::Graphics::Direct2D::Common::{
        D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
    };
    use windows::Win32::Graphics::Direct2D::{
        D2D1CreateFactory, ID2D1Factory1, D2D1_FACTORY_TYPE_SINGLE_THREADED,
        D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    };
    use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
    use windows::Win32::Graphics::Imaging::{
        CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
        WICBitmapCacheOnLoad,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED,
    };

    use crate::state::default_draw;

    fn gaming_state() -> OverlayState {
        let schema: Schema = serde_json::from_str(include_str!(
            "../../../oma-core/tests/fixtures/this-machine-schema.json"
        ))
        .expect("schema fixture");
        let profile = builtin_profile(BuiltinId::Gaming, &schema);
        let sensors: Vec<SensorInfo> = profile
            .blocks
            .iter()
            .filter_map(|b| match &b.source {
                Source::Sensor(id) => Some(SensorInfo {
                    id: id.clone(),
                    label: id.rsplit('/').next().unwrap_or(id).to_owned(),
                    unit: if id.contains("/temperature/") {
                        "celsius".into()
                    } else {
                        "percent".into()
                    },
                }),
                _ => None,
            })
            .collect();
        assert!(!sensors.is_empty(), "the fixture binds sensors");
        let values = sensors
            .iter()
            .map(|s| WireValue {
                id: s.id.clone(),
                value: Some(57.0),
                quality: "fresh".into(),
            })
            .collect();
        let strings = BTreeMap::from(
            [
                ("metric.fps-displayed", "FPS"),
                ("metric.fps-rendered", "Rendered"),
                ("metric.low-1", "1% low"),
                ("low.integral", "(int.)"),
                ("fgSuspected", "FG?"),
                ("sensorAbsent", "missing"),
            ]
            .map(|(k, v)| (k.to_owned(), v.to_owned())),
        );
        let mut state = OverlayState::default();
        state.apply(
            OverlayMessage::SetProfile(SetProfile {
                profile_id: BuiltinId::Gaming.as_str().into(),
                profile_json: serde_json::to_string(&profile).expect("json"),
                sensors,
                strings,
                draw: default_draw(),
            }),
            0.0,
        );
        assert!(state.profile.is_some(), "the built-in profile parses");
        state.apply(
            OverlayMessage::SetPlacement(SetPlacement {
                area: Some(PxArea {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                }),
                dpi: 96,
            }),
            0.0,
        );
        state.apply(
            OverlayMessage::Values(Values {
                at_ms: 1000,
                values,
            }),
            1.0,
        );
        state.apply(
            OverlayMessage::FrameMetrics(FrameMetrics {
                state: "running".into(),
                fps_displayed: Some(144.0),
                fps_rendered: Some(72.0),
                fps_presented: Some(144.0),
                rendered_source: Some("reflex".into()),
                fg_suspected: false,
                frametime_displayed_ms: Some(6.9),
                frametime_app_ms: Some(13.8),
                fg_multiplier: Some(2.0),
                stutter_count: Some(0),
                stutter_percent: Some(0.0),
                latency_pc_ms: None,
                latency_display_ms: None,
                bound: Some("gpu".into()),
                lows: vec![],
            }),
            1.0,
        );
        let frames = (0..300)
            .map(|i| WireFrameTime {
                t_s: 1.0 + f64::from(i) / 144.0,
                displayed_ms: Some(if i % 50 == 0 { 20.0 } else { 6.9 }),
                app_ms: Some(13.8),
            })
            .collect();
        state.apply(OverlayMessage::FrameTimes(FrameTimes { frames }), 3.0);
        state
    }

    /// A top-down 32-bit BMP of premultiplied BGRA pixels.
    fn bmp(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(54 + pixels.len());
        let le = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        out.extend_from_slice(b"BM");
        le(&mut out, 54 + pixels.len() as u32);
        le(&mut out, 0);
        le(&mut out, 54);
        le(&mut out, 40);
        le(&mut out, w);
        le(&mut out, (-(h as i32)) as u32);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        for _ in 0..6 {
            le(&mut out, 0);
        }
        out.extend_from_slice(pixels);
        out
    }

    #[test]
    #[ignore = "requires real Windows hardware"]
    fn renders_gaming_profile_to_a_bitmap() {
        const W: u32 = 400;
        const H: u32 = 300;
        let state = gaming_state();
        // SAFETY: COM for this test thread (WIC needs it); the factories,
        // the bitmap and the target are owned locals used from this thread.
        let (bitmap, rt) = unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .expect("COM");
            let wic: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)
                    .expect("WIC");
            let bitmap = wic
                .CreateBitmap(W, H, &GUID_WICPixelFormat32bppPBGRA, WICBitmapCacheOnLoad)
                .expect("bitmap");
            let d2d: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).expect("d2d");
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
                ..Default::default()
            };
            let rt = d2d
                .CreateWicBitmapRenderTarget(&bitmap, &props)
                .expect("WIC target");
            (bitmap, rt)
        };
        let mut cache = RenderCache::default();
        let due = Due {
            text: true,
            charts: true,
        };
        // Twice: the second frame replays the cache.
        for _ in 0..2 {
            // SAFETY: a frame on our target, from this thread.
            unsafe { rt.BeginDraw() };
            draw(&rt, &mut cache, &state, due).expect("draw");
            // SAFETY: closes the frame; no tags.
            unsafe { rt.EndDraw(None, None) }.expect("end draw");
        }
        let mut pixels = vec![0u8; (W * H * 4) as usize];
        // SAFETY: the whole bitmap into a buffer of exactly its size.
        unsafe { bitmap.CopyPixels(std::ptr::null(), W * 4, &mut pixels) }.expect("pixels");
        // `OMA_RENDER_DUMP=<file.bmp>` saves the frame, to look at it.
        if let Some(path) = std::env::var_os("OMA_RENDER_DUMP") {
            std::fs::write(path, bmp(W, H, &pixels)).expect("dump");
        }
        let alpha = |x: u32, y: u32| pixels[((y * W + x) * 4 + 3) as usize];
        let drawn = (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .filter(|&(x, y)| alpha(x, y) != 0)
            .count();
        assert!(drawn > 1000, "only {drawn} pixels drawn");
        assert_eq!(alpha(W - 1, H - 1), 0, "the far corner stays transparent");
        // Text is drawn over the panel: more than one alpha level inside it.
        let levels: std::collections::HashSet<u8> = (0..60)
            .flat_map(|y| (0..170).map(move |x| (x, y)))
            .map(|(x, y)| alpha(x, y))
            .collect();
        assert!(levels.len() > 2, "{levels:?}");
    }
}
