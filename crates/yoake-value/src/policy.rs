use crate::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rule {
    Unrestricted,
    Finite,
    Number(f64, f64),
    ClampedNumber(f64, f64),
    Vector(f64, f64),
    BlockState,
    XpMerge,
    Subset,
}

impl Rule {
    pub fn accepts(self, value: &Value, block_states: Option<usize>) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Finite => match value {
                Value::Float(value) => value.is_finite(),
                Value::List(values) => values.iter().all(|value| self.accepts(value, block_states)),
                Value::Record(values) => values
                    .values()
                    .all(|value| self.accepts(value, block_states)),
                _ => true,
            },
            Self::Number(minimum, maximum) | Self::ClampedNumber(minimum, maximum) => {
                let number = match value {
                    Value::Signed(value) => *value as f64,
                    Value::Unsigned(value) => *value as f64,
                    Value::Float(value) => *value,
                    _ => return false,
                };
                number.is_finite() && (minimum..=maximum).contains(&number)
            }
            Self::Vector(minimum, maximum) => match value {
                Value::List(values) if values.len() == 3 => values
                    .iter()
                    .all(|value| Self::Number(minimum, maximum).accepts(value, block_states)),
                _ => false,
            },
            Self::BlockState => match (value, block_states) {
                (Value::Unsigned(value), Some(count)) => *value < count as u64,
                _ => false,
            },
            Self::XpMerge | Self::Subset => matches!(value, Value::List(_)),
        }
    }

    pub fn normalize_return(self, mut value: Value, original: &Value) -> Option<Value> {
        match self {
            Self::ClampedNumber(minimum, maximum) => match &mut value {
                Value::Float(number) if number.is_finite() => {
                    *number = number.clamp(minimum, maximum);
                }
                _ => return None,
            },
            Self::XpMerge => {
                let (Value::List(values), Value::List(original)) = (&value, original) else {
                    return None;
                };
                let sum = |values: &[Value]| {
                    values.iter().try_fold(0_u64, |total, value| match value {
                        Value::Unsigned(amount) if (1..=u64::from(u32::MAX)).contains(amount) => {
                            total.checked_add(*amount)
                        }
                        _ => None,
                    })
                };
                if values.len() > original.len() || sum(values)? != sum(original)? {
                    return None;
                }
            }
            Self::Subset => {
                let (Value::List(values), Value::List(original)) = (&value, original) else {
                    return None;
                };
                if values.len() > original.len() {
                    return None;
                }
                let mut remaining: Vec<_> = original.iter().collect();
                for entry in values {
                    let index = remaining.iter().position(|candidate| *candidate == entry)?;
                    remaining.swap_remove(index);
                }
            }
            _ => {}
        }
        Some(value)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub arguments: &'static [(usize, Rule)],
    pub result: Rule,
    pub commands: bool,
    pub client_chat: bool,
}

pub fn site_policy(path: &str) -> Policy {
    use Rule::{BlockState, ClampedNumber, Finite, Number, Subset, Vector, XpMerge};

    let arguments: &'static [(usize, Rule)] = match path {
        "world::level::on_place" => &[
            (1, BlockState),
            (2, Vector(-30_000_000.0, 30_000_000.0)),
            (3, BlockState),
        ],
        "world::level::tick::crop_random_tick" | "game::drop_falling_block" => {
            &[(1, BlockState), (2, Vector(-30_000_000.0, 30_000_000.0))]
        }
        "world::level::bonemeal::crop_bonemeal_increase" => &[(1, BlockState)],
        "game::explosion::server_explode" => &[
            (1, Vector(-30_000_000.0, 30_000_000.0)),
            (2, Number(0.0, 128.0)),
        ],
        "game::block_entity_ticks::tick_furnace"
        | "game::break_block"
        | "game::commands::unpack_opened_loot"
        | "game::machines::crafter::crafter_eject"
        | "game::place_used_fire"
        | "game::try_sleep"
        | "world::level::tick::fire_ignite_odds_at" => &[(1, Vector(-30_000_000.0, 30_000_000.0))],
        "world::level::tick::fire_burn_odds_at" => &[
            (0, Vector(-30_000_000.0, 30_000_000.0)),
            (1, Number(0.0, 100.0)),
        ],
        "world::food::exhaustion_amount" => &[(0, Number(0.0, 40.0))],
        "game::split_orb_values" => &[(0, Number(0.0, u32::MAX as f64))],
        "world::level::tick::crop_growth_speed" => &[(2, Vector(-30_000_000.0, 30_000_000.0))],
        "game::creatures::drop_death_loot" => &[(5, Vector(-30_000_000.0, 30_000_000.0))],
        "game::creatures::hurt_player_by" | "game::creatures::mob_hits_mob" => {
            &[(2, Number(0.0, 1_000_000.0))]
        }
        "game::creatures::mob_hits_player" => &[
            (1, Number(0.0, 1_000_000.0)),
            (2, Vector(-30_000_000.0, 30_000_000.0)),
        ],
        "world::food::add_exhaustion" => &[(1, Number(0.0, 40.0))],
        "world::food::eat_properties" => &[(1, Number(0.0, 20.0)), (2, Number(0.0, 40.0))],
        "game::spawn_item" => &[
            (2, Vector(-30_000_000.0, 30_000_000.0)),
            (3, Vector(-1024.0, 1024.0)),
            (4, Number(0.0, 32_767.0)),
        ],
        "game::spawn_orbs" => &[
            (1, Vector(-30_000_000.0, 30_000_000.0)),
            (2, Number(0.0, 1_000_000.0)),
        ],
        "game::commands::change_experience" => &[(3, Number(-1_000_000.0, 1_000_000.0))],
        "items::enchant::enchantment_cost" => &[(2, Number(0.0, 2.0)), (3, Number(0.0, 15.0))],
        "world::living_fall::calculate_fall_damage"
        | "world::living_fall::cause_living_fall_damage" => {
            &[(1, Number(-1024.0, 1_000_000.0)), (2, Number(0.0, 1024.0))]
        }
        "world::living_motion::jump_power" => &[(1, Number(0.0, 1024.0))],
        "game::tick_sleep_timer" => &[(0, Number(0.0, 110.0))],
        _ => &[],
    };
    let result = match path {
        "crafting::crafting_result_count" | "furnace::smelting_result_count" => Number(0.0, 99.0),
        "game::attack_strength_scale" => Number(0.0, 1.0),
        "game::hand::attack_damage" => Number(0.0, 1_000_000.0),
        "game::hand::main_hand_attack_speed" => Number(0.01, 1024.0),
        "game::tick_sleep_timer" => Number(0.0, 110.0),
        "world::level::tick::crop_growth_speed" => Number(0.01, 1024.0),
        "world::level::tick::fire_ignite_odds_at" => Number(0.0, 10_000.0),
        "world::level::tick::fire_burn_odds_at" => Number(0.0, 100.0),
        "world::food::exhaustion_amount" => ClampedNumber(0.0, 40.0),
        "game::split_orb_values" => XpMerge,
        "game::explosion::affected_blocks" | "game::sleeping_players" => Subset,
        "world::level::bonemeal::crop_bonemeal_increase" => Number(0.0, 25.0),
        "world::living_fall::calculate_fall_damage" => Number(0.0, i32::MAX as f64),
        "world::living_motion::jump_power" => Number(0.0, 64.0),
        "items::enchant::enchantment_cost" => Number(0.0, 1_000_000.0),
        "game::commands::execute::execute_line" | "game::commands::execute_command" => {
            Number(i32::MIN as f64, i32::MAX as f64)
        }
        _ => Finite,
    };
    let commands = matches!(
        path,
        "game::chat::handle_chat_input"
            | "game::commands::execute::execute_line"
            | "game::commands::execute_command"
    );
    Policy {
        arguments,
        result,
        commands,
        client_chat: path == "game::chat::handle_chat_input",
    }
}

pub const DENIED_COMMANDS: &[&str] = &[
    "transfer",
    "fetchprofile",
    "publish",
    "unpublish",
    "debug",
    "jfr",
    "perf",
    "stop",
    "save-off",
    "save-on",
    "op",
    "deop",
    "whitelist",
    "ban",
    "ban-ip",
    "pardon",
    "pardon-ip",
    "setidletimeout",
    "test",
    "compute",
    "pregen",
    "schedule",
    "clone",
    "data",
    "item",
    "reload",
    "datapack",
];

pub fn command_gate(line: &str, declared: bool) -> Result<(), String> {
    if !declared {
        return Err("denied: the package does not declare service.commands".into());
    }
    if line.len() > 65_536 {
        return Err("command too long".into());
    }
    if line.chars().any(char::is_control) {
        return Err("denied: a command is one line of plain text".into());
    }
    let mut remaining = line;
    let mut depth = 0;
    loop {
        let token = required_argument(&mut remaining)?;
        let name = token.trim_start_matches('/').to_ascii_lowercase();
        let name = name.strip_prefix("minecraft:").unwrap_or(&name);
        if DENIED_COMMANDS.contains(&name) {
            return Err(format!(
                "denied: /{name} is not available to WebAssembly mods"
            ));
        }
        let nested = match name {
            "execute" => execute_tail(&mut remaining)?,
            "return" => command_argument(&mut remaining)? == Some("run"),
            "setblock" | "fill" if !remaining.trim().is_empty() => {
                placed_block_gate(&mut remaining, if name == "fill" { 6 } else { 3 })?;
                false
            }
            "give" if !remaining.trim().is_empty() => {
                required_argument(&mut remaining)?;
                let item = required_argument(&mut remaining)?;
                if item.contains(['[', '{']) {
                    return Err("denied: item components can carry deferred commands".into());
                }
                false
            }
            "summon" if !remaining.trim().is_empty() => {
                let entity = required_argument(&mut remaining)?;
                if entity.trim_start_matches("minecraft:") == "command_block_minecart"
                    || remaining.contains('{')
                {
                    return Err("denied: entity NBT can carry deferred commands".into());
                }
                false
            }
            _ => false,
        };
        if !nested {
            return Ok(());
        }
        depth += 1;
        if depth > 16 {
            return Err("denied: commands nested too deep".into());
        }
    }
}

fn command_argument<'a>(remaining: &mut &'a str) -> Result<Option<&'a str>, String> {
    let input = remaining.trim_start();
    let mut brackets = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
        } else if quote.is_some() && character == '\\' {
            escaped = true;
        } else if quote == Some(character) {
            quote = None;
        } else if quote.is_none() {
            match character {
                '\'' | '"' => quote = Some(character),
                '[' => brackets.push(']'),
                '{' => brackets.push('}'),
                '(' => brackets.push(')'),
                ']' | '}' | ')' if brackets.pop() != Some(character) => {
                    return Err("denied: malformed command argument".into());
                }
                _ if character.is_whitespace() && brackets.is_empty() => {
                    *remaining = &input[index..];
                    return Ok(Some(&input[..index]));
                }
                _ => {}
            }
        }
    }
    if quote.is_some() || !brackets.is_empty() {
        return Err("denied: malformed command argument".into());
    }
    *remaining = "";
    Ok((!input.is_empty()).then_some(input))
}

fn required_argument<'a>(remaining: &mut &'a str) -> Result<&'a str, String> {
    command_argument(remaining)?.ok_or_else(|| "denied: missing command argument".into())
}

fn skip_arguments(remaining: &mut &str, count: usize) -> Result<(), String> {
    for _ in 0..count {
        required_argument(remaining)?;
    }
    Ok(())
}

fn placed_block_gate(remaining: &mut &str, coordinates: usize) -> Result<(), String> {
    skip_arguments(remaining, coordinates)?;
    let block = required_argument(remaining)?.to_ascii_lowercase();
    let name = block.split(['[', '{']).next().unwrap_or_default();
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    if name.ends_with("command_block")
        || name == "sign"
        || name.ends_with("_sign")
        || block.contains('{')
    {
        return Err("denied: command blocks, signs and block entity NBT require player commands".into());
    }
    Ok(())
}

fn execute_tail(remaining: &mut &str) -> Result<bool, String> {
    while let Some(modifier) = command_argument(remaining)? {
        match modifier {
            "run" => return Ok(true),
            "as" | "at" | "align" | "anchored" | "in" | "summon" | "on" => skip_arguments(remaining, 1)?,
            "positioned" => {
                let kind = required_argument(remaining)?;
                skip_arguments(remaining, if matches!(kind, "as" | "over") { 1 } else { 2 })?;
            }
            "facing" => skip_arguments(remaining, 3)?,
            "rotated" => skip_arguments(remaining, 2)?,
            "store" => {
                required_argument(remaining)?;
                match required_argument(remaining)? {
                    "score" | "bossbar" => skip_arguments(remaining, 2)?,
                    _ => return Err("denied: execute store can write deferred command data".into()),
                }
            }
            "if" | "unless" => {
                let condition = required_argument(remaining)?;
                match condition {
                    "entity" | "predicate" | "function" | "dimension" => skip_arguments(remaining, 1)?,
                    "stopwatch" => skip_arguments(remaining, 2)?,
                    "loaded" => skip_arguments(remaining, 3)?,
                    "block" | "biome" => skip_arguments(remaining, 4)?,
                    "blocks" => skip_arguments(remaining, 10)?,
                    "data" | "items" | "slots" => {
                        let provider = required_argument(remaining)?;
                        let count = match provider {
                            "block" => 4,
                            "entity" | "storage" if condition == "data" || provider == "entity" => 2,
                            _ => return Err("denied: unsupported execute provider".into()),
                        };
                        skip_arguments(remaining, count + usize::from(condition == "items"))?;
                    }
                    "score" => {
                        skip_arguments(remaining, 2)?;
                        let comparison = required_argument(remaining)?;
                        skip_arguments(remaining, if comparison == "matches" { 1 } else { 2 })?;
                    }
                    _ => return Err("denied: unsupported execute condition".into()),
                }
            }
            _ => return Err("denied: unsupported execute modifier".into()),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_states_use_the_live_registry_domain() {
        for path in [
            "world::level::on_place",
            "world::level::tick::crop_random_tick",
            "world::level::bonemeal::crop_bonemeal_increase",
            "game::drop_falling_block",
        ] {
            for (_, rule) in site_policy(path).arguments {
                if *rule != Rule::BlockState {
                    continue;
                }
                assert!(rule.accepts(&Value::Unsigned(3), Some(4)));
                assert!(!rule.accepts(&Value::Unsigned(4), Some(4)));
                assert!(!rule.accepts(&Value::Unsigned(4_000_000_000), Some(4)));
                assert!(!rule.accepts(&Value::Unsigned(0), None));
            }
        }
    }

    #[test]
    fn numbers_are_finite_including_nested_vectors() {
        for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(!Rule::Finite.accepts(&Value::List(vec![Value::Float(number)]), None));
        }
        assert!(Rule::Finite.accepts(&Value::Float(-1.0), None));
        let position = Rule::Vector(-30_000_000.0, 30_000_000.0);
        assert!(position.accepts(&Value::List(vec![Value::Signed(0); 3]), None));
        assert!(!position.accepts(&Value::List(vec![Value::Float(f64::NAN); 3]), None));
        assert!(!position.accepts(&Value::List(vec![Value::Signed(i64::MAX); 3]), None));
        assert!(
            !site_policy("world::level::tick::crop_growth_speed")
                .result
                .accepts(&Value::Float(0.0), None)
        );
        assert!(
            !site_policy("world::level::bonemeal::crop_bonemeal_increase")
                .result
                .accepts(&Value::Signed(i32::MAX.into()), None)
        );
    }

    #[test]
    fn command_rewrites_obey_the_shared_denylist() {
        for name in DENIED_COMMANDS {
            assert!(command_gate(name, true).is_err());
            assert!(
                command_gate(&format!("/MINECRAFT:{}", name.to_ascii_uppercase()), true).is_err()
            );
            assert!(command_gate(&format!("execute as @s run minecraft:{name}"), true).is_err());
        }
        assert!(command_gate("id", true).is_ok());
        assert!(command_gate("id", false).is_err());
    }

    #[test]
    fn command_gate_scans_large_nested_commands_without_recursive_tails() {
        let prefix = "execute as @s run ".repeat(16);
        let permitted = format!("{prefix}say {}", "a".repeat(65_536 - prefix.len() - 4));
        assert_eq!(permitted.len(), 65_536);
        assert!(command_gate(&permitted, true).is_ok());
        assert!(command_gate(&format!("{prefix}stop"), true).is_err());
        assert!(command_gate(&format!("{prefix}execute run say safe"), true).is_err());
        assert!(command_gate(&format!("{permitted}a"), true).is_err());
    }

    #[test]
    fn command_gate_only_follows_command_bearing_run_arguments() {
        for line in [
            "say please run stop",
            "say I'm allowed to run stop",
            "execute run say please run stop",
            "execute as run run say stop",
            "execute as @e[name=\"please run stop\"] run say safe",
            "execute if score run stop matches 1 run say safe",
            "execute if data storage ns:run \"run stop\" run say safe",
            "execute store result score run stop run say safe",
            "return run say please run stop",
            "return 1",
            "setblock ~ ~ ~ stone",
            "fill ~ ~ ~ ~1 ~1 ~1 stone replace dirt",
            "give @s stone 1",
            "summon pig ~ ~ ~",
        ] {
            assert!(command_gate(line, true).is_ok(), "{line}");
        }
        for line in [
            "execute as run run stop",
            "execute run return run op Evil",
            "return run execute at @s run stop",
            "execute as @e[name=\"run stop\"] run stop",
            "execute if score run stop matches 1 run op Evil",
            "execute if data storage ns:run \"run stop\" run stop",
            "execute as @e[name=\"unterminated] run say safe",
        ] {
            assert!(command_gate(line, true).is_err(), "{line}");
        }
    }

    #[test]
    fn command_gate_refuses_deferred_command_authoring() {
        for line in [
            "schedule function ns:f 1t",
            "setblock ~ ~ ~ minecraft:command_block",
            "fill ~ ~ ~ ~1 ~1 ~1 repeating_command_block[conditional=false]",
            "setblock ~ ~ ~ chain_command_block{Command:'stop'}",
            "setblock ~ ~ ~ oak_sign",
            "fill ~ ~ ~ ~1 ~1 ~1 birch_wall_hanging_sign",
            "setblock ~ ~ ~ chest{Items:[]}",
            "clone ~ ~ ~ ~1 ~1 ~1 ~2 ~2 ~2",
            "data merge block ~ ~ ~ {Command:'stop'}",
            "item replace block ~ ~ ~ container.0 with command_block",
            "execute store result block ~ ~ ~ Command int 1 run say safe",
            "execute store success entity @s Command int 1 run say safe",
            "execute store result storage ns:f Command int 1 run say safe",
            "give @s oak_sign[minecraft:block_entity_data={id:'sign'}]",
            "summon command_block_minecart",
            "summon pig ~ ~ ~ {Passengers:[{id:'command_block_minecart',Command:'stop'}]}",
            "reload",
            "datapack enable file/commands",
            "test run ns:t",
            "compute 1 + 1",
            "pregen start",
        ] {
            assert!(command_gate(line, true).is_err(), "{line}");
            assert!(command_gate(&format!("execute as @s run {line}"), true).is_err(), "{line}");
        }
    }

    #[test]
    fn result_counts_allow_only_the_empty_sentinel_or_a_vanilla_stack() {
        for path in [
            "crafting::crafting_result_count",
            "furnace::smelting_result_count",
        ] {
            let rule = site_policy(path).result;
            for count in [0, 1, 99] {
                assert!(rule.accepts(&Value::Unsigned(count), None));
            }
            for count in [100, 1024, u64::MAX] {
                assert!(!rule.accepts(&Value::Unsigned(count), None));
            }
        }
    }
}
