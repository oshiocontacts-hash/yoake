use super::{Game, game_mode::mode_id_of, shield::OFFHAND_SLOT};
use horizon_client::{
    first_person::{Hand, SwingState},
    pose::Pose,
};
use horizon_protocol::item::Stack;
use horizon_world::identifier::Identifier;

/// A detached, read-only local-player view, shared by replay and avatar mods.
/// Equipment is in main-hand, off-hand, head, chest, legs, feet order and uses
/// this connection's registry ids and data component patches.
#[derive(Clone, Debug)]
pub struct LocalPlayerSnapshot {
    pub entity_id: i32,
    pub uuid: u128,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub head_yaw: f32,
    pub body_yaw: f32,
    pub on_ground: bool,
    pub pose: Pose,
    pub sprinting: bool,
    pub swimming: bool,
    pub fall_flying: bool,
    pub swing: Option<(Hand, u32)>,
    pub using_item: Option<Hand>,
    pub use_ticks: i32,
    pub equipment: [Option<Stack>; 6],
    pub vehicle: Option<i32>,
    pub skin_parts: u8,
    pub main_hand_right: bool,
    pub dimension: Identifier,
    pub game_mode: u8,
    pub in_water: bool,
    pub eye_in_water: bool,
    pub flying: bool,
    pub health: f32,
    pub max_health: Option<f32>,
    pub food_level: i32,
}

fn activity(
    swing: SwingState,
    using: Option<(usize, u32)>,
) -> (Option<(Hand, u32)>, Option<Hand>, i32) {
    (
        swing
            .current
            .filter(|_| swing.ticks == 1)
            .map(|description| (description.hand, swing.ticks.max(0) as u32)),
        using.map(|(slot, _)| {
            if slot == OFFHAND_SLOT {
                Hand::Off
            } else {
                Hand::Main
            }
        }),
        using.map_or(0, |(_, ticks)| ticks.min(i32::MAX as u32) as i32),
    )
}

impl Game<'_> {
    pub(super) fn local_player_snapshot(&self) -> Option<LocalPlayerSnapshot> {
        self.native_client_world_key()?;
        let player = self.player.as_ref()?;
        let uuid = self
            .remote
            .as_ref()
            .map(|remote| remote.profile())
            .or_else(|| {
                self.level.player_uuid.map(|words| {
                    words.iter().fold(0_u128, |uuid, &word| {
                        (uuid << 32) | u128::from(word as u32)
                    })
                })
            })?;
        let slots = [
            self.inventory.get(self.selected)?,
            &self.offhand,
            &self.armor[0],
            &self.armor[1],
            &self.armor[2],
            &self.armor[3],
        ];
        let equipment = if self.remote.is_some() {
            slots.map(|stack| self.to_network(stack))
        } else {
            let synced = super::lan::sync_data(&self.options.resources, &self.pack_files()).ok()?;
            slots.map(|stack| {
                horizon_client::lan::guest::stack_to_network(
                    |name| {
                        synced
                            .builtin
                            .get("minecraft:item")
                            .and_then(|ids| ids.id(name))
                            .unwrap_or(0)
                    },
                    |registry, name| synced.id(registry, name),
                    stack,
                )
            })
        };
        let (swing, using_item, use_ticks) = activity(self.swing, self.using);
        let attributes = &player.attributes;
        let max_health = attributes.key("minecraft:max_health").and_then(|key| {
            let value = if let Some(handle) = attributes.lookup(Some(&key)) {
                attributes.instance(&handle).ok()?.clone().value()
            } else {
                let catalog = attributes.catalog();
                let attribute = key.attribute();
                catalog
                    .attribute(attribute)
                    .definition
                    .sanitize(catalog.default_base("minecraft:player", attribute)?)
            };
            Some(value as f32)
        });
        Some(LocalPlayerSnapshot {
            entity_id: self.remote.as_ref().map_or(0, |remote| remote.entity_id),
            uuid,
            pos: [player.position.x, player.position.y, player.position.z],
            yaw: self.yaw,
            pitch: self.pitch,
            head_yaw: self
                .player_model
                .as_ref()
                .map_or(player.head_yaw, |model| model.head_yaw),
            body_yaw: self
                .player_model
                .as_ref()
                .map_or(player.yaw, |model| model.body_yaw),
            on_ground: player.ground.on_ground,
            pose: player.pose,
            sprinting: player.sprinting,
            swimming: player.swimming,
            fall_flying: player.fall_flying,
            swing,
            using_item,
            use_ticks,
            equipment,
            vehicle: self.riding.map(|vehicle| vehicle as i32),
            skin_parts: self.settings.file.model_customisation(),
            main_hand_right: !self.settings.main_hand_left,
            dimension: Identifier::parse(
                self.remote
                    .as_ref()
                    .map_or(self.dim.id(), |remote| remote.level.as_str()),
            )
            .ok()?,
            game_mode: self
                .remote
                .as_ref()
                .map_or(mode_id_of(self.mode), |remote| remote.game_mode) as u8,
            in_water: player.fluids.was_touching_water,
            eye_in_water: player.is_eye_in_fluid(horizon_client::motion::WATER),
            flying: player.flying,
            health: player.health.health(),
            max_health,
            food_level: player.health.food.food,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::first_person::{SwingAnimation, SwingDescription};

    #[test]
    fn activity_observes_the_completed_tick_without_advancing_it() {
        let mut swing = SwingState {
            current: Some(SwingDescription {
                hand: Hand::Off,
                animation: SwingAnimation::DEFAULT,
                duration_ticks: 6,
            }),
            ..SwingState::default()
        };
        swing.tick();
        let before = swing;
        assert_eq!(
            activity(swing, Some((OFFHAND_SLOT, 12))),
            (Some((Hand::Off, 1)), Some(Hand::Off), 12)
        );
        assert_eq!(swing, before);
        swing.tick();
        assert_eq!(activity(swing, None), (None, None, 0));
        assert_eq!(activity(SwingState::default(), None), (None, None, 0));
    }

    #[test]
    fn elapsed_use_ticks_cannot_wrap_negative() {
        assert_eq!(
            activity(SwingState::default(), Some((0, u32::MAX))).2,
            i32::MAX
        );
    }
}
