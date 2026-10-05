//! The four built-in overlay profiles (spec §6.6), bound to the sensors of
//! the current machine by role.

use crate::model::Schema;
use crate::roles::{role_sensor, Role};

use super::profile::{
    Anchor, Block, CellRect, FrameMetric, GraphMode, GraphStyle, Kind, Outline, Profile, Rgba,
    Source, Style, TextStyle, PROFILE_FORMAT,
};

/// Width in cells of the Gaming and Full columns; the frametime graph spans it.
const WIDTH: u32 = 20;
const ROW_H: u32 = 2;
const GRAPH_H: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinId {
    MinimalFps,
    Gaming,
    Full,
    Bar,
}

impl BuiltinId {
    pub const ALL: [Self; 4] = [Self::MinimalFps, Self::Gaming, Self::Full, Self::Bar];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::MinimalFps => "builtin-minimal-fps",
            Self::Gaming => "builtin-gaming",
            Self::Full => "builtin-full",
            Self::Bar => "builtin-bar",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|id| id.as_str() == s)
    }

    fn name(self) -> &'static str {
        match self {
            Self::MinimalFps => "Minimal FPS",
            Self::Gaming => "Gaming",
            Self::Full => "Full",
            Self::Bar => "Bar",
        }
    }
}

/// One block before layout: its size in cells and what it shows.
struct Item {
    id: &'static str,
    source: Source,
    kind: Kind,
    w: u32,
    h: u32,
}

fn metric(id: &'static str, m: FrameMetric) -> Item {
    Item {
        id,
        source: Source::Frames(m),
        kind: Kind::Text,
        w: WIDTH / 2,
        h: ROW_H,
    }
}

/// A sensor block, or `None` when the machine has no sensor for the role.
fn sensor(schema: &Schema, id: &'static str, role: Role) -> Option<Item> {
    Some(Item {
        id,
        source: Source::Sensor(role_sensor(schema, role)?.to_owned()),
        kind: Kind::Text,
        w: WIDTH / 2,
        h: ROW_H,
    })
}

fn frametime_graph() -> Item {
    Item {
        id: "frametime-graph",
        source: Source::Frames(FrameMetric::FrametimeDisplayed),
        kind: Kind::Graph,
        w: WIDTH,
        h: GRAPH_H,
    }
}

fn text_style() -> TextStyle {
    TextStyle {
        outline: Some(Outline {
            width: 1.0,
            color: Rgba {
                r: 0,
                g: 0,
                b: 0,
                a: 0xC0,
            },
        }),
        ..TextStyle::default()
    }
}

fn block(item: &Item, x: i32, y: i32) -> Block {
    let mut style = Style {
        label_style: text_style(),
        value_style: text_style(),
        unit_style: text_style(),
        ..Style::default()
    };
    if item.kind == Kind::Graph {
        style.graph = GraphStyle {
            mode: GraphMode::Frametime,
            range_s: 30,
            ..GraphStyle::default()
        };
    }
    Block {
        id: item.id.to_owned(),
        rect: CellRect {
            x,
            y,
            w: item.w,
            h: item.h,
        },
        z: 0,
        source: item.source.clone(),
        stat: Default::default(),
        kind: item.kind,
        style,
        thresholds: vec![],
        visible_if: None,
        panel: None,
    }
}

/// Flows the items left to right and wraps at `max_w` cells, so omitted
/// blocks leave no holes.
fn flow(items: &[Item], max_w: u32) -> Vec<Block> {
    let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);
    let mut blocks = Vec::new();
    for item in items {
        if x > 0 && x + item.w > max_w {
            y += row_h;
            x = 0;
            row_h = 0;
        }
        blocks.push(block(item, x as i32, y as i32));
        x += item.w;
        row_h = row_h.max(item.h);
    }
    blocks
}

fn gaming_items(schema: &Schema) -> Vec<Item> {
    [
        Some(metric("fps-displayed", FrameMetric::FpsDisplayed)),
        Some(metric("fps-rendered", FrameMetric::FpsRendered)),
        Some(frametime_graph()),
        Some(metric("low-1", FrameMetric::Low1)),
        sensor(schema, "gpu-load", Role::GpuLoad),
        sensor(schema, "gpu-temperature", Role::GpuTemperature),
        sensor(schema, "cpu-load", Role::CpuLoad),
        sensor(schema, "cpu-temperature", Role::CpuTemperature),
        sensor(schema, "gpu-memory-used", Role::GpuMemoryUsed),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn full_items(schema: &Schema) -> Vec<Item> {
    let mut items = gaming_items(schema);
    items.extend(
        [
            sensor(schema, "ram-used", Role::RamUsed),
            sensor(schema, "gpu-clock", Role::GpuClock),
            sensor(schema, "cpu-clock", Role::CpuClock),
            sensor(schema, "gpu-power", Role::GpuPower),
            Some(metric("latency-pc", FrameMetric::LatencyPc)),
            Some(metric("bound", FrameMetric::Bound)),
            Some(metric("fg-multiplier", FrameMetric::FgMultiplier)),
        ]
        .into_iter()
        .flatten(),
    );
    items
}

fn bar_items(schema: &Schema) -> Vec<Item> {
    let mut items: Vec<Item> = [
        Some(metric("fps-displayed", FrameMetric::FpsDisplayed)),
        Some(metric("frametime", FrameMetric::FrametimeDisplayed)),
        sensor(schema, "gpu-load", Role::GpuLoad),
        sensor(schema, "gpu-temperature", Role::GpuTemperature),
        sensor(schema, "cpu-load", Role::CpuLoad),
        sensor(schema, "cpu-temperature", Role::CpuTemperature),
    ]
    .into_iter()
    .flatten()
    .collect();
    for item in &mut items {
        item.w = 8;
    }
    items
}

/// Builds a built-in profile for this machine. Blocks whose role has no
/// sensor in `schema` are left out; labels stay `None` so the overlay uses
/// the sensor or metric labels.
pub fn builtin_profile(id: BuiltinId, schema: &Schema) -> Profile {
    let (anchor, blocks) = match id {
        BuiltinId::MinimalFps => (
            Anchor::TopLeft,
            flow(&[metric("fps-displayed", FrameMetric::FpsDisplayed)], WIDTH),
        ),
        BuiltinId::Gaming => (Anchor::TopLeft, flow(&gaming_items(schema), WIDTH)),
        BuiltinId::Full => (Anchor::TopLeft, flow(&full_items(schema), WIDTH)),
        // A single row: the wrap width is never reached.
        BuiltinId::Bar => (Anchor::Top, flow(&bar_items(schema), u32::MAX)),
    };
    Profile {
        format: PROFILE_FORMAT,
        name: id.name().to_owned(),
        anchor,
        offset: Default::default(),
        scale: 1.0,
        panel: Default::default(),
        blocks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Device, DeviceKind};
    use crate::overlay::footprint;
    use crate::overlay::profile::{FrameMetric, GraphMode, Kind, Source};
    use crate::roles::role_sensor;
    use std::collections::BTreeMap;

    fn this_machine() -> Schema {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/this-machine-schema.json"
        ))
        .unwrap()
    }

    fn sensor_of(p: &Profile, id: &str) -> Option<String> {
        match &p.blocks.iter().find(|b| b.id == id)?.source {
            Source::Sensor(s) => Some(s.clone()),
            _ => None,
        }
    }

    #[test]
    fn builtin_ids_round_trip() {
        for id in BuiltinId::ALL {
            assert_eq!(BuiltinId::parse(id.as_str()), Some(id));
        }
        assert_eq!(BuiltinId::Gaming.as_str(), "builtin-gaming");
        assert_eq!(BuiltinId::MinimalFps.as_str(), "builtin-minimal-fps");
        assert_eq!(BuiltinId::Full.as_str(), "builtin-full");
        assert_eq!(BuiltinId::Bar.as_str(), "builtin-bar");
        assert_eq!(BuiltinId::parse("builtin-nope"), None);
        assert_eq!(BuiltinId::parse("gaming"), None);
    }

    #[test]
    fn gaming_template_binds_this_machine_schema() {
        let schema = this_machine();
        let p = builtin_profile(BuiltinId::Gaming, &schema);
        assert_eq!(
            sensor_of(&p, "gpu-load").as_deref(),
            Some("gpu/pci-0000:01:00.0/load/core")
        );
        assert_eq!(
            sensor_of(&p, "cpu-temperature").as_deref(),
            role_sensor(&schema, Role::CpuTemperature)
        );
        assert_eq!(
            sensor_of(&p, "gpu-memory-used").as_deref(),
            Some("gpu/pci-0000:01:00.0/data/memory-dedicated-used")
        );
        let ids: Vec<&str> = p.blocks.iter().map(|b| b.id.as_str()).collect();
        for want in ["fps-displayed", "fps-rendered", "frametime-graph", "low-1"] {
            assert!(ids.contains(&want), "missing {want}");
        }
        let graph = p.blocks.iter().find(|b| b.id == "frametime-graph").unwrap();
        assert_eq!(graph.kind, Kind::Graph);
        assert_eq!(graph.style.graph.mode, GraphMode::Frametime);
        assert_eq!((graph.rect.w, graph.rect.h), (20, 4));
        assert_eq!(
            graph.source,
            Source::Frames(FrameMetric::FrametimeDisplayed)
        );
        let fps = p.blocks.iter().find(|b| b.id == "fps-displayed").unwrap();
        let o = fps.style.value_style.outline.unwrap();
        assert_eq!(o.width, 1.0);
        assert_eq!(o.color.to_string(), "#000000C0");
        assert_eq!(p.panel, Default::default());
    }

    #[test]
    fn templates_omit_blocks_without_a_role() {
        let schema = Schema {
            revision: 1,
            devices: vec![Device {
                id: "cpu/0".to_owned(),
                kind: DeviceKind::Cpu,
                name: "cpu".to_owned(),
                vendor: None,
                properties: BTreeMap::new(),
            }],
            sensors: vec![],
        };
        for id in [BuiltinId::Gaming, BuiltinId::Full] {
            let p = builtin_profile(id, &schema);
            assert!(p.blocks.iter().any(|b| b.id == "fps-displayed"));
            assert!(!p.blocks.iter().any(|b| b.id.starts_with("gpu-")));
            assert!(!p.blocks.iter().any(|b| b.id.starts_with("cpu-")));
            p.validate().unwrap();
        }
    }

    #[test]
    fn templates_are_valid_profiles() {
        for schema in [this_machine(), Schema::default()] {
            for id in BuiltinId::ALL {
                let p = builtin_profile(id, &schema);
                p.validate().unwrap();
                assert!(!p.blocks.is_empty());
                for (i, a) in p.blocks.iter().enumerate() {
                    for b in &p.blocks[i + 1..] {
                        let (r, s) = (a.rect, b.rect);
                        let apart = r.x + r.w as i32 <= s.x
                            || s.x + s.w as i32 <= r.x
                            || r.y + r.h as i32 <= s.y
                            || s.y + s.h as i32 <= r.y;
                        assert!(apart, "{:?}: {} overlaps {}", id, a.id, b.id);
                    }
                }
                let f = footprint(&p.blocks).unwrap();
                assert!(f.w <= 48 && f.h <= 32, "{id:?} footprint {f:?}");
            }
        }
    }

    #[test]
    fn bar_is_one_row_at_the_top() {
        let p = builtin_profile(BuiltinId::Bar, &this_machine());
        assert_eq!(p.anchor, crate::overlay::Anchor::Top);
        assert!(p.blocks.iter().all(|b| b.rect.y == 0 && b.rect.h == 2));
        assert_eq!(
            builtin_profile(BuiltinId::MinimalFps, &this_machine())
                .blocks
                .len(),
            1
        );
        assert!(builtin_profile(BuiltinId::Full, &this_machine())
            .blocks
            .iter()
            .any(|b| b.id == "fg-multiplier"));
    }
}
