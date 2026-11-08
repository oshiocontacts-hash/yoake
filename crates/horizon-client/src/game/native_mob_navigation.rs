//! Caller-owned commands for authoritative ground-walker AI ticks.

use super::{Game, commands::Target};
use horizon_client::mod_api::{
    EntityRef, NativeMobControlMode, NativeMobNavigation, NativeMobPath, NativeMobPathOptions,
};

impl Game<'_> {
    fn native_mob_control_context(&self, mob: EntityRef) -> Result<(usize, usize), String> {
        super::native_entity_query::require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )?;
        // CHECK: Integrate the strict open, running server-tick lifecycle gate.
        if self.native_lifecycle.mob_control_tick().is_none() {
            return Err("native mob control requires an open authoritative server tick".into());
        }
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("native mob control requires an active native mod call")?;
        let Some(Target::Mob(id)) = self.native_entity_target(mob) else {
            return Err("native mob control received a stale or non-mob entity handle".into());
        };
        let index = self
            .mobs
            .iter()
            .position(|candidate| candidate.id == id && !candidate.removed)
            .ok_or("native mob control received a stale entity handle")?;
        Ok((caller.index, index))
    }

    pub(super) fn native_mob_set_control_impl(
        &mut self,
        mob: EntityRef,
        mode: NativeMobControlMode,
    ) -> bool {
        let Ok((owner, index)) = self.native_mob_control_context(mob) else {
            return false;
        };
        self.mobs[index].native_control_set(owner, mode)
    }

    pub(super) fn native_mob_move_to_impl(
        &mut self,
        mob: EntityRef,
        target: [i32; 3],
        speed: f64,
        options: &NativeMobPathOptions,
    ) -> bool {
        let Ok((owner, index)) = self.native_mob_control_context(mob) else {
            return false;
        };
        self.mobs[index].native_control_move(owner, target, speed, options)
    }

    pub(super) fn native_mob_find_path_impl(
        &mut self,
        mob: EntityRef,
        target: [i32; 3],
        options: &NativeMobPathOptions,
    ) -> Result<Option<NativeMobPath>, String> {
        let (owner, index) = self.native_mob_control_context(mob)?;
        let world = self.mob_world_base(&[]);
        self.mobs[index].native_control_find_path(owner, &world, target, options)
    }

    pub(super) fn native_mob_stop_impl(&mut self, mob: EntityRef) -> bool {
        let Ok((owner, index)) = self.native_mob_control_context(mob) else {
            return false;
        };
        self.mobs[index].native_control_stop(owner)
    }

    pub(super) fn native_mob_look_at_impl(&mut self, mob: EntityRef, target: [f64; 3]) -> bool {
        let Ok((owner, index)) = self.native_mob_control_context(mob) else {
            return false;
        };
        self.mobs[index].native_control_look(owner, target)
    }

    pub(super) fn native_mob_jump_impl(&mut self, mob: EntityRef) -> bool {
        let Ok((owner, index)) = self.native_mob_control_context(mob) else {
            return false;
        };
        self.mobs[index].native_control_jump(owner)
    }

    pub(super) fn native_mob_navigation_impl(&self, mob: EntityRef) -> Option<NativeMobNavigation> {
        let (owner, index) = self.native_mob_control_context(mob).ok()?;
        self.mobs[index].native_control_navigation(owner)
    }
}
