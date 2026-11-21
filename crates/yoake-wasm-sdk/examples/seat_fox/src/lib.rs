//! Seat Fox (F4M-4d check 7, `tools/custom_mob_seats_e2e.py --seat-wasm`): a tiny `server-full`
//! module using the `service.mobs` permission that automatically places a nearby
//! `minecraft:chicken` onto seat 1 of a newly spawned `sample_mobs:cinder_fox` via `mobs::seat_set`.
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, mobs, types::Vec3};

const FOX_KIND: &str = "sample_mobs:cinder_fox";
const CHICKEN_KIND: &str = "minecraft:chicken";
const MAX_BOARD_DISTANCE: f64 = 4.0;

#[derive(Default)]
struct SeatFox {
    /// Custom foxes waiting to board a nearby chicken.
    foxes: Vec<u32>,
    /// Chickens currently in the level that have not yet been seated by this module.
    chickens: Vec<(u32, Vec3)>,
    /// Foxes that already had seat 1 populated.
    done: Vec<u32>,
}

fn dist(a: &Vec3, b: &Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl SeatFox {
    fn try_seat(&mut self, fox: u32) {
        if self.done.contains(&fox) || self.chickens.is_empty() {
            return;
        }
        let Ok(Some(info)) = mobs::info(fox) else {
            return;
        };
        let Some((idx, _)) = self
            .chickens
            .iter()
            .enumerate()
            .filter(|(_, (_, pos))| dist(&info.position, pos) <= MAX_BOARD_DISTANCE)
            .min_by(|(_, (_, a)), (_, (_, b))| {
                dist(&info.position, a).total_cmp(&dist(&info.position, b))
            })
        else {
            return;
        };
        let chicken = self.chickens[idx].0;
        if matches!(mobs::seat_set(fox, 1, Some(chicken)), Ok(true)) {
            self.chickens.swap_remove(idx);
            self.foxes.retain(|&id| id != fox);
            self.done.push(fox);
        }
    }
}

impl ServerMod for SeatFox {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::CUSTOM_MOB_SPAWNED,
            hook::MOB_TICK,
            hook::ENTITY_JOIN_LEVEL,
            hook::ENTITY_LEAVE_LEVEL,
        ]
    }

    fn custom_mob_spawned(&mut self, mob: u32, kind: String, _reason: String) {
        if kind == FOX_KIND && !self.done.contains(&mob) && !self.foxes.contains(&mob) {
            self.foxes.push(mob);
            self.try_seat(mob);
        }
    }

    fn mob_tick(&mut self, mob: u32) {
        if self.foxes.contains(&mob) {
            self.try_seat(mob);
        }
    }

    fn entity_join_level(
        &mut self,
        _dimension: String,
        entity: u32,
        kind: String,
        pos: Vec<f64>,
        loaded_from_disk: bool,
    ) -> bool {
        if !loaded_from_disk && kind == CHICKEN_KIND && pos.len() == 3 {
            let p = Vec3 {
                x: pos[0],
                y: pos[1],
                z: pos[2],
            };
            self.chickens.retain(|(id, _)| *id != entity);
            self.chickens.push((entity, p));
        }
        false
    }

    fn entity_leave_level(&mut self, _dimension: String, entity: u32, _kind: String) {
        self.chickens.retain(|(id, _)| *id != entity);
        self.foxes.retain(|&id| id != entity);
        self.done.retain(|&id| id != entity);
    }
}

yoake_wasm_sdk::export_server_full!(SeatFox);
