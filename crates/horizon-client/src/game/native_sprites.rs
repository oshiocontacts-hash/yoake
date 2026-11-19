//! Runtime item and block sprite overrides for WASM and native client mods.
use super::Game;
use horizon_assets::item_model::{DisplayContext, NoEnv, StackView};
use horizon_client::font::Patch;
use horizon_client::hud::Rect;
use horizon_client::item_render::{ItemRenderer, LayerGeometry, sprite_name};
use horizon_client::mipmap::Settings;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone)]
struct ItemOverride {
    owner: String,
    rgba: Vec<u8>,
    size: [u32; 2],
    sprite_keys: Vec<String>,
}

#[derive(Clone)]
struct AtlasOriginal {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Clone)]
struct BlockOverride {
    owner: String,
    rgba: Vec<u8>,
    size: [u32; 2],
}

#[derive(Default)]
pub(super) struct SpriteOverrideStore {
    item_overrides: BTreeMap<String, ItemOverride>,
    item_originals: HashMap<String, AtlasOriginal>,
    block_overrides: BTreeMap<String, BlockOverride>,
}

impl SpriteOverrideStore {
    pub(super) fn is_empty(&self) -> bool {
        self.item_overrides.is_empty() && self.block_overrides.is_empty()
    }

    #[cfg(test)]
    pub(super) fn item_owner(&self, item_id: &str) -> Option<&str> {
        self.item_overrides.get(item_id).map(|entry| entry.owner.as_str())
    }

    #[cfg(test)]
    pub(super) fn block_owner(&self, texture_id: &str) -> Option<&str> {
        self.block_overrides.get(texture_id).map(|entry| entry.owner.as_str())
    }

    pub(super) fn set_item<'a, F>(
        &mut self,
        owner: &str,
        item_id: &str,
        rgba: &[u8],
        size: [u32; 2],
        atlas_width: u32,
        atlas_height: u32,
        atlas_pixels: &mut [u8],
        sprites: &HashMap<String, Rect>,
        resolve_sprites: F,
        patches: &mut Vec<Patch>,
    ) -> Result<String, String>
    where
        F: FnOnce(&str) -> Vec<String>,
    {
        let canonical = canonical_id(item_id)?;
        let expected_atlas = expected_rgba_len([atlas_width, atlas_height])?;
        if atlas_pixels.len() < expected_atlas {
            return Err("shared atlas pixel buffer is smaller than atlas dimensions".into());
        }
        let sprite_keys = resolve_sprites(&canonical);
        if sprite_keys.is_empty() {
            return Err(format!("no atlas sprite found for item `{canonical}`"));
        }
        let mut targets = Vec::with_capacity(sprite_keys.len());
        for key in &sprite_keys {
            let rect = sprites
                .get(key)
                .copied()
                .ok_or_else(|| format!("missing atlas rect for sprite `{key}`"))?;
            validate_rgba(rgba, size, [rect.w, rect.h])?;
            let (x, y) = rect_origin(rect, atlas_width, atlas_height)?;
            targets.push((key.clone(), x, y, rect.w, rect.h));
        }
        for (key, x, y, width, height) in targets {
            self.snapshot_and_write_atlas(
                &key,
                x,
                y,
                width,
                height,
                rgba,
                atlas_width,
                atlas_pixels,
                patches,
            );
        }
        self.item_overrides.insert(
            canonical.clone(),
            ItemOverride {
                owner: owner.to_owned(),
                rgba: rgba.to_vec(),
                size,
                sprite_keys,
            },
        );
        Ok(canonical)
    }

    pub(super) fn reset_item(
        &mut self,
        item_id: &str,
        atlas_width: u32,
        atlas_pixels: &mut [u8],
        sprites: &HashMap<String, Rect>,
        patches: &mut Vec<Patch>,
    ) -> Result<Option<String>, String> {
        let canonical = canonical_id(item_id)?;
        let Some(removed) = self.item_overrides.remove(&canonical) else {
            return Ok(None);
        };
        for key in &removed.sprite_keys {
            if let Some(other) = self
                .item_overrides
                .values()
                .find(|entry| entry.sprite_keys.iter().any(|candidate| candidate == key))
            {
                if let Some(rect) = sprites.get(key).copied()
                    && [rect.w, rect.h] == other.size
                {
                    let (x, y) = rect_origin(rect, atlas_width, atlas_pixels_height(atlas_pixels, atlas_width))?;
                    write_rect_pixels(atlas_pixels, atlas_width, x, y, rect.w, rect.h, &other.rgba);
                    patches.push(Patch {
                        x,
                        y,
                        width: rect.w,
                        height: rect.h,
                        rgba: other.rgba.clone(),
                    });
                }
            } else if let Some(original) = self.item_originals.remove(key) {
                write_rect_pixels(
                    atlas_pixels,
                    atlas_width,
                    original.x,
                    original.y,
                    original.width,
                    original.height,
                    &original.rgba,
                );
                patches.push(Patch {
                    x: original.x,
                    y: original.y,
                    width: original.width,
                    height: original.height,
                    rgba: original.rgba,
                });
            }
        }
        Ok(Some(canonical))
    }

    pub(super) fn set_block(
        &mut self,
        owner: &str,
        texture_id: &str,
        rgba: &[u8],
        size: [u32; 2],
        sprite_size: u32,
        sprite_layers: &BTreeMap<String, u32>,
    ) -> Result<(String, u32), String> {
        let canonical = canonical_id(texture_id)?;
        let layer = *sprite_layers
            .get(&canonical)
            .ok_or_else(|| format!("unknown block texture `{canonical}`"))?;
        validate_rgba(rgba, size, [sprite_size, sprite_size])?;
        self.block_overrides.insert(
            canonical.clone(),
            BlockOverride {
                owner: owner.to_owned(),
                rgba: rgba.to_vec(),
                size,
            },
        );
        Ok((canonical, layer))
    }

    pub(super) fn reset_block<'a>(
        &mut self,
        texture_id: &str,
        sprite_size: u32,
        sprite_layers: &BTreeMap<String, u32>,
        pack_pixels: &'a [u8],
    ) -> Result<Option<(String, u32, Vec<u8>, bool)>, String> {
        let canonical = canonical_id(texture_id)?;
        let Some(_removed) = self.block_overrides.remove(&canonical) else {
            return Ok(None);
        };
        let Some(&layer) = sprite_layers.get(&canonical) else {
            return Ok(Some((canonical, 0, Vec::new(), false)));
        };
        if let Some((_, remaining)) = self.block_overrides.iter().find(|(id, entry)| {
            sprite_layers.get(*id) == Some(&layer) && entry.size == [sprite_size, sprite_size]
        }) {
            return Ok(Some((canonical, layer, remaining.rgba.clone(), true)));
        }
        let pixels = layer_slice(pack_pixels, layer, sprite_size)?.to_vec();
        Ok(Some((canonical, layer, pixels, false)))
    }

    /// The current RGBA8 pixels of an item's sprite as `(width, height, rgba)`, read from the CPU copy of
    /// the shared atlas (so a mod override set by `set_item` is included). Nothing is allocated or kept.
    pub(super) fn item_pixels<F>(
        item_id: &str,
        atlas_width: u32,
        atlas_height: u32,
        atlas_pixels: &[u8],
        sprites: &HashMap<String, Rect>,
        resolve_sprites: F,
    ) -> Result<(u32, u32, Vec<u8>), String>
    where
        F: FnOnce(&str) -> Vec<String>,
    {
        let canonical = canonical_id(item_id)?;
        let key = resolve_sprites(&canonical)
            .into_iter()
            .next()
            .ok_or_else(|| format!("no atlas sprite found for item `{canonical}`"))?;
        let rect = sprites
            .get(&key)
            .copied()
            .ok_or_else(|| format!("missing atlas rect for sprite `{key}`"))?;
        let (x, y) = rect_origin(rect, atlas_width, atlas_height)?;
        let needed = expected_rgba_len([atlas_width, atlas_height])?;
        if atlas_pixels.len() < needed {
            return Err("atlas pixels are not available".into());
        }
        Ok((rect.w, rect.h, read_rect_pixels(atlas_pixels, atlas_width, x, y, rect.w, rect.h)))
    }

    /// The current RGBA8 pixels of a block texture layer as `(size, size, rgba)`: a mod override on that
    /// layer (this id's own first, else one sharing the layer), otherwise the pack's first frame.
    pub(super) fn block_pixels(
        &self,
        texture_id: &str,
        sprite_size: u32,
        sprite_layers: &BTreeMap<String, u32>,
        pack_pixels: &[u8],
    ) -> Result<(u32, u32, Vec<u8>), String> {
        let canonical = canonical_id(texture_id)?;
        let layer = *sprite_layers
            .get(&canonical)
            .ok_or_else(|| format!("unknown block texture `{canonical}`"))?;
        let expected = [sprite_size, sprite_size];
        let own = self.block_overrides.get(&canonical).filter(|entry| entry.size == expected);
        let shared = || {
            self.block_overrides
                .iter()
                .find(|(id, entry)| sprite_layers.get(*id) == Some(&layer) && entry.size == expected)
                .map(|(_, entry)| entry)
        };
        if let Some(entry) = own.or_else(shared) {
            return Ok((sprite_size, sprite_size, entry.rgba.clone()));
        }
        let pixels = layer_slice(pack_pixels, layer, sprite_size)?.to_vec();
        Ok((sprite_size, sprite_size, pixels))
    }

    #[allow(clippy::too_many_arguments)]
    fn snapshot_and_write_atlas(
        &mut self,
        key: &str,
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
        atlas_width: u32,
        atlas_pixels: &mut [u8],
        patches: &mut Vec<Patch>,
    ) {
        self.item_originals
            .entry(key.to_owned())
            .or_insert_with(|| AtlasOriginal {
                x,
                y,
                width,
                height,
                rgba: read_rect_pixels(atlas_pixels, atlas_width, x, y, width, height),
            });
        write_rect_pixels(atlas_pixels, atlas_width, x, y, width, height, rgba);
        patches.push(Patch {
            x,
            y,
            width,
            height,
            rgba: rgba.to_vec(),
        });
    }

    pub(super) fn reload_items<F>(
        &mut self,
        atlas_width: u32,
        atlas_height: u32,
        atlas_pixels: &mut [u8],
        sprites: &HashMap<String, Rect>,
        mut resolve_sprites: F,
        patches: &mut Vec<Patch>,
    ) where
        F: FnMut(&str) -> Vec<String>,
    {
        self.item_originals.clear();
        if self.item_overrides.is_empty() {
            return;
        }
        let keys: Vec<String> = self.item_overrides.keys().cloned().collect();
        for item_id in keys {
            let Some(entry) = self.item_overrides.get(&item_id).cloned() else {
                continue;
            };
            let resolved = resolve_sprites(&item_id);
            if resolved.is_empty() {
                log::warn!("Yoake: skipping item sprite override `{item_id}` after reload (not found)");
                continue;
            }
            let mut matched = Vec::with_capacity(resolved.len());
            let mut ok = true;
            for sprite_key in &resolved {
                let Some(rect) = sprites.get(sprite_key).copied() else {
                    ok = false;
                    break;
                };
                if [rect.w, rect.h] != entry.size {
                    log::warn!(
                        "Yoake: skipping item sprite override `{item_id}` after reload (expected {}x{}, pack has {}x{})",
                        entry.size[0], entry.size[1], rect.w, rect.h
                    );
                    ok = false;
                    break;
                }
                let Ok((x, y)) = rect_origin(rect, atlas_width, atlas_height) else {
                    ok = false;
                    break;
                };
                matched.push((sprite_key.clone(), x, y, rect.w, rect.h));
            }
            if !ok {
                continue;
            }
            for (sprite_key, x, y, width, height) in matched {
                self.snapshot_and_write_atlas(
                    &sprite_key, x, y, width, height, &entry.rgba, atlas_width, atlas_pixels, patches,
                );
            }
            if let Some(stored) = self.item_overrides.get_mut(&item_id) {
                stored.sprite_keys = resolved;
            }
        }
    }

    pub(super) fn collect_block_reapply(
        &self,
        sprite_size: u32,
        sprite_layers: &BTreeMap<String, u32>,
    ) -> Vec<(u32, Vec<u8>)> {
        let mut updates = Vec::new();
        for (texture_id, entry) in &self.block_overrides {
            let Some(&layer) = sprite_layers.get(texture_id) else {
                log::warn!("Yoake: skipping block sprite override `{texture_id}` after reload (not found)");
                continue;
            };
            if entry.size != [sprite_size, sprite_size] {
                log::warn!(
                    "Yoake: skipping block sprite override `{texture_id}` after reload (expected {}x{}, pack has {sprite_size}x{sprite_size})",
                    entry.size[0], entry.size[1]
                );
                continue;
            }
            updates.push((layer, entry.rgba.clone()));
        }
        updates
    }
}

pub(super) fn canonical_id(raw: &str) -> Result<String, String> {
    if raw.is_empty()
        || raw.chars().any(|ch| ch.is_whitespace() || ch.is_control())
        || raw.contains("..")
    {
        return Err(format!("invalid identifier `{raw}`"));
    }
    if let Some((ns, path)) = raw.split_once(':') {
        if ns.is_empty() || path.is_empty() || ns.contains('/') || ns.contains(':') || path.contains(':') {
            return Err(format!("invalid identifier `{raw}`"));
        }
        Ok(raw.to_owned())
    } else {
        Ok(format!("minecraft:{raw}"))
    }
}

fn expected_rgba_len(size: [u32; 2]) -> Result<usize, String> {
    if size[0] == 0 || size[1] == 0 {
        return Err(format!("sprite dimensions must be non-zero, got {}x{}", size[0], size[1]));
    }
    (size[0] as usize)
        .checked_mul(size[1] as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| "sprite dimensions overflow".to_owned())
}

pub(super) fn validate_rgba(rgba: &[u8], size: [u32; 2], expected: [u32; 2]) -> Result<(), String> {
    if size[0] == 0 || size[1] == 0 {
        return Err(format!("sprite dimensions must be non-zero, got {}x{}", size[0], size[1]));
    }
    if size != expected {
        return Err(format!(
            "sprite size mismatch: expected {}x{}, got {}x{}",
            expected[0], expected[1], size[0], size[1]
        ));
    }
    let bytes = expected_rgba_len(size)?;
    if rgba.len() != bytes {
        return Err(format!("expected {bytes} RGBA bytes for {}x{}, got {}", size[0], size[1], rgba.len()));
    }
    Ok(())
}

fn atlas_pixels_height(pixels: &[u8], width: u32) -> u32 {
    let row_bytes = (width as usize).saturating_mul(4);
    if row_bytes == 0 { 0 } else { (pixels.len() / row_bytes) as u32 }
}

fn rect_origin(rect: Rect, atlas_width: u32, atlas_height: u32) -> Result<(u32, u32), String> {
    let x = (rect.u0 * atlas_width as f32).round() as u32;
    let y = (rect.v0 * atlas_height as f32).round() as u32;
    if x.checked_add(rect.w).is_none_or(|end| end > atlas_width)
        || y.checked_add(rect.h).is_none_or(|end| end > atlas_height)
    {
        return Err("sprite atlas rect out of bounds".into());
    }
    Ok((x, y))
}

fn read_rect_pixels(pixels: &[u8], atlas_width: u32, x: u32, y: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    let row_stride = atlas_width as usize * 4;
    let copy_len = w as usize * 4;
    for row in 0..h as usize {
        let start = (y as usize + row) * row_stride + x as usize * 4;
        out.extend_from_slice(&pixels[start..start + copy_len]);
    }
    out
}

fn write_rect_pixels(pixels: &mut [u8], atlas_width: u32, x: u32, y: u32, w: u32, h: u32, rgba: &[u8]) {
    let row_stride = atlas_width as usize * 4;
    let copy_len = w as usize * 4;
    for row in 0..h as usize {
        let dst = (y as usize + row) * row_stride + x as usize * 4;
        let src = row * copy_len;
        pixels[dst..dst + copy_len].copy_from_slice(&rgba[src..src + copy_len]);
    }
}

fn layer_slice(pack_pixels: &[u8], layer: u32, sprite_size: u32) -> Result<&[u8], String> {
    let layer_bytes = expected_rgba_len([sprite_size, sprite_size])?;
    let start = (layer as usize)
        .checked_mul(layer_bytes)
        .ok_or_else(|| "block layer offset overflow".to_owned())?;
    let end = start
        .checked_add(layer_bytes)
        .ok_or_else(|| "block layer offset overflow".to_owned())?;
    pack_pixels
        .get(start..end)
        .ok_or_else(|| format!("missing original pixels for block layer {layer}"))
}

fn resolve_item_sprites(item_renderer: &ItemRenderer, sprites: &HashMap<String, Rect>, item_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    if item_renderer.models.definitions.contains_key(item_id) {
        let state = item_renderer.resolve(&StackView::new(item_id, 1), DisplayContext::Gui, None, None, 0, &NoEnv);
        for layer in &state.layers {
            if let LayerGeometry::Quads(baked) = &layer.geometry {
                for q in &baked.quads {
                    let key = sprite_name(&q.material.sprite);
                    if key != "missingno" && key != "misc/missingno" && key != "horizon/missingno" && sprites.contains_key(key) && !out.iter().any(|existing| existing == key) {
                        out.push(key.to_owned());
                    }
                }
            }
        }
    }
    if out.is_empty() {
        let candidates = if let Some(path) = item_id.strip_prefix("minecraft:") {
            [format!("item/{path}"), format!("block/{path}")]
        } else if let Some((ns, path)) = item_id.split_once(':') {
            [format!("{ns}:item/{path}"), format!("{ns}:block/{path}")]
        } else {
            return out;
        };
        for key in candidates {
            if sprites.contains_key(&key) {
                out.push(key);
                break;
            }
        }
    }
    out
}

fn caller_owner() -> &'static str {
    horizon_client::crash::native_mod_call()
        .and_then(|call| call.id)
        .unwrap_or("native")
}

impl Game<'_> {
    /// Override an item's shared-atlas sprite (`item_id`, packed RGBA8 pixels, `[width, height]`).
    pub fn native_item_sprite_set(&mut self, item_id: &str, rgba: &[u8], size: [u32; 2]) -> Result<(), String> {
        let owner = caller_owner();
        let (width, height) = (self.gui.atlas.width, self.gui.atlas.height);
        let (sprites, pixels) = (&self.gui.atlas.sprites, &mut self.gui.atlas.pixels);
        let item_renderer = &self.pack.item_renderer;
        self.native_sprites.set_item(
            owner,
            item_id,
            rgba,
            size,
            width,
            height,
            pixels,
            sprites,
            |id| resolve_item_sprites(item_renderer, sprites, id),
            &mut self.sprite_patches,
        )?;
        Ok(())
    }

    /// Restore an overridden item sprite back to the active resource pack texture.
    pub fn native_item_sprite_reset(&mut self, item_id: &str) -> Result<(), String> {
        let width = self.gui.atlas.width;
        let (sprites, pixels) = (&self.gui.atlas.sprites, &mut self.gui.atlas.pixels);
        self.native_sprites
            .reset_item(item_id, width, pixels, sprites, &mut self.sprite_patches)?;
        Ok(())
    }

    /// Override a block model texture layer (`texture_id`, packed RGBA8 pixels, `[width, height]`) and regenerate its mipmaps.
    pub fn native_block_sprite_set(&mut self, texture_id: &str, rgba: &[u8], size: [u32; 2]) -> Result<(), String> {
        let owner = caller_owner();
        let render = &self.pack.render;
        let (_canonical, layer) = self.native_sprites.set_block(
            owner,
            texture_id,
            rgba,
            size,
            render.sprite_size,
            &render.sprite_layers,
        )?;
        if let Some(renderer) = &mut self.renderer {
            let settings = render.sprite_mips.get(layer as usize).copied().unwrap_or_default();
            renderer
                .queue_block_sprite_layer(layer, rgba, settings, true)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Restore an overridden block sprite layer back to the active resource pack texture and mipmaps.
    pub fn native_block_sprite_reset(&mut self, texture_id: &str) -> Result<(), String> {
        let render = &self.pack.render;
        let Some((_canonical, layer, pixels, still_overridden)) = self.native_sprites.reset_block(
            texture_id,
            render.sprite_size,
            &render.sprite_layers,
            &render.pixels,
        )?
        else {
            return Ok(());
        };
        if !pixels.is_empty()
            && let Some(renderer) = &mut self.renderer
        {
            let settings = render.sprite_mips.get(layer as usize).copied().unwrap_or_else(Settings::default);
            renderer
                .queue_block_sprite_layer(layer, &pixels, settings, still_overridden)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// The item's current sprite pixels `([width, height], rgba)`, including a mod override. Read from the
    /// atlas's CPU copy: no GPU readback, no extra memory kept.
    pub fn native_item_sprite_pixels(&self, item_id: &str) -> Result<([u32; 2], Vec<u8>), String> {
        let atlas = &self.gui.atlas;
        let item_renderer = &self.pack.item_renderer;
        let (w, h, rgba) = SpriteOverrideStore::item_pixels(
            item_id,
            atlas.width,
            atlas.height,
            &atlas.pixels,
            &atlas.sprites,
            |id| resolve_item_sprites(item_renderer, &atlas.sprites, id),
        )?;
        Ok(([w, h], rgba))
    }

    /// The block texture's current pixels `([width, height], rgba)`, including a mod override; an animated
    /// texture gives its first frame. Read from the pack's CPU copy.
    pub fn native_block_sprite_pixels(&self, texture_id: &str) -> Result<([u32; 2], Vec<u8>), String> {
        let render = &self.pack.render;
        let (w, h, rgba) = self.native_sprites.block_pixels(
            texture_id,
            render.sprite_size,
            &render.sprite_layers,
            &render.pixels,
        )?;
        Ok(([w, h], rgba))
    }

    pub(super) fn reapply_native_sprites(&mut self) {
        if self.native_sprites.is_empty() {
            return;
        }
        let (width, height) = (self.gui.atlas.width, self.gui.atlas.height);
        let (sprites, pixels) = (&self.gui.atlas.sprites, &mut self.gui.atlas.pixels);
        let item_renderer = &self.pack.item_renderer;
        self.native_sprites.reload_items(
            width,
            height,
            pixels,
            sprites,
            |id| resolve_item_sprites(item_renderer, sprites, id),
            &mut self.sprite_patches,
        );
        let render = &self.pack.render;
        let updates = self
            .native_sprites
            .collect_block_reapply(render.sprite_size, &render.sprite_layers);
        if let Some(renderer) = &mut self.renderer {
            for (layer, rgba) in updates {
                let settings = render.sprite_mips.get(layer as usize).copied().unwrap_or_default();
                if let Err(error) = renderer.queue_block_sprite_layer(layer, &rgba, settings, true) {
                    log::warn!("Yoake: failed to reapply block layer {layer}: {error}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
