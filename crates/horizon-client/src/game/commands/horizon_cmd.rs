//! `/horizon ...`, Horizon's own debug commands (on with the other Horizon commands, game
//! masters only). Today: `/horizon custommobs stats [<type>]`.
use super::{CommandError, Context, Game, R};
use horizon_client::{
    command::Arg,
    mobs::custom::{self, ai::budget, stats},
    text::Text,
};
use std::collections::BTreeMap;

impl Game<'_> {
    pub(super) fn cmd_horizon(&mut self, id: &str, ctx: &Context) -> R<i32> {
        match id {
            "horizon.custommobs.stats" => {
                let filter = match ctx.get("type") {
                    Some(Arg::Resource(t)) => Some(t.as_str()),
                    _ => None,
                };
                let mut mobs: BTreeMap<String, u32> = BTreeMap::new();
                for m in &self.mobs {
                    if m.kind.custom.is_some() {
                        *mobs.entry(m.kind.id.clone()).or_default() += 1;
                    }
                }
                let loaded = custom::ids();
                let searches = budget::stats_by_type();
                let input = stats::Input {
                    loaded: &loaded,
                    mobs: &mobs,
                    searches: &searches,
                    path: budget::stats(),
                    limit: budget::limit(),
                };
                let lines = stats::report(&input, filter)
                    .map_err(|e| CommandError::new(Text::literal(e)))?;
                self.feedback_unbroadcast(Text::literal(lines.join("\n")));
                Ok(mobs.values().sum::<u32>() as i32)
            }
            _ => Err(CommandError::new(Text::literal("Unknown /horizon command"))),
        }
    }
}
