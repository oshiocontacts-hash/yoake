use std::collections::{BTreeMap, BTreeSet};

use crate::lcs::{self, AlignedStmt};
use crate::{CallerSnapshot, Outcome, Record, Site};

type AnchorMap<'a> = BTreeMap<(&'a str, u64), Vec<usize>>;

#[derive(Clone, Copy)]
enum MatchKind {
    Confirmed,
    Moved,
    Changed,
    Fragile,
}

struct PreparedAlignment {
    aligned: Vec<AlignedStmt>,
    old_map: Vec<Option<usize>>,
    new_map: Vec<Option<usize>>,
}

/// Converts extracted sites into persisted records, assigning sequential IDs from `next_id`.
pub fn records_from_sites(sites: &[Site], next_id: &mut u32) -> Vec<Record> {
    sites
        .iter()
        .map(|site| {
            let id = *next_id;
            *next_id += 1;
            Record {
                id,
                caller: site.caller.clone(),
                callee: site.callee.clone(),
                anchor: site.anchor,
                stmt_path_hashes: site.stmt_path_hashes.clone(),
                context: site.context,
                dup_rank: site.dup_rank,
            }
        })
        .collect()
}

/// Re-identifies old call-site records against newly extracted sites.
pub fn reidentify(old: &[Record], new_sites: &[Site], next_id: &mut u32) -> Outcome {
    reidentify_with_snapshots(old, &[], new_sites, &[], next_id)
}

/// Re-identifies old call-site records using stored per-caller statement snapshots for diff alignment.
pub fn reidentify_with_snapshots(
    old: &[Record],
    old_snapshots: &[CallerSnapshot],
    new_sites: &[Site],
    new_snapshots: &[CallerSnapshot],
    next_id: &mut u32,
) -> Outcome {
    let mut callers: BTreeSet<&str> = BTreeSet::new();
    for rec in old {
        callers.insert(rec.caller.as_str());
    }
    for site in new_sites {
        callers.insert(site.caller.as_str());
    }

    let old_snap_map: BTreeMap<&str, &CallerSnapshot> =
        old_snapshots.iter().map(|s| (s.caller.as_str(), s)).collect();
    let new_snap_map: BTreeMap<&str, &CallerSnapshot> =
        new_snapshots.iter().map(|s| (s.caller.as_str(), s)).collect();

    let mut outcome = Outcome::default();
    for caller in callers {
        let old_recs: Vec<&Record> = old.iter().filter(|r| r.caller == caller).collect();
        let new_list: Vec<(usize, &Site)> = new_sites
            .iter()
            .enumerate()
            .filter(|(_, s)| s.caller == caller)
            .collect();
        let mut matcher = CallerMatcher::new(old_recs, new_list, &mut outcome);
        let alignment = matcher.prepare_alignment(
            old_snap_map.get(caller).copied(),
            new_snap_map.get(caller).copied(),
        );
        matcher.pass_exact();
        matcher.pass_alignment_identical(&alignment);
        matcher.pass_unique_anchor();
        let stmt_ambig = matcher.pass_alignment_gaps(&alignment);
        matcher.pass_duplicates();
        matcher.pass_finalize(next_id, stmt_ambig);
    }

    outcome.confirmed.sort_by_key(|&(id, _)| id);
    outcome.moved.sort_by_key(|&(id, _)| id);
    outcome.changed.sort_by_key(|&(id, _)| id);
    outcome.fragile.sort_by_key(|&(id, _)| id);
    outcome.ambiguous.sort_by_key(|(id, _)| *id);
    outcome.lost.sort_unstable();
    outcome.new.sort_by_key(|&(_, idx)| idx);
    outcome
}

struct CallerMatcher<'a> {
    old_recs: Vec<&'a Record>,
    new_sites: Vec<(usize, &'a Site)>,
    old_matched: Vec<bool>,
    new_matched: Vec<bool>,
    old_ambiguous: Vec<bool>,
    new_ambiguous: Vec<bool>,
    outcome: &'a mut Outcome,
}

impl<'a> CallerMatcher<'a> {
    fn new(
        old_recs: Vec<&'a Record>,
        new_sites: Vec<(usize, &'a Site)>,
        outcome: &'a mut Outcome,
    ) -> Self {
        let old_len = old_recs.len();
        let new_len = new_sites.len();
        Self {
            old_recs,
            new_sites,
            old_matched: vec![false; old_len],
            new_matched: vec![false; new_len],
            old_ambiguous: vec![false; old_len],
            new_ambiguous: vec![false; new_len],
            outcome,
        }
    }

    fn link(&mut self, oi: usize, ni: usize, kind: MatchKind) {
        self.old_matched[oi] = true;
        self.new_matched[ni] = true;
        let pair = (self.old_recs[oi].id, self.new_sites[ni].0);
        match kind {
            MatchKind::Confirmed => self.outcome.confirmed.push(pair),
            MatchKind::Moved => self.outcome.moved.push(pair),
            MatchKind::Changed => self.outcome.changed.push(pair),
            MatchKind::Fragile => self.outcome.fragile.push(pair),
        }
    }

    fn prepare_alignment(
        &self,
        old_snap: Option<&CallerSnapshot>,
        new_snap: Option<&CallerSnapshot>,
    ) -> PreparedAlignment {
        let (old_stmts, old_map) = match old_snap {
            Some(snap) => (
                snap.body_stmt_hashes.clone(),
                lcs::map_records_to_stmts(&self.old_recs, &snap.body_stmt_hashes),
            ),
            None => lcs::reconstruct_old_stmts(&self.old_recs),
        };
        let (new_stmts, new_map) = match new_snap {
            Some(snap) => (
                snap.body_stmt_hashes.clone(),
                self.new_sites.iter().map(|(_, s)| Some(s.stmt_index)).collect(),
            ),
            None => {
                let raw: Vec<&Site> = self.new_sites.iter().map(|(_, s)| *s).collect();
                lcs::reconstruct_new_stmts(&raw)
            }
        };
        let aligned = lcs::align_statements(&old_stmts, &new_stmts);
        PreparedAlignment {
            aligned,
            old_map,
            new_map,
        }
    }

    fn pass_exact(&mut self) {
        let mut old_counts: BTreeMap<(&str, u64, &[u64]), usize> = BTreeMap::new();
        for rec in &self.old_recs {
            *old_counts
                .entry((rec.callee.as_str(), rec.anchor, rec.stmt_path_hashes.as_slice()))
                .or_insert(0) += 1;
        }
        let mut new_counts: BTreeMap<(&str, u64, &[u64]), usize> = BTreeMap::new();
        for (_, site) in &self.new_sites {
            *new_counts
                .entry((site.callee.as_str(), site.anchor, site.stmt_path_hashes.as_slice()))
                .or_insert(0) += 1;
        }

        let mut old_groups: BTreeMap<(&str, u64, &[u64], (u64, u64)), Vec<usize>> = BTreeMap::new();
        for (oi, rec) in self.old_recs.iter().enumerate() {
            if self.old_matched[oi] {
                continue;
            }
            let stmt_key = (rec.callee.as_str(), rec.anchor, rec.stmt_path_hashes.as_slice());
            let top = rec.stmt_path_hashes.first().copied().unwrap_or(0);
            let adj_dup_changed = (rec.context.0 == top || rec.context.1 == top)
                && old_counts.get(&stmt_key) != new_counts.get(&stmt_key);
            if !adj_dup_changed {
                old_groups
                    .entry((
                        rec.callee.as_str(),
                        rec.anchor,
                        rec.stmt_path_hashes.as_slice(),
                        rec.context,
                    ))
                    .or_default()
                    .push(oi);
            }
        }

        let mut new_groups: BTreeMap<(&str, u64, &[u64], (u64, u64)), Vec<usize>> = BTreeMap::new();
        for (ni, (_, site)) in self.new_sites.iter().enumerate() {
            if self.new_matched[ni] {
                continue;
            }
            let stmt_key = (site.callee.as_str(), site.anchor, site.stmt_path_hashes.as_slice());
            let top = site.stmt_path_hashes.first().copied().unwrap_or(0);
            let adj_dup_changed = (site.context.0 == top || site.context.1 == top)
                && old_counts.get(&stmt_key) != new_counts.get(&stmt_key);
            if !adj_dup_changed {
                new_groups
                    .entry((
                        site.callee.as_str(),
                        site.anchor,
                        site.stmt_path_hashes.as_slice(),
                        site.context,
                    ))
                    .or_default()
                    .push(ni);
            }
        }

        for (key, o_list) in old_groups {
            if o_list.len() == 1
                && let Some(n_list) = new_groups.get(&key)
                && n_list.len() == 1
            {
                self.link(o_list[0], n_list[0], MatchKind::Confirmed);
            }
        }
    }

    fn pass_alignment_identical(&mut self, alignment: &PreparedAlignment) {
        for pair in &alignment.aligned {
            if !pair.identical {
                continue;
            }
            let mut old_by_callee: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (oi, rec) in self.old_recs.iter().enumerate() {
                if alignment.old_map[oi] == Some(pair.old_idx) {
                    old_by_callee.entry(rec.callee.as_str()).or_default().push(oi);
                }
            }
            let mut new_by_callee: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (ni, (_, site)) in self.new_sites.iter().enumerate() {
                if alignment.new_map[ni] == Some(pair.new_idx) {
                    new_by_callee.entry(site.callee.as_str()).or_default().push(ni);
                }
            }
            for (callee, o_list) in old_by_callee {
                if o_list.len() == 1
                    && let Some(n_list) = new_by_callee.get(callee)
                    && n_list.len() == 1
                {
                    let oi = o_list[0];
                    let ni = n_list[0];
                    if !self.old_matched[oi] && !self.new_matched[ni] {
                        self.link(oi, ni, MatchKind::Confirmed);
                    }
                }
            }
        }
    }

    fn group_unmatched_by_anchor(&self) -> (AnchorMap<'a>, AnchorMap<'a>) {
        let mut old_groups: AnchorMap<'a> = BTreeMap::new();
        for (oi, rec) in self.old_recs.iter().enumerate() {
            if !self.old_matched[oi] {
                old_groups.entry((rec.callee.as_str(), rec.anchor)).or_default().push(oi);
            }
        }
        let mut new_groups: AnchorMap<'a> = BTreeMap::new();
        for (ni, (_, site)) in self.new_sites.iter().enumerate() {
            if !self.new_matched[ni] {
                new_groups.entry((site.callee.as_str(), site.anchor)).or_default().push(ni);
            }
        }
        (old_groups, new_groups)
    }

    fn mark_ambiguous(&mut self, oi: usize, n_list: &[usize]) {
        self.old_ambiguous[oi] = true;
        for &ni in n_list {
            self.new_ambiguous[ni] = true;
        }
        let global_indices = n_list.iter().map(|&ni| self.new_sites[ni].0).collect();
        self.outcome.ambiguous.push((self.old_recs[oi].id, global_indices));
    }

    fn pass_unique_anchor(&mut self) {
        let (old_groups, new_groups) = self.group_unmatched_by_anchor();
        for (key, o_list) in old_groups {
            if o_list.len() == 1
                && let Some(n_list) = new_groups.get(&key)
                && n_list.len() == 1
            {
                self.link(o_list[0], n_list[0], MatchKind::Moved);
            }
        }
    }

    fn pass_alignment_gaps(&mut self, alignment: &PreparedAlignment) -> Vec<(usize, Vec<usize>)> {
        let mut stmt_ambig = Vec::new();
        for pair in &alignment.aligned {
            if pair.identical {
                continue;
            }
            let mut old_by_callee: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (oi, rec) in self.old_recs.iter().enumerate() {
                if !self.old_matched[oi] && alignment.old_map[oi] == Some(pair.old_idx) {
                    old_by_callee.entry(rec.callee.as_str()).or_default().push(oi);
                }
            }
            let mut new_by_callee: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
            for (ni, (_, site)) in self.new_sites.iter().enumerate() {
                if !self.new_matched[ni] && alignment.new_map[ni] == Some(pair.new_idx) {
                    new_by_callee.entry(site.callee.as_str()).or_default().push(ni);
                }
            }
            for (callee, o_list) in old_by_callee {
                if o_list.len() == 1
                    && let Some(n_list) = new_by_callee.get(callee)
                {
                    if n_list.len() == 1 {
                        self.link(o_list[0], n_list[0], MatchKind::Changed);
                    } else if n_list.len() >= 2 {
                        stmt_ambig.push((o_list[0], n_list.clone()));
                    }
                }
            }
        }
        stmt_ambig
    }

    fn pass_duplicates(&mut self) {
        let (old_groups, mut new_groups) = self.group_unmatched_by_anchor();
        for (key, mut o_list) in old_groups {
            if o_list.len() >= 2
                && let Some(mut n_list) = new_groups.remove(&key)
            {
                o_list.sort_by_key(|&oi| (self.old_recs[oi].dup_rank, oi));
                n_list.sort_by_key(|&ni| (self.new_sites[ni].1.dup_rank, ni));
                for (oi, ni) in o_list.into_iter().zip(n_list) {
                    self.link(oi, ni, MatchKind::Fragile);
                }
            }
        }
    }

    fn pass_finalize(&mut self, next_id: &mut u32, stmt_ambig: Vec<(usize, Vec<usize>)>) {
        let (old_groups, new_groups) = self.group_unmatched_by_anchor();
        for (key, o_list) in old_groups {
            if o_list.len() == 1
                && let Some(n_list) = new_groups.get(&key)
                && n_list.len() >= 2
            {
                self.mark_ambiguous(o_list[0], n_list);
            }
        }
        for (oi, n_list) in stmt_ambig {
            if self.old_matched[oi] || self.old_ambiguous[oi] {
                continue;
            }
            let avail: Vec<usize> = n_list
                .into_iter()
                .filter(|&ni| !self.new_matched[ni] && !self.new_ambiguous[ni])
                .collect();
            if avail.len() >= 2 {
                self.mark_ambiguous(oi, &avail);
            }
        }
        for (oi, rec) in self.old_recs.iter().enumerate() {
            if !self.old_matched[oi] && !self.old_ambiguous[oi] {
                self.outcome.lost.push(rec.id);
            }
        }
        for (ni, &(global_idx, _)) in self.new_sites.iter().enumerate() {
            if !self.new_matched[ni] && !self.new_ambiguous[ni] {
                let id = *next_id;
                *next_id += 1;
                self.outcome.new.push((id, global_idx));
            }
        }
    }
}
