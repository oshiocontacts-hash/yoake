use crate::Value;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rule {
    Unrestricted,
    Finite,
    Number(f64, f64),
    Vector(f64, f64),
    BlockState,
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
            Self::Number(minimum, maximum) => {
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
        }
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
    use Rule::{BlockState, Finite, Number, Vector};

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
    let tokens: Vec<_> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("denied: empty command".into());
    }
    let mut depth = 0;
    for (index, token) in tokens.iter().enumerate() {
        if index != 0 {
            if !tokens[index - 1].eq_ignore_ascii_case("run") {
                continue;
            }
            depth += 1;
            if depth > 16 {
                return Err("denied: commands nested too deep".into());
            }
        }
        let name = token.trim_start_matches('/').to_ascii_lowercase();
        let name = name.strip_prefix("minecraft:").unwrap_or(&name);
        if DENIED_COMMANDS.contains(&name) {
            return Err(format!(
                "denied: /{name} is not available to WebAssembly mods"
            ));
        }
    }
    Ok(())
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
