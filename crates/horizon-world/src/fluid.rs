//! Fluid states carried by the pinned vanilla block-state registry.
//! Flow updates, spreading and entity interaction are separate simulation work.
use crate::{
    chunk::BlockState,
    compression::invalid,
    registry::{BlockRegistry, BlockStateId},
};
use std::io;
pub mod custom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidKind {
    Empty,
    FlowingWater,
    Water,
    FlowingLava,
    Lava,
    Custom(u8),
    FlowingCustom(u8),
}
impl FluidKind {
    pub const ALL: [Self; 5] = [
        Self::Empty,
        Self::FlowingWater,
        Self::Water,
        Self::FlowingLava,
        Self::Lava,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Empty => "minecraft:empty",
            Self::FlowingWater => "minecraft:flowing_water",
            Self::Water => "minecraft:water",
            Self::FlowingLava => "minecraft:flowing_lava",
            Self::Lava => "minecraft:lava",
            Self::Custom(_) => {
                custom::definition(self).map_or("minecraft:empty", |fluid| &fluid.id)
            }
            Self::FlowingCustom(_) => {
                custom::definition(self).map_or("minecraft:empty", |fluid| &fluid.flowing_id)
            }
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.name() == name)
            .or_else(|| custom::named(name))
    }
    pub const fn same_family(self, other: Self) -> bool {
        if let (
            Self::Custom(left) | Self::FlowingCustom(left),
            Self::Custom(right) | Self::FlowingCustom(right),
        ) = (self, other)
        {
            return left == right;
        }
        matches!(
            (self, other),
            (Self::Empty, Self::Empty)
                | (
                    Self::Water | Self::FlowingWater,
                    Self::Water | Self::FlowingWater
                )
                | (
                    Self::Lava | Self::FlowingLava,
                    Self::Lava | Self::FlowingLava
                )
        )
    }
    pub const fn mask(self) -> u8 {
        match self {
            Self::Custom(_) | Self::FlowingCustom(_) => 0,
            _ => 1 << self.wire_id(),
        }
    }
    pub const fn wire_id(self) -> u32 {
        match self {
            Self::Empty => 0,
            Self::FlowingWater => 1,
            Self::Water => 2,
            Self::FlowingLava => 3,
            Self::Lava => 4,
            Self::Custom(index) => 5 + index as u32 * 2,
            Self::FlowingCustom(index) => 6 + index as u32 * 2,
        }
    }
    pub fn from_wire_id(id: u32) -> Option<Self> {
        if id < 5 {
            return Self::ALL.get(id as usize).copied();
        }
        let index = u8::try_from((id - 5) / 2).ok()?;
        custom::definitions().get(usize::from(index))?;
        Some(if (id - 5) % 2 == 0 {
            Self::Custom(index)
        } else {
            Self::FlowingCustom(index)
        })
    }
    pub const fn source(self) -> Self {
        match self {
            Self::FlowingWater => Self::Water,
            Self::FlowingLava => Self::Lava,
            Self::FlowingCustom(index) => Self::Custom(index),
            source => source,
        }
    }
    pub const fn flowing(self) -> Self {
        match self {
            Self::Water => Self::FlowingWater,
            Self::Lava => Self::FlowingLava,
            Self::Custom(index) => Self::FlowingCustom(index),
            flowing => flowing,
        }
    }
    pub fn motion_kind(self) -> Self {
        match custom::definition(self).map(|fluid| fluid.motion) {
            Some(custom::FluidMotion::Water) => Self::Water,
            Some(custom::FluidMotion::Lava) => Self::Lava,
            Some(custom::FluidMotion::None) => Self::Empty,
            None => self,
        }
    }
    pub fn can_drown(self) -> bool {
        custom::definition(self).map_or(self.same_family(Self::Water), |fluid| fluid.can_drown)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FluidState {
    kind: FluidKind,
    amount: u8,
    falling: bool,
}
pub fn spread_state(kind: FluidKind, highest: u8, above: bool, drop_off: u8) -> FluidState {
    if above {
        return FluidState::new(kind.flowing(), 8, true);
    }
    let amount = highest.saturating_sub(drop_off);
    if amount == 0 {
        FluidState::EMPTY
    } else {
        FluidState::new(kind.flowing(), amount, false)
    }
}
impl FluidState {
    /// Native fluid states require an object; properties decode leniently.
    pub fn from_json(json: &str) -> io::Result<Self> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|e| invalid(e.to_string()))?;
        let name = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("missing fluid identifier"))?;
        let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", name));
        let name = format!(
            "{}:{path}",
            if namespace.is_empty() {
                "minecraft"
            } else {
                namespace
            }
        );
        let kind =
            FluidKind::from_name(&name).ok_or_else(|| invalid("unknown fluid identifier"))?;
        if kind == FluidKind::Empty {
            return Ok(Self::EMPTY);
        }
        let flowing = kind != kind.source();
        let mut state = Self {
            kind,
            amount: if flowing { 1 } else { 8 },
            falling: true,
        };
        if let Some(properties) = value
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            if let Some(value) = properties
                .get("falling")
                .and_then(serde_json::Value::as_str)
            {
                match value {
                    "true" => state.falling = true,
                    "false" => state.falling = false,
                    _ => {}
                }
            }
            if flowing {
                if let Some(value) = properties
                    .get("level")
                    .and_then(serde_json::Value::as_str)
                    .and_then(crate::registry::java_integer)
                    .filter(|v| (1..=8).contains(v))
                {
                    state.amount = value as u8;
                }
            }
        }
        Ok(state)
    }
    pub fn to_json(self) -> io::Result<String> {
        let mut value = serde_json::json!({"id": self.kind.name()});
        if self.kind != FluidKind::Empty {
            let mut properties = serde_json::json!({"falling": self.falling.to_string()});
            if !self.is_source() {
                properties["level"] = self.amount.to_string().into();
            }
            value["properties"] = properties;
        }
        serde_json::to_string(&value).map_err(|e| invalid(e.to_string()))
    }
    /// A fluid state from its parts; the empty fluid ignores amount and falling.
    pub const fn new(kind: FluidKind, amount: u8, falling: bool) -> Self {
        if matches!(kind, FluidKind::Empty) {
            return Self::EMPTY;
        }
        Self {
            kind,
            amount,
            falling,
        }
    }
    pub const EMPTY: Self = Self {
        kind: FluidKind::Empty,
        amount: 0,
        falling: false,
    };
    pub const WATER: Self = Self {
        kind: FluidKind::Water,
        amount: 8,
        falling: false,
    };
    pub const fn kind(self) -> FluidKind {
        self.kind
    }
    pub const fn amount(self) -> u8 {
        self.amount
    }
    pub const fn falling(self) -> bool {
        self.falling
    }
    pub const fn is_source(self) -> bool {
        matches!(
            self.kind,
            FluidKind::Water | FluidKind::Lava | FluidKind::Custom(_)
        )
    }
    pub const fn is_full(self) -> bool {
        self.amount == 8
    }
    pub fn height_at<W: FluidHeightWorld>(
        self,
        world: &mut W,
        position: [i32; 3],
    ) -> Result<f32, W::Error> {
        if self.kind == FluidKind::Empty {
            return Ok(0.0);
        }
        let above = [position[0], position[1].wrapping_add(1), position[2]];
        Ok(if self.kind.same_family(world.fluid(above)?.kind) {
            1.0
        } else {
            self.own_height()
        })
    }
    pub fn camera_height_at<W: FluidHeightWorld>(
        self,
        world: &mut W,
        position: [i32; 3],
    ) -> Result<f32, W::Error> {
        if self.is_source() {
            let above = [position[0], position[1].wrapping_add(1), position[2]];
            if world.sturdy_down(above)? {
                return Ok(1.0);
            }
        }
        self.height_at(world, position)
    }
    pub fn own_height(self) -> f32 {
        f32::from(self.amount) / 9.0
    }
    pub fn from_block(state: &BlockState) -> io::Result<Self> {
        if let Some(kind) = custom::block(&state.name) {
            let level = state
                .properties
                .get("level")
                .and_then(|value| value.parse::<u8>().ok())
                .filter(|level| *level < 16)
                .ok_or_else(|| invalid("custom fluid needs level 0..15"))?;
            return Ok(Self::new(
                if level == 0 {
                    kind.source()
                } else {
                    kind.flowing()
                },
                if level == 0 || level >= 8 {
                    8
                } else {
                    8 - level
                },
                level >= 8,
            ));
        }
        let water = state.name == "minecraft:water";
        if water || state.name == "minecraft:lava" {
            let level = state
                .properties
                .get("level")
                .and_then(|v| v.parse::<u8>().ok())
                .filter(|v| *v <= 15)
                .ok_or_else(|| invalid("invalid liquid block level"))?;
            let kind = match (water, level == 0) {
                (true, true) => FluidKind::Water,
                (true, false) => FluidKind::FlowingWater,
                (false, true) => FluidKind::Lava,
                (false, false) => FluidKind::FlowingLava,
            };
            return Ok(Self {
                kind,
                amount: if level == 0 || level >= 8 {
                    8
                } else {
                    8 - level
                },
                falling: level >= 8,
            });
        }
        if matches!(
            state.name.as_str(),
            "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass"
                | "minecraft:bubble_column"
        ) {
            return Ok(Self::WATER);
        }
        match state.properties.get("waterlogged").map(String::as_str) {
            Some("true") => {
                // The pinned copper-grate blocks use falling source water.
                let falling = matches!(
                    state.name.as_str(),
                    "minecraft:copper_grate"
                        | "minecraft:exposed_copper_grate"
                        | "minecraft:weathered_copper_grate"
                        | "minecraft:oxidized_copper_grate"
                        | "minecraft:waxed_copper_grate"
                        | "minecraft:waxed_exposed_copper_grate"
                        | "minecraft:waxed_weathered_copper_grate"
                        | "minecraft:waxed_oxidized_copper_grate"
                );
                Ok(Self {
                    falling,
                    ..Self::WATER
                })
            }
            None | Some("false") => Ok(Self::EMPTY),
            _ => Err(invalid("invalid waterlogged block property")),
        }
    }
}
/// Queries required by fluid surface height and camera immersion. The caller
/// owns block shape/support behavior and world-query failures.
pub trait FluidHeightWorld {
    type Error;
    fn fluid(&mut self, position: [i32; 3]) -> Result<FluidState, Self::Error>;
    fn sturdy_down(&mut self, position: [i32; 3]) -> Result<bool, Self::Error>;
}
#[derive(Debug)]
pub struct FluidStateTable(Vec<FluidState>);
impl FluidStateTable {
    pub fn new(registry: &BlockRegistry) -> io::Result<Self> {
        (0..registry.len())
            .map(|i| FluidState::from_block(registry.state(BlockStateId(i as u32)).unwrap()))
            .collect::<io::Result<Vec<_>>>()
            .map(Self)
    }
    pub fn get(&self, state: BlockStateId) -> Option<FluidState> {
        self.0.get(state.0 as usize).copied()
    }
}

/// Bound identities for converting fluid states into their legacy liquid blocks.
pub struct FluidBlockBindings {
    air: BlockStateId,
    water: [BlockStateId; 16],
    lava: [BlockStateId; 16],
    custom: Vec<([BlockStateId; 16], [BlockStateId; 16])>,
}
impl FluidBlockBindings {
    pub fn new(registry: &BlockRegistry) -> io::Result<Self> {
        Self::with_custom(registry, custom::definitions())
    }
    fn with_custom(
        registry: &BlockRegistry,
        custom: &[custom::FluidDefinition],
    ) -> io::Result<Self> {
        let levels = |name: &str| -> io::Result<[BlockStateId; 16]> {
            let default = registry
                .default_state(name)
                .ok_or_else(|| invalid("missing liquid block"))?;
            let mut state = registry.state(default).unwrap().clone();
            let mut result = [default; 16];
            for (level, id) in result.iter_mut().enumerate() {
                state.properties.insert("level".into(), level.to_string());
                *id = registry
                    .id(&state)
                    .ok_or_else(|| invalid("missing liquid block level"))?;
            }
            Ok(result)
        };
        Ok(Self {
            air: registry
                .default_state("minecraft:air")
                .ok_or_else(|| invalid("missing air block"))?,
            water: levels("minecraft:water")?,
            lava: levels("minecraft:lava")?,
            custom: custom
                .iter()
                .map(|fluid| Ok((levels(&fluid.still_block)?, levels(&fluid.flowing_block)?)))
                .collect::<io::Result<_>>()?,
        })
    }
    pub fn block(&self, state: FluidState) -> BlockStateId {
        if state.kind == FluidKind::Empty {
            return self.air;
        }
        let level = if state.is_source() {
            0
        } else {
            8 - state.amount.min(8) + if state.falling { 8 } else { 0 }
        } as usize;
        if let FluidKind::Custom(index) | FluidKind::FlowingCustom(index) = state.kind {
            let Some((source, flowing)) = self.custom.get(usize::from(index)) else {
                return self.air;
            };
            return if state.is_source() {
                source[0]
            } else {
                flowing[level]
            };
        }
        if state.kind.same_family(FluidKind::Water) {
            self.water[level]
        } else {
            self.lava[level]
        }
    }
}

#[cfg(test)]
mod custom_tests {
    use super::*;
    use crate::fluid_flow::{FluidFlowWorld, flow_at};
    use horizon_core::{geometry::Direction, position::BlockPos};
    use std::{collections::HashMap, convert::Infallible};

    #[derive(Default)]
    struct FlowWorld {
        fluids: HashMap<BlockPos, FluidState>,
        reads: usize,
    }

    impl FluidFlowWorld for FlowWorld {
        type Error = Infallible;
        type Block = ();
        fn fluid(&mut self, position: BlockPos) -> Result<FluidState, Infallible> {
            self.reads += 1;
            Ok(self
                .fluids
                .get(&position)
                .copied()
                .unwrap_or(FluidState::EMPTY))
        }
        fn block(&mut self, _position: BlockPos) -> Result<(), Infallible> {
            Ok(())
        }
        fn blocks_fluid_flow(&self, _state: ()) -> bool {
            true
        }
        fn is_ice_class(&self, _state: ()) -> bool {
            false
        }
        fn is_face_sturdy(
            &mut self,
            _state: (),
            _position: BlockPos,
            _direction: Direction,
        ) -> Result<bool, Infallible> {
            Ok(false)
        }
    }

    #[test]
    fn no_fluid_does_no_neighbor_work() {
        let mut world = FlowWorld::default();
        assert_eq!(
            flow_at(&mut world, FluidState::EMPTY, BlockPos::new(0, 0, 0)).unwrap(),
            horizon_core::geometry::Vec3::ZERO
        );
        assert_eq!(world.reads, 0);
        assert_eq!(FluidKind::Water.wire_id(), 2);
        assert_eq!(FluidKind::Lava.wire_id(), 4);
        assert_eq!(spread_state(FluidKind::Water, 8, false, 1).amount(), 7);
    }

    #[test]
    fn custom_placement_spread_and_current_share_fluid_identity() {
        let source = FluidState::new(FluidKind::Custom(0), 8, false);
        let bindings = FluidBlockBindings {
            air: BlockStateId(0),
            water: [BlockStateId(1); 16],
            lava: [BlockStateId(2); 16],
            custom: vec![([BlockStateId(3); 16], [BlockStateId(4); 16])],
        };
        let side = spread_state(source.kind(), 8, false, 2);
        let below = spread_state(source.kind(), 0, true, 2);
        assert_eq!(bindings.block(source), BlockStateId(3));
        assert_eq!(bindings.block(side), BlockStateId(4));
        assert_eq!(side.amount(), 6);
        assert!(below.falling());
        assert!(source.kind().same_family(side.kind()));
        let position = BlockPos::new(0, 0, 0);
        let mut world = FlowWorld::default();
        world.fluids.insert(position.offset(1, 0, 0), side);
        let current = flow_at(&mut world, source, position).unwrap();
        assert!(current.x > 0.0);
        assert_eq!(current.z, 0.0);
    }

    #[test]
    fn custom_blocks_bind_every_source_and_flowing_level() {
        let fluid = custom::tests::oil();
        let mut states = vec![serde_json::json!([
            "air",
            true,
            false,
            null,
            "minecraft:air"
        ])];
        let mut defaults = serde_json::Map::new();
        defaults.insert("minecraft:air".into(), 0.into());
        for name in [
            "minecraft:water",
            "minecraft:lava",
            &fluid.still_block,
            &fluid.flowing_block,
        ] {
            defaults.insert(name.into(), states.len().into());
            for level in 0..16 {
                states.push(serde_json::json!([
                    name,
                    false,
                    true,
                    null,
                    {"id": name, "properties": {"level": level.to_string()}}
                ]));
            }
        }
        let facts = serde_json::json!({"states": states, "defaults": defaults});
        let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes()).unwrap();
        let bindings = FluidBlockBindings::with_custom(&registry, std::slice::from_ref(&fluid))
            .expect("every custom fluid level is present");
        let placed = bindings.block(FluidState::new(FluidKind::Custom(0), 8, false));
        let source = registry.state(placed).unwrap();
        assert_eq!(source.name, fluid.still_block);
        assert_eq!(
            source.properties.get("level").map(String::as_str),
            Some("0")
        );
        for level in 0..16u8 {
            let fluid = if level == 0 {
                FluidState::new(FluidKind::Custom(0), 8, false)
            } else {
                FluidState::new(
                    FluidKind::FlowingCustom(0),
                    // a falling state's block level is 8 + (8 - amount)
                    if level >= 8 { 16 - level } else { 8 - level },
                    level >= 8,
                )
            };
            let state = registry.state(bindings.block(fluid)).unwrap();
            assert_eq!(
                state.name,
                if level == 0 {
                    "test:oil"
                } else {
                    "test:flowing_oil"
                }
            );
            let level = level.to_string();
            assert_eq!(
                state.properties.get("level").map(String::as_str),
                Some(level.as_str())
            );
        }
    }
}
