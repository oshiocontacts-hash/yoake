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
            "yoake.mobdebug" | "yoake.mobs" | "yoake.mob.reload" => {
                let words = match id {
                    "yoake.mobdebug" => "mobdebug",
                    "yoake.mobs" => "mobs",
                    _ => "mob reload",
                };
                let tail = match (ctx.get("options"), ctx.get("type")) {
                    (Some(Arg::Str(s)), _) | (_, Some(Arg::Resource(s))) => s.clone(),
                    _ => String::new(),
                };
                let lines = self.yoake_lines(&format!("{words} {tail}"));
                let count = lines.len() as i32;
                self.feedback_unbroadcast(Text::literal(lines.join("
")));
                Ok(count)
            }
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
            "horizon.custommobs.trigger" => {
                // Dev aid: fires an animation trigger of the targeted custom mobs as the server's own
                // animation runtime would (`anim_sync::request_trigger`); it plays from the next tick.
                let name = match ctx.get("name") {
                    Some(Arg::Str(n)) => n.clone(),
                    _ => return Err(CommandError::new(Text::literal("A trigger name is needed"))),
                };
                let targets = self.entities(ctx, "targets", false)?;
                let mut fired = 0;
                for target in targets {
                    let super::Target::Mob(id) = target else { continue };
                    let Some(mob) = self.mobs.iter_mut().find(|m| m.id == id) else { continue };
                    if custom::anim_sync::request_trigger(mob, &name, 1.0) {
                        fired += 1;
                    }
                }
                if fired == 0 {
                    return Err(CommandError::new(Text::literal(format!(
                        "No targeted custom mob has the trigger {name}"
                    ))));
                }
                self.feedback_unbroadcast(Text::literal(format!("Fired {name} on {fired} custom mob(s)")));
                Ok(fired)
            }
            _ => Err(CommandError::new(Text::literal("Unknown /horizon command"))),
        }
    }
}
