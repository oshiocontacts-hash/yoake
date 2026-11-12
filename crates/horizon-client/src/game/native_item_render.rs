use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use glam::{Mat4, Vec3};
use horizon_assets::item_model::DisplayContext;
use horizon_client::item_render::ItemRenderState;
use horizon_client::mesher::Vertex;
use horizon_client::mod_api::Components;

use super::{Game, native_block_entity_draw::BlockEntityDraw};

pub type NativeItemRenderStack = horizon_client::items::ItemStack;
pub type NativeItemDraw = BlockEntityDraw;

#[derive(Clone, Copy, Debug)]
pub struct NativeItemRenderContext<'a> {
    pub components: &'a Components,
    pub partial_tick: f32,
    pub game_time: i64,
    pub light: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeItemContext {
    FirstPersonRight,
    FirstPersonLeft,
    ThirdPersonRight,
    ThirdPersonLeft,
    Gui,
    Ground,
    Fixed,
    Head,
    ItemDisplay,
    OnShelf,
    None,
}

impl NativeItemContext {
    fn from_display(display: DisplayContext, item_display: bool) -> Self {
        if item_display {
            return Self::ItemDisplay;
        }
        match display {
            DisplayContext::FirstPersonRightHand => Self::FirstPersonRight,
            DisplayContext::FirstPersonLeftHand => Self::FirstPersonLeft,
            DisplayContext::ThirdPersonRightHand => Self::ThirdPersonRight,
            DisplayContext::ThirdPersonLeftHand => Self::ThirdPersonLeft,
            DisplayContext::Gui => Self::Gui,
            DisplayContext::Ground => Self::Ground,
            DisplayContext::Fixed => Self::Fixed,
            DisplayContext::Head => Self::Head,
            DisplayContext::OnShelf => Self::OnShelf,
            DisplayContext::None => Self::None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NativeItemHolder {
    pub entity_id: Option<i32>,
    pub is_local_player: bool,
    pub offhand: bool,
    pub using_item: bool,
    pub use_ticks: i32,
    pub swing_progress: Option<f32>,
    pub partial_tick: f32,
}

fn item_holder(
    owner: &horizon_assets::item_model::OwnerState,
    entity_id: Option<i32>,
    swing_progress: Option<f32>,
    offhand: bool,
    partial_tick: f32,
) -> NativeItemHolder {
    NativeItemHolder {
        entity_id,
        is_local_player: owner.is_local_player,
        offhand,
        using_item: owner.using_this_item,
        use_ticks: owner.use_item_remaining_ticks,
        swing_progress,
        partial_tick,
    }
}

#[derive(Default)]
pub(super) struct NativeItemClaims {
    listened: Cell<bool>,
    claims: RefCell<HashMap<String, bool>>,
    dispatching: Cell<bool>,
}

impl NativeItemClaims {
    fn refresh(&self, listened: bool, reload: bool) {
        if reload || self.listened.replace(listened) != listened {
            self.claims.borrow_mut().clear();
        }
        self.listened.set(listened);
    }

    fn claimed(&self, item: &str, query: impl FnOnce(&str) -> bool) -> bool {
        if !self.listened.get() || self.dispatching.get() {
            return false;
        }
        if let Some(&claimed) = self.claims.borrow().get(item) {
            return claimed;
        }
        let claimed = query(item);
        self.claims.borrow_mut().insert(item.to_owned(), claimed);
        claimed
    }
}

struct DispatchGuard<'a>(&'a Cell<bool>);
impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

pub(super) fn keeps_vanilla(drawn: Option<bool>) -> bool {
    !drawn.unwrap_or(false)
}

fn dispatch_item(
    context: NativeItemContext,
    render: impl FnOnce(NativeItemContext, &mut bool),
) -> bool {
    let mut drawn = false;
    render(context, &mut drawn);
    drawn
}

impl Game<'_> {
    pub(super) fn refresh_native_item_render(&self, reload: bool) {
        if horizon_client::mod_api::mods().is_empty() {
            return;
        }
        let mut listened = false;
        self.native_item_render_listened(&mut listened);
        self.native_item_claims.refresh(listened, reload);
    }

    pub(super) fn native_item_render_claimed(&self, item: &str) -> bool {
        self.native_item_claims.claimed(item, |item| {
            let mut claimed = false;
            self.native_item_render_claims(item, &mut claimed);
            claimed
        })
    }

    pub(super) fn native_item_geometry(
        &self,
        state: &ItemRenderState,
        pose: Mat4,
        light: u32,
        lights: (Vec3, Vec3),
    ) -> Option<(bool, Vec<Vertex>)> {
        let stack = state.native_stack.as_ref()?;
        if self.native_item_claims.dispatching.replace(true) {
            return None;
        }
        let _guard = DispatchGuard(&self.native_item_claims.dispatching);
        let partial = self.clock.fraction() as f32;
        let holder = state.native_owner.as_ref().map(|owner| {
            item_holder(
                owner,
                state.native_entity_id,
                state.native_swing_progress,
                state.native_offhand,
                partial,
            )
        });
        let block = state.native_owner.as_ref().map_or([0; 3], |owner| {
            owner.position.map(|coordinate| coordinate.floor() as i32)
        });
        let pose = pose * state.layers.first().map_or(Mat4::IDENTITY, |layer| layer.matrix);
        let stack_view = horizon_client::item_render::stack_view(stack);
        let components = self.pack.item_renderer.models.components(&stack_view);
        let presentation = NativeItemRenderContext {
            components: &components,
            partial_tick: partial,
            game_time: self.native_item_ticks,
            light,
        };
        let mut draw = NativeItemDraw::posed(
            partial,
            self.native_item_ticks,
            light,
            block,
            pose,
            lights,
            Vec::new(),
        );
        let drawn = dispatch_item(
            NativeItemContext::from_display(state.display, state.native_item_display),
            |context, drawn| {
                self.native_item_render(stack, context, holder, &presentation, &mut draw, drawn)
            },
        );
        Some((drawn, draw.into_vertices()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holder_preserves_owner_identity_and_interpolated_swing() {
        let owner = horizon_assets::item_model::OwnerState {
            is_local_player: true,
            using_this_item: true,
            use_item_remaining_ticks: 9,
            ..Default::default()
        };
        let holder = item_holder(&owner, Some(47), Some(0.375), true, 0.5);
        assert_eq!(holder.entity_id, Some(47));
        assert_eq!(holder.swing_progress, Some(0.375));
        assert!(holder.is_local_player);
        assert!(holder.offhand);
        assert!(holder.using_item);
        assert_eq!(holder.use_ticks, 9);
        assert_eq!(holder.partial_tick, 0.5);
        let unavailable = item_holder(&owner, None, None, false, 0.0);
        assert_eq!(unavailable.entity_id, None);
        assert_eq!(unavailable.swing_progress, None);
    }

    #[test]
    fn claims_are_empty_without_listeners_and_rebuilt_on_reload() {
        let cache = NativeItemClaims::default();
        let calls = Cell::new(0);
        let query = |_: &str| {
            calls.set(calls.get() + 1);
            true
        };
        assert!(!cache.claimed("example:item", query));
        assert!(cache.claims.borrow().is_empty());
        cache.refresh(true, false);
        assert!(cache.claimed("example:item", query));
        assert!(cache.claimed("example:item", query));
        assert_eq!(calls.get(), 1);
        assert!(!cache.claimed("example:unclaimed", |_| false));
        assert!(!cache.claimed("example:unclaimed", |_| {
            panic!("negative claim was not cached")
        }));
        cache.refresh(true, true);
        assert!(cache.claims.borrow().is_empty());
        assert!(cache.claimed("example:item", query));
        assert_eq!(calls.get(), 2);
        cache.refresh(false, false);
        assert!(cache.claims.borrow().is_empty());
    }

    #[test]
    fn false_drawn_keeps_vanilla_model() {
        assert!(keeps_vanilla(None));
        assert!(keeps_vanilla(Some(false)));
        assert!(!keeps_vanilla(Some(true)));
    }

    #[test]
    fn display_contexts_match_item_callers() {
        let contexts = [
            (
                "first-person right",
                DisplayContext::FirstPersonRightHand,
                NativeItemContext::FirstPersonRight,
            ),
            (
                "first-person left",
                DisplayContext::FirstPersonLeftHand,
                NativeItemContext::FirstPersonLeft,
            ),
            (
                "third-person right",
                DisplayContext::ThirdPersonRightHand,
                NativeItemContext::ThirdPersonRight,
            ),
            (
                "third-person left",
                DisplayContext::ThirdPersonLeftHand,
                NativeItemContext::ThirdPersonLeft,
            ),
            ("GUI", DisplayContext::Gui, NativeItemContext::Gui),
            ("dropped item", DisplayContext::Ground, NativeItemContext::Ground),
            (
                "item frame and block display",
                DisplayContext::Fixed,
                NativeItemContext::Fixed,
            ),
            ("head equipment", DisplayContext::Head, NativeItemContext::Head),
            ("shelf", DisplayContext::OnShelf, NativeItemContext::OnShelf),
            ("unspecified", DisplayContext::None, NativeItemContext::None),
        ];
        for (caller, display, expected) in contexts {
            assert_eq!(NativeItemContext::from_display(display, false), expected, "{caller}");
            assert_eq!(
                NativeItemContext::from_display(display, true),
                NativeItemContext::ItemDisplay,
                "item_display: {caller}",
            );
        }
    }

    #[test]
    fn each_context_dispatches_once() {
        let contexts = [
            NativeItemContext::FirstPersonRight,
            NativeItemContext::FirstPersonLeft,
            NativeItemContext::ThirdPersonRight,
            NativeItemContext::ThirdPersonLeft,
            NativeItemContext::Gui,
            NativeItemContext::Ground,
            NativeItemContext::Fixed,
            NativeItemContext::Head,
            NativeItemContext::ItemDisplay,
            NativeItemContext::OnShelf,
            NativeItemContext::None,
        ];
        for context in contexts {
            let calls = Cell::new(0);
            let drawn = dispatch_item(context, |actual, drawn| {
                assert_eq!(actual, context);
                calls.set(calls.get() + 1);
                *drawn = true;
            });
            assert!(drawn);
            assert_eq!(calls.get(), 1);
        }
    }
}
