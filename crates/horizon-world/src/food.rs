//! Hunger, saturation, exhaustion and server-side natural regeneration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difficulty {
    Peaceful,
    Easy,
    Normal,
    Hard,
}
#[yoake_state::yoake_state(read_only)]
#[derive(Clone, Copy, Debug)]
pub struct FoodData {
    #[yoake_state(read_only)]
    pub food: i32,
    #[yoake_state(read_only)]
    pub saturation: f32,
    #[yoake_state(read_only)]
    pub exhaustion: f32,
    #[yoake_state(read_only)]
    pub timer: i32,
}
impl Default for FoodData {
    fn default() -> Self {
        Self {
            food: 20,
            saturation: 5.0,
            exhaustion: 0.0,
            timer: 0,
        }
    }
}
fn min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() | b.to_bits())
    } else {
        a.min(b)
    }
}
fn max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else {
        a.max(b)
    }
}
fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    if v < lo { lo } else { min(v, hi) }
}
#[yoake_inject_macro::yoake_inject(
    id = "world::food::exhaustion_amount",
    side = "server",
    allow = "return"
)]
fn exhaustion_amount(value: f32) -> f32 {
    value
}

#[cfg(test)]
mod utility_inject_tests {
    use super::*;
    use std::sync::Arc;
    use yoake_inject_rt::{CallbackError, Guest, Mod, RETURN, Registry, Side, Subscription, Value};

    #[test]
    fn exhaustion_handler_changes_the_added_amount_with_vanilla_fallback() {
        struct Replace(f64);
        impl Guest for Replace {
            fn returned(&self, _: &str, _: &Value) -> Result<Option<Value>, CallbackError> {
                Ok(Some(Value::Float(self.0)))
            }
        }
        let _side = yoake_inject_rt::enter_side(Side::Server);
        let add = || {
            let mut food = FoodData::default();
            food.add_exhaustion(2.0);
            food.exhaustion
        };
        assert_eq!(add(), 2.0);
        for (replacement, expected) in [(0.5, 0.5), (-2.0, 0.0), (80.0, 40.0), (f64::NAN, 2.0)] {
            let site = __yoake_inject_site_exhaustion_amount();
            let mut registry = Registry::new([site]).unwrap();
            let prepared = registry
                .prepare(
                    [Subscription {
                        site_id: site.id(),
                        side: Side::Server,
                        points: RETURN,
                        module: Arc::new(Mod::new(
                            "exhaustion-test",
                            Arc::new(Replace(replacement)),
                        )),
                    }],
                    |_, _, _| true,
                )
                .unwrap();
            registry.publish(prepared).unwrap();
            assert_eq!(add(), expected);
            drop(registry);
            assert_eq!(add(), 2.0);
        }
    }
}

impl FoodData {
    pub fn eat(&mut self, nutrition: i32, saturation_modifier: f32) {
        self.eat_properties(nutrition, nutrition as f32 * saturation_modifier * 2.0);
    }
    #[yoake_inject_macro::yoake_inject(id = "world::food::eat_properties", side = "server")]
    pub fn eat_properties(&mut self, nutrition: i32, saturation: f32) {
        self.food = nutrition.wrapping_add(self.food).clamp(0, 20);
        self.saturation = clamp(saturation + self.saturation, 0.0, self.food as f32);
    }
    #[yoake_inject_macro::yoake_inject(id = "world::food::add_exhaustion", side = "server")]
    pub fn add_exhaustion(&mut self, value: f32) {
        self.exhaustion = min(self.exhaustion + exhaustion_amount(value), 40.0);
    }
    pub fn has_enough_food(&self) -> bool {
        self.food as f32 > 6.0
    }
    pub fn needs_food(&self) -> bool {
        self.food < 20
    }
    pub fn tick<W: FoodWorld>(&mut self, world: &mut W) -> Result<(), W::Error> {
        let level = world.level(self)?;
        let difficulty = world.difficulty(level, self)?;
        if self.exhaustion > 4.0 {
            self.exhaustion -= 4.0;
            if self.saturation > 0.0 {
                self.saturation = max(self.saturation - 1.0, 0.0);
            } else if difficulty != Difficulty::Peaceful {
                self.food = self.food.wrapping_sub(1).max(0);
            }
        }
        let regenerate = world.natural_regeneration(level, self)?;
        if regenerate && self.saturation > 0.0 && world.is_hurt(self)? && self.food >= 20 {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 10 {
                let amount = min(self.saturation, 6.0);
                world.heal(self, amount / 6.0)?;
                self.add_exhaustion(amount);
                self.timer = 0;
            }
        } else if regenerate && self.food >= 18 && world.is_hurt(self)? {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 80 {
                world.heal(self, 1.0)?;
                self.add_exhaustion(6.0);
                self.timer = 0;
            }
        } else if self.food <= 0 {
            self.timer = self.timer.wrapping_add(1);
            if self.timer >= 80 {
                if world.health(self)? > 10.0
                    || difficulty == Difficulty::Hard
                    || (world.health(self)? > 1.0 && difficulty == Difficulty::Normal)
                {
                    world.hurt_starvation(level, self, 1.0)?;
                }
                self.timer = 0;
            }
        } else {
            self.timer = 0;
        }
        Ok(())
    }
}
/// World hooks preserve short-circuit query order. Callbacks can mutate food
/// state; errors retain earlier writes and effects. The level is captured once.
pub trait FoodWorld {
    type Error;
    type Level: Copy;
    fn level(&mut self, food: &mut FoodData) -> Result<Self::Level, Self::Error>;
    fn difficulty(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
    ) -> Result<Difficulty, Self::Error>;
    fn natural_regeneration(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
    ) -> Result<bool, Self::Error>;
    fn is_hurt(&mut self, food: &mut FoodData) -> Result<bool, Self::Error>;
    fn heal(&mut self, food: &mut FoodData, amount: f32) -> Result<(), Self::Error>;
    fn health(&mut self, food: &mut FoodData) -> Result<f32, Self::Error>;
    /// Resolve the player's starvation damage source and dispatch hurtServer on
    /// the captured level; the native boolean hurt result is ignored.
    fn hurt_starvation(
        &mut self,
        level: Self::Level,
        food: &mut FoodData,
        amount: f32,
    ) -> Result<(), Self::Error>;
}
