//! Opt-in, server-owned navigation for ordinary ground walkers.

use super::{
    Mob, MobWorld, Species,
    control::{MoveKind, Operation},
    nav::NavKind,
};

/// Who supplies the supported mob's movement decisions during server AI ticks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeMobControlMode {
    /// Release ownership and resume the existing selectors and brain.
    Vanilla,
    /// Claim ownership and suspend selectors and brain without removing goals.
    External,
}

/// Bounds for a ground-path search; omitted limits derive from follow range.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMobPathOptions {
    /// Manhattan target tolerance, clamped to 0..=16 blocks.
    pub reach: i32,
    /// Search distance in blocks, clamped to 1..=128; must be finite.
    pub max_distance: Option<f32>,
    /// Visited-node budget, clamped to 1..=4096.
    pub max_visited: Option<u32>,
    /// Override floating permission, or retain the mob's current navigation flag.
    pub can_float: Option<bool>,
    /// Return and follow a partial path when the target cannot be reached.
    pub allow_partial: bool,
}

impl Default for NativeMobPathOptions {
    fn default() -> Self {
        Self {
            reach: 1,
            max_distance: None,
            max_visited: None,
            can_float: None,
            allow_partial: true,
        }
    }
}

/// A detached path snapshot with width-aware, floor-adjusted foot positions.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMobPath {
    pub target: [i32; 3],
    pub waypoints: Vec<[f64; 3]>,
    pub reaches_target: bool,
}

/// Terminal reasons persist until another navigation command or ownership release.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NativeMobNavigationState {
    #[default]
    Idle,
    Pending,
    Moving,
    Done,
    Stopped,
    Stuck,
    Unreachable,
}

/// Navigation status visible only to the mod that owns the mob's control.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeMobNavigation {
    pub state: NativeMobNavigationState,
    pub target: Option<[i32; 3]>,
    pub next_waypoint: usize,
    pub waypoint_count: usize,
    pub reaches_target: bool,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct PathOptions {
    pub reach: i32,
    pub max_distance: f32,
    pub max_visited: i32,
    pub can_float: bool,
    pub allow_partial: bool,
}

impl PathOptions {
    pub(super) fn new(mob: &Mob, options: &NativeMobPathOptions) -> Result<Self, String> {
        let follow_range = mob.attrs.follow_range as f32;
        let follow_range = if follow_range.is_finite() {
            follow_range.clamp(1.0, 128.0)
        } else {
            16.0
        };
        let max_distance = options.max_distance.unwrap_or(follow_range);
        if !max_distance.is_finite() {
            return Err("mob path distance must be finite".into());
        }
        Ok(Self {
            reach: options.reach.clamp(0, 16),
            max_distance: max_distance.clamp(1.0, 128.0),
            max_visited: options
                .max_visited
                .unwrap_or((follow_range * 16.0) as u32)
                .clamp(1, 4096) as i32,
            can_float: options.can_float.unwrap_or(mob.ai.nav.can_float),
            allow_partial: options.allow_partial,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct MoveRequest {
    pub target: [i32; 3],
    pub speed: f64,
    pub options: PathOptions,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Command {
    Move(MoveRequest),
    Stop,
}

#[derive(Clone, Debug)]
pub(crate) struct NativeMobControl {
    pub(super) owner: usize,
    pub(super) suspend_pending: bool,
    pub(super) command: Option<Command>,
    pub(super) look: Option<[f64; 3]>,
    pub(super) jump: bool,
    pub(super) route: Option<MoveRequest>,
    pub(super) navigation: NativeMobNavigation,
    pub(super) saved_can_float: bool,
    pub(super) saved_can_open_doors: bool,
}

impl Mob {
    pub(crate) fn native_control_supported(&self) -> bool {
        // CHECK: Keep this allowlist paired with ordinary ground navigation and move control.
        matches!(
            self.species(),
            Species::Zombie
                | Species::Husk
                | Species::ZombieVillager
                | Species::Villager
                | Species::WanderingTrader
        ) && self.ai.nav.kind == NavKind::Ground
            && self.ai.mov.kind == MoveKind::Ground
            && self.alive()
            && !self.removed
            && !self.no_ai
            && self.extra.mount.is_none()
            && !self.extra.passenger
            && self.extra.rider.is_none()
            && self.rider.is_none()
    }

    pub(crate) fn native_control_owned(&self, owner: usize) -> bool {
        self.native_control_supported()
            && self
                .native_control
                .as_ref()
                .is_some_and(|state| state.owner == owner)
    }

    pub fn native_control_set(&mut self, owner: usize, mode: NativeMobControlMode) -> bool {
        if !self.native_control_supported()
            || self
                .native_control
                .as_ref()
                .is_some_and(|state| state.owner != owner)
        {
            return false;
        }
        match mode {
            NativeMobControlMode::Vanilla => self.native_control_drop(),
            NativeMobControlMode::External => {
                if self.native_control.is_none() {
                    self.native_control = Some(Box::new(NativeMobControl {
                        owner,
                        suspend_pending: true,
                        command: None,
                        look: None,
                        jump: false,
                        route: None,
                        navigation: NativeMobNavigation::default(),
                        saved_can_float: self.ai.nav.can_float,
                        saved_can_open_doors: self.ai.nav.can_open_doors,
                    }));
                }
            }
        }
        true
    }

    pub(super) fn native_control_take_suspension(&mut self) -> bool {
        self.native_control
            .as_mut()
            .is_some_and(|state| std::mem::take(&mut state.suspend_pending))
    }

    pub fn native_control_drop(&mut self) {
        if let Some(state) = self.native_control.take() {
            self.native_navigation_reset();
            self.ai.nav.can_float = state.saved_can_float;
            self.ai.nav.can_open_doors = state.saved_can_open_doors;
            self.native_control_halt();
        }
    }

    pub(super) fn native_control_halt(&mut self) {
        self.ai.mov.operation = Operation::Wait;
        self.ai.xxa = 0.0;
        self.ai.zza = 0.0;
        self.ai.jump = false;
        self.ai.jumping = false;
    }

    pub fn native_control_move(
        &mut self,
        owner: usize,
        target: [i32; 3],
        speed: f64,
        options: &NativeMobPathOptions,
    ) -> bool {
        if !self.native_control_owned(owner) || !speed.is_finite() || speed <= 0.0 {
            return false;
        }
        let Ok(options) = PathOptions::new(self, options) else {
            return false;
        };
        let state = self.native_control.as_mut().unwrap();
        state.command = Some(Command::Move(MoveRequest {
            target,
            speed: speed.min(4.0),
            options,
        }));
        state.navigation = NativeMobNavigation {
            state: NativeMobNavigationState::Pending,
            target: Some(target),
            ..NativeMobNavigation::default()
        };
        true
    }

    pub fn native_control_stop(&mut self, owner: usize) -> bool {
        if !self.native_control_owned(owner) {
            return false;
        }
        let state = self.native_control.as_mut().unwrap();
        state.command = Some(Command::Stop);
        state.look = None;
        state.jump = false;
        state.navigation.state = NativeMobNavigationState::Pending;
        true
    }

    pub fn native_control_find_path(
        &self,
        owner: usize,
        world: &MobWorld,
        target: [i32; 3],
        options: &NativeMobPathOptions,
    ) -> Result<Option<NativeMobPath>, String> {
        if !self.native_control_owned(owner) {
            return Ok(None);
        }
        let options = PathOptions::new(self, options)?;
        Ok(self
            .native_navigation_query(world, target, &options)
            .map(|path| NativeMobPath {
                target: path.target,
                waypoints: (0..path.nodes.len())
                    .map(|index| {
                        let position = self.path_foot_position(world, &path, index);
                        [position.x, position.y, position.z]
                    })
                    .collect(),
                reaches_target: path.reached,
            }))
    }

    pub fn native_control_look(&mut self, owner: usize, target: [f64; 3]) -> bool {
        if !self.native_control_owned(owner) || !target.iter().all(|value| value.is_finite()) {
            return false;
        }
        self.native_control.as_mut().unwrap().look = Some(target);
        true
    }

    pub fn native_control_jump(&mut self, owner: usize) -> bool {
        if !self.native_control_owned(owner) {
            return false;
        }
        self.native_control.as_mut().unwrap().jump = true;
        true
    }

    pub fn native_control_navigation(&self, owner: usize) -> Option<NativeMobNavigation> {
        self.native_control_owned(owner)
            .then(|| self.native_control.as_ref().unwrap().navigation.clone())
    }

    pub(super) fn native_control_tick(&mut self, world: &MobWorld) {
        let Some(control) = &mut self.native_control else {
            return;
        };
        let command = control.command.take();
        let look = control.look.take();
        let jump = std::mem::take(&mut control.jump);
        match command {
            Some(Command::Move(request)) => self.native_navigation_start(world, request),
            Some(Command::Stop) => {
                self.native_navigation_reset();
                let control = self.native_control.as_mut().unwrap();
                control.route = None;
                control.navigation.state = NativeMobNavigationState::Stopped;
            }
            None => {}
        }
        self.nav_tick(world);
        self.native_navigation_progress();
        if self.native_control.as_ref().unwrap().navigation.state
            != NativeMobNavigationState::Moving
        {
            self.native_control_halt();
        }
        if let Some(target) = look {
            self.look_at(target);
        }
        if jump {
            self.ai.jump = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::{geometry::Vec3, random::RandomSource};

    fn walker() -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        std::sync::Arc::make_mut(&mut kind).species = Species::Zombie;
        Mob::new(1, kind, Vec3::new(0.5, 0.0, 0.5), 0.0)
    }

    #[test]
    fn search_defaults_and_overrides_are_bounded() {
        let mut mob = walker();
        mob.attrs.follow_range = 16.0;
        let defaults = PathOptions::new(&mob, &NativeMobPathOptions::default()).unwrap();
        assert_eq!(defaults.max_distance, 16.0);
        assert_eq!(defaults.max_visited, 256);
        let options = NativeMobPathOptions {
            reach: i32::MAX,
            max_distance: Some(f32::MAX),
            max_visited: Some(u32::MAX),
            can_float: Some(true),
            allow_partial: false,
        };
        let bounded = PathOptions::new(&mob, &options).unwrap();
        assert_eq!(bounded.reach, 16);
        assert_eq!(bounded.max_distance, 128.0);
        assert_eq!(bounded.max_visited, 4096);
        assert!(bounded.can_float);
        assert!(!bounded.allow_partial);
        let minimum = PathOptions::new(
            &mob,
            &NativeMobPathOptions {
                reach: -1,
                max_distance: Some(-1.0),
                max_visited: Some(0),
                ..NativeMobPathOptions::default()
            },
        )
        .unwrap();
        assert_eq!(minimum.reach, 0);
        assert_eq!(minimum.max_distance, 1.0);
        assert_eq!(minimum.max_visited, 1);
        mob.attrs.follow_range = f64::NAN;
        let fallback = PathOptions::new(&mob, &NativeMobPathOptions::default()).unwrap();
        assert_eq!(fallback.max_distance, 16.0);
        assert_eq!(fallback.max_visited, 256);
    }

    #[test]
    fn invalid_requests_preserve_the_last_accepted_command() {
        let mut mob = walker();
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        assert!(mob.native_control_move(7, [4, 0, 0], 1.0, &NativeMobPathOptions::default()));
        let accepted = mob.native_control_navigation(7).unwrap();
        for speed in [f64::NAN, f64::INFINITY, 0.0, -1.0] {
            assert!(!mob.native_control_move(
                7,
                [9, 0, 0],
                speed,
                &NativeMobPathOptions::default()
            ));
        }
        for distance in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let options = NativeMobPathOptions {
                max_distance: Some(distance),
                ..NativeMobPathOptions::default()
            };
            assert!(!mob.native_control_move(7, [9, 0, 0], 1.0, &options));
        }
        assert_eq!(mob.native_control_navigation(7).unwrap(), accepted);
        let Some(Command::Move(request)) = mob.native_control.as_ref().unwrap().command else {
            panic!("accepted movement must remain queued");
        };
        assert_eq!(request.target, [4, 0, 0]);
    }

    #[test]
    fn release_discards_commands_and_restores_navigation_flags() {
        let mut mob = walker();
        mob.ai.nav.can_float = true;
        mob.ai.nav.can_open_doors = true;
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        mob.ai.nav.can_float = false;
        mob.ai.nav.can_open_doors = false;
        mob.ai.mov.operation = Operation::MoveTo;
        mob.ai.nav.target_pos = Some([4, 0, 0]);
        mob.ai.xxa = 1.0;
        mob.ai.zza = 1.0;
        mob.ai.jump = true;
        assert!(mob.native_control_jump(7));
        assert!(mob.native_control_set(7, NativeMobControlMode::Vanilla));
        assert!(mob.native_control.is_none());
        assert!(mob.ai.nav.can_float);
        assert!(mob.ai.nav.can_open_doors);
        assert!(mob.ai.nav.target_pos.is_none());
        assert_eq!(mob.ai.mov.operation, Operation::Wait);
        assert_eq!(mob.ai.xxa, 0.0);
        assert_eq!(mob.ai.zza, 0.0);
        assert!(!mob.ai.jump);
        assert!(mob.native_control_navigation(7).is_none());
        assert!(mob.native_control_set(8, NativeMobControlMode::External));
    }

    #[test]
    fn finite_look_and_jump_are_queued_and_stop_cancels_them() {
        let mut mob = walker();
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        assert!(mob.native_control_look(7, [2.0, 3.0, 4.0]));
        assert!(mob.native_control_jump(7));
        assert!(!mob.native_control_look(7, [f64::NAN, 0.0, 0.0]));
        let control = mob.native_control.as_ref().unwrap();
        assert_eq!(control.look, Some([2.0, 3.0, 4.0]));
        assert!(control.jump);
        assert!(!mob.ai.jump);
        assert!(mob.native_control_stop(7));
        let control = mob.native_control.as_ref().unwrap();
        assert!(matches!(control.command, Some(Command::Stop)));
        assert!(control.look.is_none());
        assert!(!control.jump);
    }

    #[test]
    fn unclaimed_vanilla_does_not_change_navigation_or_random_draws() {
        let mut mob = walker();
        let mut baseline_random = mob.random.clone();
        mob.ai.mov.operation = Operation::MoveTo;
        mob.ai.nav.target_pos = Some([9, 0, 0]);
        assert!(mob.native_control.is_none());
        assert!(mob.native_control_set(7, NativeMobControlMode::Vanilla));
        assert!(mob.native_control.is_none());
        assert_eq!(mob.ai.mov.operation, Operation::MoveTo);
        assert_eq!(mob.ai.nav.target_pos, Some([9, 0, 0]));
        for _ in 0..16 {
            assert_eq!(mob.random.next_long(), baseline_random.next_long());
        }
    }

    #[test]
    fn external_is_owned_and_reentering_does_not_suspend_again() {
        let mut mob = walker();
        assert!(!mob.native_control_take_suspension());
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        assert!(mob.native_control_take_suspension());
        assert!(!mob.native_control_take_suspension());
        let state_address = &**mob.native_control.as_ref().unwrap() as *const NativeMobControl;
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        assert_eq!(
            &**mob.native_control.as_ref().unwrap() as *const NativeMobControl,
            state_address
        );
        assert!(!mob.native_control_take_suspension());
        assert!(!mob.native_control_set(8, NativeMobControlMode::External));
        assert!(!mob.native_control_set(8, NativeMobControlMode::Vanilla));
        assert!(!mob.native_control_move(8, [3, 0, 0], 1.0, &Default::default()));
        assert!(!mob.native_control_stop(8));
        assert!(!mob.native_control_look(8, [3.0, 1.0, 0.0]));
        assert!(!mob.native_control_jump(8));
        assert!(mob.native_control_navigation(8).is_none());
        assert!(mob.native_control.as_ref().unwrap().command.is_none());
        assert!(mob.native_control_set(7, NativeMobControlMode::Vanilla));
        assert!(mob.native_control_set(8, NativeMobControlMode::External));
        assert!(mob.native_control_take_suspension());
    }

    #[test]
    fn carrier_allowlist_is_explicit() {
        let mut mob = walker();
        for species in [
            Species::Zombie,
            Species::Husk,
            Species::ZombieVillager,
            Species::Villager,
            Species::WanderingTrader,
        ] {
            std::sync::Arc::make_mut(&mut mob.kind).species = species;
            assert!(mob.native_control_supported());
        }
        for species in [Species::Cow, Species::Drowned, Species::Mannequin] {
            std::sync::Arc::make_mut(&mut mob.kind).species = species;
            assert!(!mob.native_control_supported());
        }
        assert!(mob.native_control.is_none());
    }

    #[test]
    fn unsupported_carriers_are_rejected_without_allocating_control() {
        let mut mob = walker();
        mob.no_ai = true;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.no_ai = false;
        mob.extra.mount = Some(3);
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.extra.mount = None;
        mob.extra.passenger = true;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.extra.passenger = false;
        mob.extra.rider = Some(3);
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.extra.rider = None;
        mob.removed = true;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.removed = false;
        mob.health = 0.0;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.health = 10.0;
        mob.ai.mov.kind = MoveKind::Custom;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.ai.mov.kind = MoveKind::Ground;
        mob.ai.nav.kind = NavKind::Flying;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        mob.ai.nav.kind = NavKind::Ground;
        std::sync::Arc::make_mut(&mut mob.kind).species = Species::Cow;
        assert!(!mob.native_control_set(7, NativeMobControlMode::External));
        assert!(mob.native_control.is_none());
    }

    #[test]
    fn movement_is_queued_without_touching_the_live_controls() {
        let mut mob = walker();
        assert!(mob.native_control_set(7, NativeMobControlMode::External));
        mob.ai.mov.operation = Operation::MoveTo;
        mob.ai.nav.target_pos = Some([1, 0, 0]);
        let mut baseline_random = mob.random.clone();
        assert!(mob.native_control_move(7, [9, 0, 0], 9.0, &Default::default()));
        assert_eq!(mob.ai.mov.operation, Operation::MoveTo);
        assert_eq!(mob.ai.nav.target_pos, Some([1, 0, 0]));
        let control = mob.native_control.as_ref().unwrap();
        let Some(Command::Move(request)) = control.command else {
            panic!("movement must be queued");
        };
        assert_eq!(request.speed, 4.0);
        assert_eq!(request.target, [9, 0, 0]);
        assert_eq!(control.navigation.state, NativeMobNavigationState::Pending);
        assert_eq!(mob.random.next_long(), baseline_random.next_long());
    }
}
