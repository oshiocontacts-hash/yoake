//! Bounded, authoritative entity discovery for native mods.

use std::borrow::Cow;

use super::entity::{Entity, EntityLists, entity_lists_of, for_each_entity};
use super::execution_role::ExecutionRole;
use super::native_entity_motion::corners;
use super::{Game, commands, dragon, end, fangs, potions};
use horizon_client::entities::{Cushion, ItemFrame, Painting};
use horizon_client::lan::guest::Guest;
use horizon_client::mod_api::{NativeTickId, NativeWorldEpoch};
use horizon_core::geometry::Vec3;

const MAX_RADIUS: f64 = 64.0;
const MAX_QUERIES: usize = 8;
const MAX_CANDIDATES: usize = 4096;
const MAX_RESULTS: usize = 256;

#[cfg(test)]
mod tests;

/// An entity intersecting a sphere in the current loaded server level.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct NativeEntityInfo {
    pub id: u32,
    pub dimension: String,
    pub kind: String,
    pub position: [f64; 3],
    pub bounding_box: [[f64; 3]; 2],
    pub alive: bool,
    pub player_uuid: Option<u128>,
}

impl NativeEntityInfo {
    /// Constructs a snapshot without consulting a world, including in mod tests.
    pub fn new(
        id: u32,
        dimension: impl Into<String>,
        kind: impl Into<String>,
        position: [f64; 3],
        bounding_box: [[f64; 3]; 2],
        alive: bool,
        player_uuid: Option<u128>,
    ) -> Self {
        Self {
            id,
            dimension: dimension.into(),
            kind: kind.into(),
            position,
            bounding_box,
            alive,
            player_uuid,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct QueryUsage {
    mod_index: usize,
    queries: usize,
    candidates: usize,
    results: usize,
}

#[derive(Default)]
pub(super) struct QueryBudgets {
    tick: Option<(NativeWorldEpoch, NativeTickId)>,
    mods: Vec<QueryUsage>,
}

impl QueryBudgets {
    fn usage(&self, tick: (NativeWorldEpoch, NativeTickId), mod_index: usize) -> QueryUsage {
        if self.tick == Some(tick) {
            if let Some(usage) = self.mods.iter().find(|usage| usage.mod_index == mod_index) {
                return *usage;
            }
        }
        QueryUsage {
            mod_index,
            ..QueryUsage::default()
        }
    }

    fn remaining(
        &self,
        tick: (NativeWorldEpoch, NativeTickId),
        mod_index: usize,
    ) -> Result<(usize, usize), String> {
        let usage = self.usage(tick, mod_index);
        if usage.queries >= MAX_QUERIES {
            return Err("native entity query limit exceeded (8 per mod per server tick)".into());
        }
        Ok((
            MAX_CANDIDATES - usage.candidates,
            MAX_RESULTS - usage.results,
        ))
    }

    fn reserve(
        &mut self,
        tick: (NativeWorldEpoch, NativeTickId),
        mod_index: usize,
        candidates: usize,
    ) -> &mut QueryUsage {
        if self.tick != Some(tick) {
            self.mods.clear();
            self.tick = Some(tick);
        }
        let usage = if let Some(index) = self
            .mods
            .iter()
            .position(|usage| usage.mod_index == mod_index)
        {
            &mut self.mods[index]
        } else {
            self.mods.push(QueryUsage {
                mod_index,
                ..QueryUsage::default()
            });
            self.mods.last_mut().unwrap()
        };
        usage.queries += 1;
        usage.candidates += candidates;
        usage
    }
}

#[derive(Clone, Copy)]
struct Sphere {
    center: [f64; 3],
    radius: f64,
}

impl Sphere {
    fn new(center: [f64; 3], radius: f64) -> Result<Self, String> {
        if !center.iter().all(|coordinate| coordinate.is_finite())
            || !radius.is_finite()
            || !(0.0..=MAX_RADIUS).contains(&radius)
        {
            return Err("native entity query needs a finite center and radius in 0..=64".into());
        }
        Ok(Self { center, radius })
    }

    fn intersects(self, bounding_box: [[f64; 3]; 2]) -> Result<bool, String> {
        let [minimum, maximum] = bounding_box;
        if (0..3).any(|axis| {
            !minimum[axis].is_finite()
                || !maximum[axis].is_finite()
                || minimum[axis] > maximum[axis]
        }) {
            return Err("native entity query encountered an invalid bounding box".into());
        }
        let mut distance_squared = 0.0;
        for axis in 0..3 {
            let nearest = self.center[axis].clamp(minimum[axis], maximum[axis]);
            let distance = (self.center[axis] - nearest).abs();
            if distance > self.radius {
                return Ok(false);
            }
            distance_squared += distance * distance;
        }
        Ok(distance_squared <= self.radius * self.radius)
    }
}

#[derive(Clone)]
struct Candidate<'world> {
    id: u32,
    dimension: &'world str,
    kind: Cow<'world, str>,
    position: [f64; 3],
    bounding_box: [[f64; 3]; 2],
    alive: bool,
    removed: bool,
    player_uuid: Option<u128>,
}

impl<'world> Candidate<'world> {
    fn named(
        id: u32,
        dimension: &'world str,
        kind: &'world str,
        position: [f64; 3],
        bounding_box: [[f64; 3]; 2],
    ) -> Self {
        Self {
            id,
            dimension,
            kind: Cow::Borrowed(kind),
            position,
            bounding_box,
            alive: true,
            removed: false,
            player_uuid: None,
        }
    }

    fn point(id: u32, dimension: &'world str, kind: &'world str, position: [f64; 3]) -> Self {
        Self::named(id, dimension, kind, position, [position; 2])
    }

    fn shared(entity: &'world impl Entity, dimension: &'world str) -> Self {
        Self {
            id: entity.id(),
            dimension,
            kind: entity.native_type(),
            position: coordinates(entity.position()),
            bounding_box: corners(entity.bounds()),
            alive: entity.native_query_alive(),
            removed: entity.native_query_removed(),
            player_uuid: None,
        }
    }

    fn guest(guest: &'world Guest) -> Result<Self, String> {
        let mut candidate = Self::named(
            player_id(guest.entity_id)?,
            &guest.dimension,
            "minecraft:player",
            guest.position,
            corners(guest.bounds()),
        );
        candidate.alive = guest.health > 0.0 && !guest.dead;
        candidate.removed = !guest.placed;
        candidate.player_uuid = Some(guest.uuid);
        Ok(candidate)
    }

    fn matches(&self, dimension: &str, sphere: Sphere) -> Result<bool, String> {
        if self.removed || self.dimension != dimension {
            return Ok(false);
        }
        if !self
            .position
            .iter()
            .all(|coordinate| coordinate.is_finite())
        {
            return Err("native entity query encountered an invalid position".into());
        }
        sphere.intersects(self.bounding_box)
    }

    fn snapshot(self) -> NativeEntityInfo {
        NativeEntityInfo::new(
            self.id,
            self.dimension,
            self.kind.into_owned(),
            self.position,
            self.bounding_box,
            self.alive,
            self.player_uuid,
        )
    }
}

fn player_id(id: i32) -> Result<u32, String> {
    u32::try_from(id).map_err(|_| "native entity query encountered a negative player id".into())
}

fn coordinates(position: Vec3) -> [f64; 3] {
    [position.x, position.y, position.z]
}

fn midpoint(bounding_box: [[f64; 3]; 2]) -> [f64; 3] {
    std::array::from_fn(|axis| bounding_box[0][axis] * 0.5 + bounding_box[1][axis] * 0.5)
}

struct QuerySources<'world, Bolts, Spawners> {
    dimension: &'world str,
    shared: EntityLists<'world>,
    guests: &'world [Guest],
    others: &'world [commands::OtherEntity],
    cushions: &'world [Cushion],
    frames: &'world [ItemFrame],
    paintings: &'world [Painting],
    potions: &'world potions::PotionState,
    thrown_count: usize,
    end: &'world end::EndState,
    eye_count: usize,
    bolts: Bolts,
    dragon: Option<&'world dragon::Dragon>,
    crystals: &'world [dragon::Crystal],
    fireballs: &'world [dragon::DragonFireball],
    clouds: &'world [dragon::Breath],
    knots: &'world std::collections::HashMap<[i32; 3], u32>,
    spawners: Spawners,
    fangs: &'world [fangs::Fangs],
}

impl<'world, Bolts, Spawners> QuerySources<'world, Bolts, Spawners>
where
    Bolts: ExactSizeIterator<Item = (u32, [f64; 3])>,
    Spawners: ExactSizeIterator<Item = (u32, [f64; 3])>,
{
    fn candidate_count(&self) -> usize {
        [
            self.shared.candidate_count(),
            self.guests.len(),
            self.others.len(),
            self.cushions.len(),
            self.frames.len(),
            self.paintings.len(),
            self.thrown_count,
            self.eye_count,
            self.bolts.len(),
            usize::from(self.dragon.is_some()),
            self.crystals.len(),
            self.fireballs.len(),
            self.clouds.len(),
            self.knots.len(),
            self.spawners.len(),
            self.fangs.len(),
        ]
        .into_iter()
        .fold(0usize, usize::saturating_add)
    }

    fn visit(
        self,
        visit: &mut dyn FnMut(Candidate<'world>) -> Result<(), String>,
    ) -> Result<(), String> {
        let dimension = self.dimension;
        for_each_entity!(self.shared, entity => {
            visit(Candidate::shared(entity, dimension))?;
        });
        for guest in self.guests {
            visit(Candidate::guest(guest)?)?;
        }
        for frame in self.frames {
            let (minimum, maximum) = frame.bounds();
            let bounding_box = [minimum, maximum];
            visit(Candidate::named(
                frame.id,
                dimension,
                frame.kind(),
                midpoint(bounding_box),
                bounding_box,
            ))?;
        }
        for painting in self.paintings {
            let (minimum, maximum) = painting.bounds();
            let bounding_box = [minimum, maximum];
            visit(Candidate::named(
                painting.id,
                dimension,
                "minecraft:painting",
                midpoint(bounding_box),
                bounding_box,
            ))?;
        }
        for cushion in self.cushions {
            let (minimum, maximum) = cushion.bounds();
            visit(Candidate::named(
                cushion.id,
                dimension,
                "minecraft:cushion",
                coordinates(cushion.position),
                [minimum, maximum],
            ))?;
        }
        for crystal in self.crystals {
            visit(Candidate::named(
                crystal.id,
                dimension,
                "minecraft:end_crystal",
                coordinates(crystal.position),
                corners(crystal.bounds()),
            ))?;
        }
        // CHECK: These stores have no shared box adapter; use a zero-size box at their position.
        for other in self.others {
            visit(Candidate::point(
                other.id,
                dimension,
                &other.kind,
                coordinates(other.position),
            ))?;
        }
        for (id, kind, _, position, _) in self.potions.thrown_tracked() {
            visit(Candidate::point(id, dimension, kind, coordinates(position)))?;
        }
        for (id, position, _) in self.end.eyes_tracked() {
            visit(Candidate::point(
                id,
                dimension,
                "minecraft:eye_of_ender",
                position,
            ))?;
        }
        for (id, position) in self.bolts {
            visit(Candidate::point(
                id,
                dimension,
                "minecraft:lightning_bolt",
                position,
            ))?;
        }
        if let Some(dragon) = self.dragon {
            let mut candidate = Candidate::point(
                dragon.id,
                dimension,
                "minecraft:ender_dragon",
                coordinates(dragon.position),
            );
            candidate.alive = dragon.alive();
            visit(candidate)?;
        }
        for fireball in self.fireballs {
            visit(Candidate::point(
                fireball.id,
                dimension,
                "minecraft:dragon_fireball",
                coordinates(fireball.position),
            ))?;
        }
        for cloud in self.clouds {
            visit(Candidate::point(
                cloud.id,
                dimension,
                "minecraft:area_effect_cloud",
                cloud.at,
            ))?;
        }
        for (position, id) in self.knots {
            visit(Candidate::point(
                *id,
                dimension,
                "minecraft:leash_knot",
                position.map(f64::from),
            ))?;
        }
        for (id, position) in self.spawners {
            visit(Candidate::point(
                id,
                dimension,
                "minecraft:ominous_item_spawner",
                position,
            ))?;
        }
        for fang in self.fangs {
            visit(Candidate::point(
                fang.id,
                dimension,
                "minecraft:evoker_fangs",
                fang.pos,
            ))?;
        }
        Ok(())
    }
}

fn require_server_context(
    role: ExecutionRole,
    remote: bool,
    acting_guest: bool,
    host_present: bool,
) -> Result<(), String> {
    if role == ExecutionRole::Client || remote {
        return Err("native entity queries require an authoritative server".into());
    }
    if acting_guest || !host_present {
        return Err("native entity query unavailable while the LAN host context is parked".into());
    }
    Ok(())
}

fn require_scoped_transients(
    thrown_count: usize,
    eye_count: usize,
    has_parked_levels: bool,
) -> Result<(), String> {
    if has_parked_levels && (thrown_count != 0 || eye_count != 0) {
        // CHECK: Thrown potions and eyes have no dimension and are not swapped with levels.
        return Err(
            "native entity query cannot attribute transient entities to a loaded level".into(),
        );
    }
    Ok(())
}

fn query_entities<'world>(
    budgets: &mut QueryBudgets,
    tick: (NativeWorldEpoch, NativeTickId),
    mod_index: usize,
    dimension: &str,
    sphere: Sphere,
    candidates: usize,
    visit: impl FnOnce(&mut dyn FnMut(Candidate<'world>) -> Result<(), String>) -> Result<(), String>,
) -> Result<Vec<NativeEntityInfo>, String> {
    let (remaining_candidates, remaining_results) = budgets.remaining(tick, mod_index)?;
    if candidates > remaining_candidates {
        return Err("native entity candidate limit exceeded (4096 per mod per server tick)".into());
    }
    let usage = budgets.reserve(tick, mod_index, candidates);
    let mut visited = 0;
    let mut matches: [Option<Candidate<'world>>; MAX_RESULTS] = std::array::from_fn(|_| None);
    let mut result_count = 0;
    visit(&mut |candidate| {
        visited += 1;
        if visited > candidates {
            return Err("native entity query adapter exceeded its candidate count".into());
        }
        if candidate.matches(dimension, sphere)? {
            if result_count == remaining_results {
                return Err(
                    "native entity result limit exceeded (256 per mod per server tick)".into(),
                );
            }
            matches[result_count] = Some(candidate);
            result_count += 1;
        }
        Ok(())
    })?;
    if visited != candidates {
        return Err("native entity query cannot enumerate an unnumbered entity".into());
    }
    let matches = &mut matches[..result_count];
    matches.sort_unstable_by_key(|candidate| candidate.as_ref().unwrap().id);
    if matches
        .windows(2)
        .any(|pair| pair[0].as_ref().unwrap().id == pair[1].as_ref().unwrap().id)
    {
        return Err("native entity query encountered duplicate network ids".into());
    }
    usage.results += result_count;
    Ok(matches
        .iter_mut()
        .map(|candidate| candidate.take().unwrap().snapshot())
        .collect())
}

impl Game<'_> {
    /// Entities whose boxes intersect a sphere in the current loaded server level.
    /// API @API:entity_query@. No chunks are loaded and errors return no partial list.
    pub(super) fn native_entities_in(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        radius: f64,
    ) -> Result<Vec<NativeEntityInfo>, String> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )?;
        let sphere = Sphere::new(center, radius)?;
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("native entity queries require an active native mod call")?;
        let tick = self
            .native_lifecycle
            .entity_query_tick()
            .ok_or("native entity queries require a ready server world")?;
        if dimension != self.dim.id() {
            return query_entities(
                &mut self.native_entity_queries,
                tick,
                caller.index,
                dimension,
                sphere,
                0,
                |_| Ok(()),
            );
        }
        let thrown_count = self.potions.thrown_ids().size_hint().0;
        let eye_count = self.end.eye_ids().size_hint().0;
        require_scoped_transients(
            thrown_count,
            eye_count,
            self.levels.parked_worlds().next().is_some(),
        )?;
        let sources = QuerySources {
            dimension: self.dim.id(),
            shared: entity_lists_of!(self),
            guests: self.lan.as_ref().map_or(&[], |host| &host.hub.guests),
            others: &self.others,
            cushions: &self.cushions,
            frames: &self.item_frames,
            paintings: &self.paintings,
            potions: &self.potions,
            thrown_count,
            end: &self.end,
            eye_count,
            bolts: self.bolts.iter().map(|bolt| (bolt.id, bolt.at)),
            dragon: self.dragon.as_ref(),
            crystals: &self.crystals,
            fireballs: &self.dragon_fireballs,
            clouds: &self.breath_clouds,
            knots: &self.knot_ids,
            spawners: self
                .ominous_spawners
                .iter()
                .map(|spawner| (spawner.id, spawner.position)),
            fangs: &self.fangs.list,
        };
        query_entities(
            &mut self.native_entity_queries,
            tick,
            caller.index,
            dimension,
            sphere,
            sources.candidate_count(),
            |visit| sources.visit(visit),
        )
    }
}
