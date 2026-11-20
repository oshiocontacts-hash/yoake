//! `ai.navigator`: which of the engine's navigators a custom mob uses, with the move control
//! and look control that go with it (FR-F4-MOBS.md section 3.3: "`ground`, `climb`, `swim`,
//! `fly`, `amphibious` map to the existing `NavKind`; move control kind is chosen by the same
//! field").
//!
//! | `navigator` | `NavKind` | move control | look control |
//! |---|---|---|---|
//! | `ground` | `Ground` | `Ground` | default |
//! | `climb` | `WallClimber` (a spider's) | `Ground` | default |
//! | `swim` | `Water { breach: false }` (a dolphin's, without the breaching) | smooth swimming | smooth swimming |
//! | `fly` | `Flying` (a parrot's: floats, opens no doors) | `Flying { max_turn: 10 }` | default |
//! | `amphibious` | `Amphibious` (a drowned's) | smooth swimming (an axolotl's) | smooth swimming |
//!
//! The swimming controls are the vanilla swimmers': they scale `movement_speed` by the
//! controller's own factors (a dolphin's attribute is 1.2, a frog's 1.0), so a `swim` or
//! `amphibious` mob wants a `movement_speed` of that order.
//!
//! Path budget: a path search visits at most [`max_visited_nodes`] nodes, the engine's
//! `follow_range * 16` capped at [`MAX_VISITED_NODES`] for a custom mob, so a large `follow_range`
//! (or many mobs) cannot make one tick expensive. The nav code reads it through that function.
use crate::mobs::{
    Mob,
    control::{LookKind, MoveKind},
    nav::NavKind,
    path::PathType,
};

/// The most nodes one path search of a custom mob may visit (vanilla's own for a follow range of
/// 64 blocks; a follow range of 16 is 256).
pub const MAX_VISITED_NODES: i32 = 1024;

/// What `navigator` selects. An unknown name is the schema's error; the engine treats it as
/// `ground`.
pub fn apply(m: &mut Mob, navigator: &str) {
    match navigator {
        "climb" => m.ai.nav.kind = NavKind::WallClimber,
        "swim" => {
            m.ai.nav.kind = NavKind::Water { breach: false };
            m.ai.mov.kind = swimming();
            m.ai.look.kind = LookKind::SmoothSwimming(10.0);
            // Water costs a swimmer nothing, so its random targets in water pass the checks.
            m.set_malus(PathType::Water, 0.0);
        }
        "fly" => {
            m.ai.nav.kind = NavKind::Flying;
            m.ai.mov.kind = MoveKind::Flying {
                max_turn: 10.0,
                hovers: false,
            };
            m.ai.nav.can_open_doors = false;
            m.ai.nav.can_float = true;
        }
        "amphibious" => {
            m.ai.nav.kind = NavKind::Amphibious {
                prefers_shallow: false,
            };
            m.ai.mov.kind = MoveKind::SmoothSwimming {
                max_turn_x: 85.0,
                max_turn_y: 10.0,
                in_water: 0.1,
                outside: 0.5,
                gravity: false,
            };
            m.ai.look.kind = LookKind::SmoothSwimming(20.0);
            m.set_malus(PathType::Water, 0.0);
        }
        // `ground`: the engine's default navigation and move control.
        _ => {}
    }
    m.update_max_visited_nodes();
}

/// A dolphin's `SmoothSwimmingMoveControl(85, 10, 0.02, 0.1, true)`.
fn swimming() -> MoveKind {
    MoveKind::SmoothSwimming {
        max_turn_x: 85.0,
        max_turn_y: 10.0,
        in_water: 0.02,
        outside: 0.1,
        gravity: true,
    }
}

/// The node budget of one path search of `m`: `nodes` (the engine's own number, times its
/// multiplier) for a vanilla mob, at most [`MAX_VISITED_NODES`] for a custom one.
pub fn max_visited_nodes(m: &Mob, nodes: i32) -> i32 {
    if m.kind.custom.is_some() {
        nodes.min(MAX_VISITED_NODES)
    } else {
        nodes
    }
}
