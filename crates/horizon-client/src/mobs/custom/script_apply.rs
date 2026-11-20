//! Carrying out what a script asked of a custom mob (the [`Command`]s of `script.rs`). Every
//! command is checked here, not in the service: a name the type does not have, a value of the wrong
//! type, a number that is not finite is logged at debug level and ignored.
use super::script::{self, Command, MAX_DATA_BYTES, synced_value};
use super::{attrs, anim_sync};
use crate::mobs::{
    Damage, Mob, MobEvent, MobWorld, Who,
};

fn finite(v: &[f64]) -> bool {
    v.iter().all(|n| n.is_finite())
}

/// The tracked value `key` of `m` as a script value (`None`: unknown name).
pub fn get_synced(m: &Mob, key: &str) -> Option<script::MobScriptValue> {
    let t = m.kind.custom.as_ref()?;
    let slot = t.synced_slot(key)?;
    Some(script::script_value(&t.synced[slot], m.extra.custom.as_ref()?.synced.get(slot)?))
}

/// Applies `c` to `m` where the context allows; false when the command needs a world or an event
/// queue the caller has not got (it is put back for the mob's next tick).
pub fn apply_in(m: &mut Mob, w: Option<&MobWorld>, events: Option<&mut Vec<MobEvent>>, c: Command) -> bool {
    let Some(t) = m.kind.custom.clone() else {
        return true;
    };
    match c {
        Command::SetSynced { key, value } => {
            let Some(slot) = t.synced_slot(&key) else {
                log::debug!("{}: script: no tracked value {key:?}", t.id);
                return true;
            };
            let Some(v) = synced_value(&t.synced[slot], &value) else {
                log::debug!("{}: script: {key:?} is not a {}", t.id, t.synced[slot].kind);
                return true;
            };
            if let Some(state) = m.extra.custom.as_mut() {
                state.set_synced_slot(&t, slot, v);
            }
        }
        Command::TriggerAnim { name, speed } => {
            if !speed.is_finite() || !anim_sync::request_trigger(m, &name, speed.clamp(0.0, 15.9)) {
                log::debug!("{}: script: no trigger {name:?}", t.id);
            }
        }
        Command::SetControllerState { controller, state } => {
            if anim_sync::force_state(m, &controller, &state) {
                // Entering a state by itself lets go of an earlier hold of the controller.
                anim_sync::release_state(m, &controller);
            } else {
                log::debug!("{}: script: no controller {controller:?} or state {state:?}", t.id);
            }
        }
        Command::HoldControllerState { controller, state, hold_ticks } => {
            if !anim_sync::hold_state(m, &controller, &state, hold_ticks) {
                log::debug!("{}: script: no controller {controller:?} or state {state:?}", t.id);
            }
        }
        Command::ReleaseControllerState { controller } => {
            if !anim_sync::release_state(m, &controller) {
                log::debug!("{}: script: no controller {controller:?}", t.id);
            }
        }
        Command::NavigateTo { position, speed } => {
            let Some(w) = w else { return false };
            if finite(&position) && speed.is_finite() && speed > 0.0 {
                m.nav_move_to(w, position, speed.min(4.0));
            }
        }
        Command::NavigateStop => m.nav_stop(),
        Command::LookAt { position, yaw_speed, pitch_speed } => {
            if finite(&position) && yaw_speed.is_finite() && pitch_speed.is_finite() {
                m.look_at_speed(position, yaw_speed.clamp(0.0, 360.0), pitch_speed.clamp(0.0, 360.0));
            }
        }
        Command::Jump => m.ai.jumping = true,
        Command::SetTarget { mob, player } => {
            m.ai.target = if player { Some(Who::Player) } else { mob.map(Who::Mob) };
        }
        Command::Attack { mob, player, damage, knockback } => {
            let Some(events) = events else { return false };
            if !damage.is_finite() || !knockback.is_finite() || damage < 0.0 {
                return true;
            }
            let who = if player { Some(Who::Player) } else { mob.map(Who::Mob) };
            let Some(who) = who else { return true };
            let from = [m.position.x, m.position.y, m.position.z];
            let source = Damage::mob(m.id, from);
            m.swing();
            match who {
                Who::Player => events.push(MobEvent::AttackPlayer(damage, from)),
                Who::Mob(target) => events.push(MobEvent::AttackMob { target, amount: damage, source }),
            }
            if knockback > 0.0 {
                let yaw = m.yaw.to_radians();
                events.push(MobEvent::Knockback {
                    target: who,
                    power: f64::from(knockback),
                    x: f64::from(yaw.sin()),
                    z: f64::from(-yaw.cos()),
                    source,
                });
            }
        }
        Command::SetAttributeBase { attribute, value } => {
            if let Some(s) = m.extra.custom.as_mut()
                && (!value.is_finite() || !s.attrs.set_base(&attribute, value))
            {
                log::debug!("{}: script: cannot set attribute {attribute:?}", t.id);
            }
        }
        Command::AddModifier { attribute, modifier, amount, operation } => {
            let op = attrs::operation(&operation.replace('-', "_"));
            if let (Some(s), Some(op)) = (m.extra.custom.as_mut(), op)
                && (!amount.is_finite() || !s.attrs.add_modifier(&attribute, &modifier, amount, op))
            {
                log::debug!("{}: script: cannot add modifier {modifier:?} to {attribute:?}", t.id);
            }
        }
        Command::RemoveModifier { attribute, modifier } => {
            if let Some(s) = m.extra.custom.as_mut() {
                s.attrs.remove_modifier(&attribute, &modifier);
            }
        }
        Command::SetHitbox { name, enabled } => {
            if let Some(part) = t.hitboxes.iter().position(|h| h.name == name)
                && let Some(s) = m.extra.custom.as_mut()
            {
                s.set_hitbox_enabled(&t, part, enabled);
            } else {
                log::debug!("{}: script: no hitbox part {name:?}", t.id);
            }
        }
        Command::PutData(bytes) => {
            if bytes.len() <= MAX_DATA_BYTES
                && let Some(s) = script::state(m)
            {
                s.data = bytes;
            }
        }
        Command::Despawn => {
            m.removed = true;
        }
        Command::Claim(on) => {
            // The goal selector is held apart while the goals run, so the flags are put on it by
            // `script::sync_claim` at the start of the mob's next tick.
            if let Some(s) = script::state(m) {
                s.claimed = on;
            }
            script::sync_claim(m);
        }
        Command::Impulse(v) => {
            if finite(&v) {
                m.velocity.x += v[0].clamp(-4.0, 4.0);
                m.velocity.y += v[1].clamp(-4.0, 4.0);
                m.velocity.z += v[2].clamp(-4.0, 4.0);
            }
        }
    }
    true
}
