//! `/horizon custommobs stats [type]`: the debug report on the custom mobs of a server.
//!
//! The command itself lives in `game/commands/horizon_cmd.rs`; this module only turns numbers into
//! lines, so the format can be tested without a game. Per custom type: how many mobs are alive,
//! and what their path searches did with the shared budget (`ai::budget`); then the budget's own
//! totals. Procedural evaluation (`horizon-mobmodel`'s `ProceduralState`) is not run by the server
//! yet and the mobs hold no such state, so the report says so instead of printing zeros.
use super::ai::budget::{PathStats, TypeSearchStats};
use std::collections::BTreeMap;

/// What the report reads.
pub struct Input<'a> {
    /// The ids of the loaded custom types, sorted.
    pub loaded: &'a [String],
    /// Live custom mobs by type id.
    pub mobs: &'a BTreeMap<String, u32>,
    /// Search counters by type id.
    pub searches: &'a BTreeMap<String, TypeSearchStats>,
    /// The budget's totals.
    pub path: PathStats,
    /// Nodes all custom mobs may visit in one tick.
    pub limit: i32,
}

/// The report for `filter` (a type id; none means every type), one string per line, or the error
/// message when no loaded type has that id.
pub fn report(input: &Input, filter: Option<&str>) -> Result<Vec<String>, String> {
    if let Some(f) = filter
        && !input.loaded.iter().any(|t| t == f)
    {
        return Err(format!("Unknown custom mob type {f}"));
    }
    let shown: Vec<&String> = input.loaded.iter().filter(|t| filter.is_none_or(|f| f == t.as_str())).collect();
    let mut lines = Vec::new();
    if shown.is_empty() {
        lines.push("No custom mob types are loaded".to_owned());
    }
    let (mut mobs, mut sum) = (0u32, TypeSearchStats::default());
    for t in &shown {
        let n = input.mobs.get(t.as_str()).copied().unwrap_or(0);
        let s = input.searches.get(t.as_str()).copied().unwrap_or_default();
        lines.push(format!(
            "{t}: {n} mobs, path searches {} granted / {} put off / {} while waiting, {} nodes",
            s.granted, s.waited, s.backed_off, s.nodes
        ));
        mobs += n;
        sum.granted += s.granted;
        sum.waited += s.waited;
        sum.backed_off += s.backed_off;
        sum.nodes += s.nodes;
    }
    if filter.is_none() && shown.len() > 1 {
        lines.push(format!(
            "Total: {mobs} mobs, path searches {} granted / {} put off / {} while waiting, {} nodes",
            sum.granted, sum.waited, sum.backed_off, sum.nodes
        ));
    }
    let p = input.path;
    lines.push(format!(
        "Path budget (all custom mobs): {} nodes a tick; {} ticks with searches, {} granted, {} put off, {} refused while waiting, {} nodes in all, busiest tick {}, last tick {}",
        input.limit, p.ticks, p.granted, p.waited, p.backed_off, p.nodes, p.busiest_tick, p.this_tick
    ));
    lines.push("Procedural evaluation: not run on the server yet, so there are no counters".to_owned());
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types() -> Vec<String> {
        vec!["demo:fox".into(), "demo:owl".into()]
    }
    fn fixture() -> (BTreeMap<String, u32>, BTreeMap<String, TypeSearchStats>) {
        let mobs = BTreeMap::from([("demo:fox".to_owned(), 3)]);
        let searches = BTreeMap::from([(
            "demo:fox".to_owned(),
            TypeSearchStats { granted: 10, waited: 4, backed_off: 6, nodes: 900 },
        )]);
        (mobs, searches)
    }
    fn input<'a>(
        loaded: &'a [String],
        mobs: &'a BTreeMap<String, u32>,
        searches: &'a BTreeMap<String, TypeSearchStats>,
    ) -> Input<'a> {
        let path = PathStats { ticks: 20, granted: 10, waited: 4, nodes: 900, busiest_tick: 300, this_tick: 12, backed_off: 6 };
        Input { loaded, mobs, searches, path, limit: 8192 }
    }

    #[test]
    fn the_report_lists_each_type_a_total_the_budget_and_what_is_missing() {
        let (mobs, searches) = fixture();
        let loaded = types();
        let lines = report(&input(&loaded, &mobs, &searches), None).unwrap();
        assert_eq!(lines[0], "demo:fox: 3 mobs, path searches 10 granted / 4 put off / 6 while waiting, 900 nodes");
        assert_eq!(lines[1], "demo:owl: 0 mobs, path searches 0 granted / 0 put off / 0 while waiting, 0 nodes");
        assert_eq!(lines[2], "Total: 3 mobs, path searches 10 granted / 4 put off / 6 while waiting, 900 nodes");
        assert!(lines[3].starts_with("Path budget (all custom mobs): 8192 nodes a tick; 20 ticks"), "{}", lines[3]);
        assert!(lines[3].ends_with("900 nodes in all, busiest tick 300, last tick 12"), "{}", lines[3]);
        assert_eq!(lines[4], "Procedural evaluation: not run on the server yet, so there are no counters");
        assert_eq!(lines.len(), 5);
    }

    #[test]
    fn a_type_filter_shows_one_type_and_an_unknown_one_is_an_error() {
        let (mobs, searches) = fixture();
        let loaded = types();
        let lines = report(&input(&loaded, &mobs, &searches), Some("demo:fox")).unwrap();
        assert_eq!(lines.len(), 3, "the type, the budget, the procedural note: {lines:?}");
        assert!(lines[0].starts_with("demo:fox: 3 mobs"));
        let e = report(&input(&loaded, &mobs, &searches), Some("demo:cow")).unwrap_err();
        assert_eq!(e, "Unknown custom mob type demo:cow");
    }

    #[test]
    fn without_custom_types_the_report_says_so() {
        let none = BTreeMap::new();
        let none2 = BTreeMap::new();
        let lines = report(&input(&[], &none, &none2), None).unwrap();
        assert_eq!(lines[0], "No custom mob types are loaded");
        assert_eq!(lines.len(), 3);
    }
}
