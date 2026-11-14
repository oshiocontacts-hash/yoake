//! The engine type table of the full-catalog WIT: types WIT cannot express structurally
//! (recursive NBT and item stacks, encapsulated text, live `&mut` draw objects) and how they
//! cross the boundary instead.
//!
//! * `Compound`, `Tag` -> `nbt`, the network binary encoding as bytes;
//! * `ItemStack` -> `item-stack { id, count, components: nbt }`;
//! * `Text` -> `text-json`, vanilla's text component JSON;
//! * `serde_json::Value` -> `json`, JSON text;
//! * `ModBlockEntity` -> the record of its public `ModBlockEntityFields`;
//! * `&mut` draw objects -> a list of typed commands the guest returns ([`EFFECTS`]).
use syn::Type;

/// How an engine type crosses the WIT boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Carrier {
    /// A named alias of a plain WIT type (`type nbt = list<u8>`).
    Alias(&'static str),
    /// A hand-written record (`item-stack`).
    Record(&'static str),
    /// The plain struct of this name, which is what the engine type derefs to.
    Struct(&'static str),
}

/// One row of the type table: the Rust type name, path segments its written path or import must
/// contain (so an unrelated type of the same name stays a handle; empty accepts any), and the
/// WIT carrier.
pub(super) struct TableRow {
    pub rust: &'static str,
    pub origin: &'static [&'static str],
    pub carrier: Carrier,
}

pub(super) const TABLE: &[TableRow] = &[
    TableRow { rust: "Compound", origin: &["nbt", "horizon_world"], carrier: Carrier::Alias("nbt") },
    TableRow { rust: "Tag", origin: &["nbt", "horizon_world"], carrier: Carrier::Alias("nbt") },
    TableRow { rust: "Text", origin: &["text", "horizon_client_core"], carrier: Carrier::Alias("text-json") },
    TableRow { rust: "Value", origin: &["serde_json"], carrier: Carrier::Alias("json") },
    TableRow { rust: "ItemStack", origin: &["items", "horizon_items", "item_stack"], carrier: Carrier::Record("item-stack") },
    TableRow { rust: "ModBlockEntity", origin: &[], carrier: Carrier::Struct("ModBlockEntityFields") },
];

pub(super) const NBT_DOC: &str = "    /// A binary NBT value in Horizon's network encoding: one tag id byte, then that tag's payload\n    /// (no root name), big-endian, strings as Java modified UTF-8. A compound is tag id 10.\n    /// Stand-in for the recursive `Tag` and `Compound`; the guest SDK has a parser and writer.\n    type nbt = list<u8>;\n";
pub(super) const TEXT_DOC: &str = "    /// A text component as vanilla's JSON (`ComponentSerialization`). Stand-in for `Text`.\n    type text-json = string;\n";
pub(super) const JSON_DOC: &str = "    /// A JSON document as text. Stand-in for `serde_json::Value`.\n    type json = string;\n";
pub(super) const ITEM_STACK_DOC: &str = "    /// An item stack: item id, count and the data components (damage, enchantments, custom name,\n    /// bundle and container contents...) as an `nbt` compound, the way Horizon writes them to the\n    /// network and to disk. An empty `components` list means no components.\n    record item-stack {\n        id: string,\n        count: u32,\n        components: nbt,\n    }\n";

/// An engine object a hook receives as `&mut`: the guest cannot hold it, so it returns the
/// commands to apply to it instead.
pub(super) struct Effect {
    pub rust: &'static [&'static str],
    /// The command variant's WIT name.
    pub wit: &'static str,
    /// Set when some of the object's methods have no command (the note goes to coverage.json).
    pub partial: Option<&'static str>,
    /// The WIT declarations of the command variant and its records.
    pub declaration: &'static str,
    /// The names `declaration` declares.
    pub names: &'static [&'static str],
}

const HUD_COMMAND: &str = r#"    /// What a mod draws into the GUI batch (`HudBatch`), applied in order. GUI pixels; colours are
    /// 0xAARRGGBB, except text, whose alpha is its default colour's.
    variant hud-command {
        /// `HudBatch::text_styled`.
        text(hud-text),
        /// `HudBatch::fill`.
        rect(hud-rect),
        /// `HudBatch::fill_gradient`.
        gradient(hud-gradient),
        /// `HudBatch::fill_quad` for a flat colour, `HudBatch::quad` with a sprite.
        quad(hud-quad),
        /// `HudBatch::sprite`, `sprite_scaled`, `sprite_part` and `sprite_region`.
        sprite(hud-sprite),
        /// An item drawn as in an inventory slot.
        item(hud-item),
        /// A texture registered through the native GUI textures, by id.
        texture(hud-texture),
        /// `HudBatch::text_highlight`.
        highlight(hud-highlight),
        /// Starts a run that `end-clip` cuts to the rectangle (`HudBatch::clip_since`).
        begin-clip(hud-bounds),
        end-clip,
        /// Starts a run that `end-scale` scales about a point (`HudBatch::scale_since`).
        begin-scale(hud-scale),
        end-scale,
        /// Starts a run drawn with a blend mode until `end-blend` (`HudBatch::blend_since`).
        begin-blend(hud-blend),
        end-blend,
    }

    record hud-text {
        text: text-json,
        x: f32,
        y: f32,
        color: u32,
        shadow: bool,
    }

    record hud-rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: u32,
    }

    record hud-gradient {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        top: u32,
        bottom: u32,
    }

    record hud-quad {
        /// The four corners in order round the shape.
        corners: list<tuple<f32, f32>>,
        /// A sprite id, or none for a flat colour.
        sprite: option<string>,
        /// UVs of the corners, 0 to 1 over the sprite; none takes the sprite's own corners.
        uv: option<list<tuple<f32, f32>>>,
        color: u32,
    }

    record hud-sprite {
        name: string,
        x: f32,
        y: f32,
        /// The size drawn; none draws the sprite as large as it is.
        size: option<tuple<f32, f32>>,
        color: u32,
        /// Draw only this fraction of the width, from the left.
        fraction: option<f32>,
        /// Draw only this region (x, y, width, height) of a texture of the given size.
        region: option<hud-region>,
    }

    record hud-region {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        texture-width: f32,
        texture-height: f32,
    }

    record hud-item {
        stack: item-stack,
        x: f32,
        y: f32,
        /// Also draw the count and the damage bar.
        decorations: bool,
    }

    record hud-texture {
        id: string,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        color: u32,
    }

    record hud-highlight {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        invert-text: bool,
    }

    record hud-bounds {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
    }

    record hud-scale {
        center-x: f32,
        center-y: f32,
        scale-x: f32,
        scale-y: f32,
    }

    enum hud-blend {
        alpha,
        invert,
        additive,
        vignette,
    }
"#;

const MODEL_COMMAND: &str = r#"    /// What a mod draws with a pose stack (`BlockEntityDraw`: block entities, mobs, players and
    /// items), applied in order. Blocks, +X east, +Y up, +Z south; colours 0xRRGGBB.
    variant model-command {
        /// `BlockEntityDraw::push`.
        push,
        /// `BlockEntityDraw::pop`.
        pop,
        /// `BlockEntityDraw::translate`, in blocks.
        translate(tuple<f32, f32, f32>),
        /// `BlockEntityDraw::rotate-about`.
        rotate-about(model-rotation),
        /// `BlockEntityDraw::scale`.
        scale(tuple<f32, f32, f32>),
        /// Sets `BlockEntityDraw::light` for what is drawn next (16 times sky light in bits 0..8,
        /// 16 times block light in bits 8..16).
        set-light(u32),
        /// `BlockEntityDraw::keep`: the block is drawn the same again until its state changes.
        keep,
        /// `native_draw_quad`.
        quad(model-quad),
        /// `native_draw_cube`.
        cube(model-cube),
        /// `native_draw_block`.
        block(model-block),
        /// `native_draw_model`: a standalone model by id.
        model(model-standalone),
        /// `native_draw_item`.
        item(model-item),
    }

    record model-rotation {
        axis: tuple<f32, f32, f32>,
        degrees: f32,
        pivot: tuple<f32, f32, f32>,
    }

    record model-quad {
        sprite: string,
        /// Four corners in blocks, in a model face's order.
        corners: list<tuple<f32, f32, f32>>,
        /// u0, v0, u1, v1 in texels of 16 per side.
        uv: list<f32>,
        tint: u32,
    }

    record model-cube-face {
        sprite: string,
        uv: option<list<f32>>,
    }

    record model-cube {
        min: tuple<f32, f32, f32>,
        max: tuple<f32, f32, f32>,
        /// Down, up, north, south, west, east; none leaves a face out.
        faces: list<option<model-cube-face>>,
        tint: u32,
    }

    record model-block {
        block: string,
        properties: list<tuple<string, string>>,
    }

    record model-standalone {
        model: string,
        tint: u32,
    }

    enum display-context {
        none,
        third-person-left-hand,
        third-person-right-hand,
        first-person-left-hand,
        first-person-right-hand,
        head,
        gui,
        ground,
        fixed,
        on-shelf,
    }

    record model-item {
        stack: item-stack,
        display: display-context,
        owner-yaw: option<f32>,
        seed: s32,
    }
"#;

const FRAME_COMMAND: &str = r#"    /// What a mod draws over a frame stage (`WorldDrawn`, `HandDrawn`, `WorldPrePost`,
    /// `OverlayDrawn`), applied in order. Raw GPU access (device, encoder, depth) is not offered
    /// to WASM mods: these are whole-frame image operations the host performs on the stage's
    /// colour target.
    variant frame-command {
        /// Fills a rectangle of the target.
        fill-rect(frame-fill),
        /// Blends 0xAARRGGBB over the whole target.
        tint(u32),
        /// Darkens the edges: 0 is none, 1 is a full vignette.
        vignette(f32),
    }

    record frame-fill {
        /// In pixels of the target.
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        /// 0xAARRGGBB.
        color: u32,
    }
"#;

const CORNER_COMMAND: &str = r#"    /// What a mod does to the HUD corner layout (`Corners`).
    variant corner-command {
        /// `Corners::place`: reserve a box in a corner, so the next panels sit past it.
        reserve(corner-box),
    }

    enum hud-corner {
        top-left,
        top-right,
        bottom-left,
        bottom-right,
    }

    record corner-box {
        corner: hud-corner,
        width: f32,
        height: f32,
    }
"#;

pub(super) const EFFECTS: &[Effect] = &[
    Effect {
        rust: &["HudBatch"],
        wit: "hud-command",
        partial: Some("HudBatch: quad_3d, depth_since, mark_carried, raise_carried and the depth/end-portal blends have no command"),
        declaration: HUD_COMMAND,
        names: &[
            "hud-command", "hud-text", "hud-rect", "hud-gradient", "hud-quad", "hud-sprite", "hud-region", "hud-item",
            "hud-texture", "hud-highlight", "hud-bounds", "hud-scale", "hud-blend",
        ],
    },
    Effect {
        rust: &["BlockEntityDraw", "NativeItemDraw"],
        wit: "model-command",
        partial: None,
        declaration: MODEL_COMMAND,
        names: &[
            "model-command", "model-rotation", "model-quad", "model-cube-face", "model-cube", "model-block",
            "model-standalone", "display-context", "model-item",
        ],
    },
    Effect {
        rust: &["WorldDrawn", "HandDrawn", "WorldPrePost", "OverlayDrawn"],
        wit: "frame-command",
        partial: Some("WorldDrawn, HandDrawn, WorldPrePost, OverlayDrawn: wgpu device, encoder and depth access is deliberately not exposed; only whole-frame fills, tints and vignettes"),
        declaration: FRAME_COMMAND,
        names: &["frame-command", "frame-fill"],
    },
    Effect {
        rust: &["Corners"],
        wit: "corner-command",
        partial: Some("Corners: the placed position is not returned to the guest; a panel is reserved, and drawn with hud-command at a position the guest computes"),
        declaration: CORNER_COMMAND,
        names: &["corner-command", "hud-corner", "corner-box"],
    },
];

/// The effect of a `&mut X` hook parameter, if `X` is a live draw object.
pub(super) fn effect_of(ty: &Type) -> Option<&'static Effect> {
    let Type::Reference(reference) = ty else { return None };
    reference.mutability?;
    let Type::Path(path) = &*reference.elem else { return None };
    let name = path.path.segments.last()?.ident.to_string();
    EFFECTS.iter().find(|effect| effect.rust.contains(&name.as_str()))
}
