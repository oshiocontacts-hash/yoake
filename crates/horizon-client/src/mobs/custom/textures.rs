//! Runtime entity textures for fully custom mobs (FR-F4-MOBS.md section 3.8, F4M-7).
//!
//! Client-side only: nothing is sent over the network, and each client renders its own runtime
//! textures. Textures live inside the reserved 1024x1024 `horizon/mob-textures` sub-region of
//! the shared entity atlas, updated with partial `font::Patch` uploads.
use crate::font::Patch;
use horizon_client_render::hud::{MOB_TEXTURE_SLOTS, MOB_TEXTURES_SIZE};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// Maximum runtime mob textures a single mod may keep alive at once (matches `gui-texture`).
pub const MAX_SLOTS_PER_MOD: usize = MOB_TEXTURE_SLOTS;
/// Side length (texels) of the reserved `horizon/mob-textures` region in the entity atlas.
pub const REGION_SIZE: u32 = MOB_TEXTURES_SIZE;
/// Maximum payload size (bytes) accepted by a single `mob-texture-write` call (4 MiB).
pub const MAX_WRITE_BYTES: usize = 4 * 1024 * 1024;
/// Maximum length of a binding slot name.
pub const MAX_SLOT_NAME: usize = 128;

/// Resolved atlas UV rectangles (`[u0, v0, u1, v1]`) bound to one mob for this frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobTextureOverrides {
    entries: Vec<(String, [f32; 4])>,
}

impl MobTextureOverrides {
    /// Looks up the UV rect bound to `slot` (`"default"`, a layer name, `"glowmask"`, or a bone name).
    pub fn get(&self, slot: &str) -> Option<[f32; 4]> {
        self.entries
            .iter()
            .find(|(name, _)| name == slot)
            .map(|(_, uv)| *uv)
    }

    /// Iterates every `(slot, uv)` binding on this mob.
    pub fn iter(&self) -> impl Iterator<Item = (&str, [f32; 4])> {
        self.entries.iter().map(|(name, uv)| (name.as_str(), *uv))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Clone, Debug)]
struct TextureEntry {
    id: u32,
    owner: String,
    generation: u64,
    width: u32,
    height: u32,
    /// Sub-rectangle `[x, y, width, height]` inside the `REGION_SIZE x REGION_SIZE` mob region.
    rect: [u32; 4],
    rgba: Vec<u8>,
    full_dirty: bool,
    dirty_rects: Vec<[u32; 4]>,
}

/// CPU-side backing store, sub-region allocator, and binding table for runtime custom mob textures.
#[derive(Debug)]
pub struct MobTextureStore {
    next_id: u32,
    free: Vec<[u32; 4]>,
    entries: BTreeMap<u32, TextureEntry>,
    /// `(entity_id, canonical_slot_name) -> texture_id`.
    bindings: BTreeMap<(u32, String), u32>,
}

impl Default for MobTextureStore {
    fn default() -> Self {
        Self {
            next_id: 1,
            free: vec![[0, 0, REGION_SIZE, REGION_SIZE]],
            entries: BTreeMap::new(),
            bindings: BTreeMap::new(),
        }
    }
}

fn canonical_slot(slot: &str) -> Result<String, String> {
    if slot.len() > MAX_SLOT_NAME || slot.chars().any(char::is_control) {
        return Err(format!("invalid mob texture slot `{slot}`"));
    }
    if slot.is_empty() {
        Ok("default".to_owned())
    } else {
        Ok(slot.to_owned())
    }
}

fn coalesce_free_rects(free: &mut Vec<[u32; 4]>) {
    free.retain(|r| r[2] > 0 && r[3] > 0);
    let mut merged = true;
    while merged {
        merged = false;
        'outer: for i in 0..free.len() {
            for j in (i + 1)..free.len() {
                let a = free[i];
                let b = free[j];
                // Horizontal merge: same y and height, touching x edges.
                if a[1] == b[1] && a[3] == b[3] {
                    if a[0] + a[2] == b[0] {
                        free[i] = [a[0], a[1], a[2] + b[2], a[3]];
                        free.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                    if b[0] + b[2] == a[0] {
                        free[i] = [b[0], b[1], a[2] + b[2], a[3]];
                        free.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
                // Vertical merge: same x and width, touching y edges.
                if a[0] == b[0] && a[2] == b[2] {
                    if a[1] + a[3] == b[1] {
                        free[i] = [a[0], a[1], a[2], a[3] + b[3]];
                        free.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                    if b[1] + b[3] == a[1] {
                        free[i] = [b[0], b[1], a[2], a[3] + b[3]];
                        free.swap_remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
            }
        }
    }
}

impl MobTextureStore {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.bindings.is_empty()
    }

    pub fn active_count(&self) -> usize {
        self.entries.len()
    }

    pub fn owner_count(&self, owner: &str) -> usize {
        self.entries.values().filter(|e| e.owner == owner).count()
    }

    fn alloc_rect(&mut self, width: u32, height: u32) -> Option<[u32; 4]> {
        let best = self
            .free
            .iter()
            .enumerate()
            .filter(|(_, r)| r[2] >= width && r[3] >= height)
            .min_by_key(|(_, r)| (r[2] * r[3], r[1], r[0]))
            .map(|(i, _)| i)?;
        let r = self.free.swap_remove(best);
        let [fx, fy, fw, fh] = r;
        let rem_w = fw - width;
        let rem_h = fh - height;
        // Guillotine split: split along the shorter axis so full-width rows stay unfragmented.
        if fw < fh {
            if rem_w > 0 {
                self.free.push([fx + width, fy, rem_w, fh]);
            }
            if rem_h > 0 {
                self.free.push([fx, fy + height, width, rem_h]);
            }
        } else {
            if rem_h > 0 {
                self.free.push([fx, fy + height, fw, rem_h]);
            }
            if rem_w > 0 {
                self.free.push([fx + width, fy, rem_w, height]);
            }
        }
        Some([fx, fy, width, height])
    }

    fn free_rect(&mut self, rect: [u32; 4]) {
        if self.entries.is_empty() {
            self.free.clear();
            self.free.push([0, 0, REGION_SIZE, REGION_SIZE]);
        } else {
            self.free.push(rect);
            coalesce_free_rects(&mut self.free);
        }
    }

    /// Allocates a `width` x `height` runtime texture for `(owner, generation)`.
    pub fn create(
        &mut self,
        owner: &str,
        generation: u64,
        width: u32,
        height: u32,
    ) -> Result<u32, String> {
        if width == 0 || height == 0 || width > REGION_SIZE || height > REGION_SIZE {
            return Err(format!(
                "mob texture size {width}x{height} must be 1..={REGION_SIZE} on each side"
            ));
        }
        if self.owner_count(owner) >= MAX_SLOTS_PER_MOD {
            return Err(format!(
                "mob texture limit reached ({MAX_SLOTS_PER_MOD} slots per mod)"
            ));
        }
        let rect = self
            .alloc_rect(width, height)
            .ok_or_else(|| "mob texture atlas region is full".to_owned())?;
        let mut id = self.next_id;
        while id == 0 || self.entries.contains_key(&id) {
            id = id.wrapping_add(1);
        }
        self.next_id = id.wrapping_add(1).max(1);
        let bytes = (width as usize) * (height as usize) * 4;
        self.entries.insert(
            id,
            TextureEntry {
                id,
                owner: owner.to_owned(),
                generation,
                width,
                height,
                rect,
                rgba: vec![0; bytes],
                full_dirty: true,
                dirty_rects: Vec::new(),
            },
        );
        Ok(id)
    }

    /// Writes `rgba` into the sub-rectangle `[origin.0, origin.1, size.0, size.1]` of `id`.
    pub fn write(
        &mut self,
        owner: &str,
        id: u32,
        origin: [u32; 2],
        size: [u32; 2],
        rgba: &[u8],
    ) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(&id)
            .ok_or_else(|| format!("unknown mob texture {id}"))?;
        if entry.owner != owner {
            return Err(format!("mob texture {id} belongs to another mod"));
        }
        if size[0] == 0 || size[1] == 0 {
            return Err("mob texture write size must be non-zero".to_owned());
        }
        let end_x = origin[0]
            .checked_add(size[0])
            .filter(|&e| e <= entry.width)
            .ok_or_else(|| "mob texture write exceeds width".to_owned())?;
        let end_y = origin[1]
            .checked_add(size[1])
            .filter(|&e| e <= entry.height)
            .ok_or_else(|| "mob texture write exceeds height".to_owned())?;
        let _ = (end_x, end_y);
        let expected = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| "mob texture write byte size overflow".to_owned())?;
        if expected > MAX_WRITE_BYTES {
            return Err(format!(
                "mob texture write ({expected} bytes) exceeds 4 MiB limit"
            ));
        }
        if rgba.len() != expected {
            return Err(format!(
                "mob texture write expected {expected} RGBA8 bytes for {}x{}, got {}",
                size[0],
                size[1],
                rgba.len()
            ));
        }
        let row_bytes = size[0] as usize * 4;
        for row in 0..size[1] as usize {
            let src_start = row * row_bytes;
            let dst_y = origin[1] as usize + row;
            let dst_start = (dst_y * entry.width as usize + origin[0] as usize) * 4;
            entry.rgba[dst_start..dst_start + row_bytes]
                .copy_from_slice(&rgba[src_start..src_start + row_bytes]);
        }
        if origin == [0, 0] && size == [entry.width, entry.height] {
            entry.full_dirty = true;
            entry.dirty_rects.clear();
        } else if !entry.full_dirty {
            if entry.dirty_rects.len() >= 32 {
                entry.full_dirty = true;
                entry.dirty_rects.clear();
            } else {
                entry.dirty_rects.push([origin[0], origin[1], size[0], size[1]]);
            }
        }
        Ok(())
    }

    /// Binds `texture_id` (or `None` to unbind) to `slot` on `entity`.
    pub fn bind(
        &mut self,
        owner: &str,
        entity: u32,
        slot: &str,
        texture_id: Option<u32>,
    ) -> Result<(), String> {
        let slot = canonical_slot(slot)?;
        match texture_id {
            Some(id) => {
                let entry = self
                    .entries
                    .get(&id)
                    .ok_or_else(|| format!("unknown mob texture {id}"))?;
                if entry.owner != owner {
                    return Err(format!("mob texture {id} belongs to another mod"));
                }
                self.bindings.insert((entity, slot), id);
                Ok(())
            }
            None => {
                if let Some(&bound) = self.bindings.get(&(entity, slot.clone())) {
                    let owned = self
                        .entries
                        .get(&bound)
                        .is_none_or(|e| e.owner == owner);
                    if !owned {
                        return Err(format!("slot `{slot}` on mob {entity} is bound by another mod"));
                    }
                    self.bindings.remove(&(entity, slot));
                }
                Ok(())
            }
        }
    }

    /// Frees every texture (and its bindings) created by `(owner, generation)`.
    pub fn release_instance(&mut self, owner: &str, generation: u64) {
        let removed: Vec<(u32, [u32; 4])> = self
            .entries
            .values()
            .filter(|e| e.owner == owner && e.generation == generation)
            .map(|e| (e.id, e.rect))
            .collect();
        self.remove_entries(&removed);
    }

    /// Frees every texture (and its bindings) created by `owner` across all generations.
    pub fn release_owner(&mut self, owner: &str) {
        let removed: Vec<(u32, [u32; 4])> = self
            .entries
            .values()
            .filter(|e| e.owner == owner)
            .map(|e| (e.id, e.rect))
            .collect();
        self.remove_entries(&removed);
    }

    fn remove_entries(&mut self, removed: &[(u32, [u32; 4])]) {
        if removed.is_empty() {
            return;
        }
        for (id, _) in removed {
            self.entries.remove(id);
            self.bindings.retain(|_, bound| bound != id);
        }
        for (_, rect) in removed {
            self.free_rect(*rect);
        }
    }

    /// Marks every active texture dirty so it re-uploads on the next frame (atlas rebuild / device loss).
    pub fn mark_all_dirty(&mut self) {
        for entry in self.entries.values_mut() {
            entry.full_dirty = true;
            entry.dirty_rects.clear();
        }
    }

    /// Drains dirty full/sub-rect patches for upload into the entity atlas.
    pub fn flush_patches(
        &mut self,
        region: [u32; 4],
        atlas_size: [u32; 2],
        out: &mut Vec<Patch>,
    ) {
        if region[2] < REGION_SIZE
            || region[3] < REGION_SIZE
            || region[0].saturating_add(region[2]) > atlas_size[0]
            || region[1].saturating_add(region[3]) > atlas_size[1]
        {
            return;
        }
        for entry in self.entries.values_mut() {
            let base_x = region[0] + entry.rect[0];
            let base_y = region[1] + entry.rect[1];
            if entry.full_dirty {
                out.push(Patch {
                    x: base_x,
                    y: base_y,
                    width: entry.width,
                    height: entry.height,
                    rgba: entry.rgba.clone(),
                });
                entry.full_dirty = false;
                entry.dirty_rects.clear();
            } else if !entry.dirty_rects.is_empty() {
                for [ox, oy, w, h] in entry.dirty_rects.drain(..) {
                    let mut sub = Vec::with_capacity((w as usize) * (h as usize) * 4);
                    let row_bytes = w as usize * 4;
                    for row in 0..h as usize {
                        let src_y = oy as usize + row;
                        let start = (src_y * entry.width as usize + ox as usize) * 4;
                        sub.extend_from_slice(&entry.rgba[start..start + row_bytes]);
                    }
                    out.push(Patch {
                        x: base_x + ox,
                        y: base_y + oy,
                        width: w,
                        height: h,
                        rgba: sub,
                    });
                }
            }
        }
    }

    /// Resolves every bound slot of `entity` into atlas UV rectangles `[u0, v0, u1, v1]`.
    pub fn overrides_for(
        &self,
        entity: u32,
        region: [u32; 4],
        atlas_size: [u32; 2],
    ) -> Option<MobTextureOverrides> {
        if atlas_size[0] == 0
            || atlas_size[1] == 0
            || region[2] < REGION_SIZE
            || region[3] < REGION_SIZE
        {
            return None;
        }
        let mut out = Vec::new();
        let aw = atlas_size[0] as f32;
        let ah = atlas_size[1] as f32;
        for ((e, slot), tex_id) in self.bindings.range((entity, String::new())..) {
            if *e != entity {
                break;
            }
            if let Some(entry) = self.entries.get(tex_id) {
                let x0 = region[0] + entry.rect[0];
                let y0 = region[1] + entry.rect[1];
                let x1 = x0 + entry.width;
                let y1 = y0 + entry.height;
                out.push((
                    slot.clone(),
                    [x0 as f32 / aw, y0 as f32 / ah, x1 as f32 / aw, y1 as f32 / ah],
                ));
            }
        }
        (!out.is_empty()).then_some(MobTextureOverrides { entries: out })
    }

    /// Reads back the CPU RGBA8 pixels of `id` (tests).
    pub fn read_back(&self, id: u32) -> Option<(u32, u32, &[u8])> {
        self.entries
            .get(&id)
            .map(|e| (e.width, e.height, e.rgba.as_slice()))
    }

    /// The allocated `[x, y, w, h]` inside the `REGION_SIZE x REGION_SIZE` region (tests).
    pub fn rect_of(&self, id: u32) -> Option<[u32; 4]> {
        self.entries.get(&id).map(|e| e.rect)
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

static STORE: OnceLock<Mutex<MobTextureStore>> = OnceLock::new();

pub fn store() -> &'static Mutex<MobTextureStore> {
    STORE.get_or_init(Default::default)
}

pub fn is_empty() -> bool {
    STORE
        .get()
        .is_some_and(|s| s.lock().unwrap_or_else(|e| e.into_inner()).is_empty())
}

pub fn create(owner: &str, generation: u64, width: u32, height: u32) -> Result<u32, String> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .create(owner, generation, width, height)
}

pub fn write(
    owner: &str,
    id: u32,
    origin: [u32; 2],
    size: [u32; 2],
    rgba: &[u8],
) -> Result<(), String> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .write(owner, id, origin, size, rgba)
}

pub fn bind(owner: &str, entity: u32, slot: &str, texture_id: Option<u32>) -> Result<(), String> {
    store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .bind(owner, entity, slot, texture_id)
}

pub fn release_instance(owner: &str, generation: u64) {
    if let Some(s) = STORE.get() {
        s.lock()
            .unwrap_or_else(|e| e.into_inner())
            .release_instance(owner, generation);
    }
}

pub fn mark_all_dirty() {
    if let Some(s) = STORE.get() {
        s.lock()
            .unwrap_or_else(|e| e.into_inner())
            .mark_all_dirty();
    }
}

pub fn flush_patches(region: [u32; 4], atlas_size: [u32; 2], out: &mut Vec<Patch>) {
    if let Some(s) = STORE.get() {
        s.lock()
            .unwrap_or_else(|e| e.into_inner())
            .flush_patches(region, atlas_size, out);
    }
}

pub fn overrides_for(
    entity: u32,
    region: [u32; 4],
    atlas_size: [u32; 2],
) -> Option<MobTextureOverrides> {
    STORE
        .get()?
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .overrides_for(entity, region, atlas_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_subrect_and_read_back_produces_partial_atlas_patch() {
        let mut store = MobTextureStore::default();
        let id = store.create("mod_a", 1, 16, 16).unwrap();
        let full = vec![10u8; 16 * 16 * 4];
        store.write("mod_a", id, [0, 0], [16, 16], &full).unwrap();

        let mut patches = Vec::new();
        let region = [2048, 1024, 1024, 1024];
        store.flush_patches(region, [4096, 4096], &mut patches);
        assert_eq!(patches.len(), 1);
        assert_eq!((patches[0].width, patches[0].height), (16, 16));
        assert_eq!(patches[0].rgba, full);

        // Partial 4x4 sub-region write at (3, 5).
        let sub: Vec<u8> = (0..16).flat_map(|i| [200 + i as u8, 50, 25, 255]).collect();
        store.write("mod_a", id, [3, 5], [4, 4], &sub).unwrap();
        patches.clear();
        store.flush_patches(region, [4096, 4096], &mut patches);
        assert_eq!(patches.len(), 1);
        let rect = store.rect_of(id).unwrap();
        assert_eq!(patches[0].x, region[0] + rect[0] + 3);
        assert_eq!(patches[0].y, region[1] + rect[1] + 5);
        assert_eq!((patches[0].width, patches[0].height), (4, 4));
        assert_eq!(patches[0].rgba, sub);

        let (w, h, pixels) = store.read_back(id).unwrap();
        assert_eq!((w, h), (16, 16));
        let off = (5 * 16 + 3) * 4;
        assert_eq!(&pixels[off..off + 16], &sub[0..16]);
        assert_eq!(&pixels[0..4], &[10, 10, 10, 10]);
    }

    #[test]
    fn slot_limits_dimension_checks_and_four_mib_write_cap_are_enforced() {
        let mut store = MobTextureStore::default();
        assert!(store.create("mod_a", 1, 0, 16).is_err());
        assert!(store.create("mod_a", 1, 16, 0).is_err());
        assert!(store.create("mod_a", 1, REGION_SIZE + 1, 16).is_err());

        for _ in 0..MAX_SLOTS_PER_MOD {
            store.create("mod_a", 1, 64, 64).unwrap();
        }
        let err = store.create("mod_a", 1, 64, 64).unwrap_err();
        assert!(err.contains("64"), "{err}");

        // Another mod still has its own 64-slot quota if space remains.
        let other = store.create("mod_b", 1, 1024, 16);
        assert!(other.is_ok(), "{other:?}");

        // Cross-mod write/bind refused; out-of-bounds write refused.
        let tex_b = other.unwrap();
        assert!(store.write("mod_a", tex_b, [0, 0], [1, 1], &[1, 2, 3, 4]).is_err());
        assert!(store.bind("mod_a", 7, "default", Some(tex_b)).is_err());
        assert!(store.write("mod_b", tex_b, [1024, 0], [1, 1], &[1, 2, 3, 4]).is_err());
        assert!(store.write("mod_b", tex_b, [0, 0], [1, 1], &[1, 2, 3]).is_err());

        // 4 MiB write limit: 1024*1024*4 == 4 MiB succeeds; exceeding it fails.
        let mut full_store = MobTextureStore::default();
        let big = full_store.create("mod_c", 1, 1024, 1024).unwrap();
        let ok_buf = vec![0xAA; MAX_WRITE_BYTES];
        assert!(full_store.write("mod_c", big, [0, 0], [1024, 1024], &ok_buf).is_ok());
        // A second 1024x1024 cannot fit in the 1024x1024 reserved region.
        assert!(full_store.create("mod_c", 1, 1, 1).is_err());
    }

    #[test]
    fn atlas_rebuild_reuploads_active_slots_and_unload_cleans_up_by_generation() {
        let mut store = MobTextureStore::default();
        let t1 = store.create("mod_a", 1, 32, 32).unwrap();
        let t2 = store.create("mod_a", 2, 32, 32).unwrap();
        let t_b = store.create("mod_b", 1, 32, 32).unwrap();
        let px1 = vec![0x11; 32 * 32 * 4];
        let px2 = vec![0x22; 32 * 32 * 4];
        let px_b = vec![0x33; 32 * 32 * 4];
        store.write("mod_a", t1, [0, 0], [32, 32], &px1).unwrap();
        store.write("mod_a", t2, [0, 0], [32, 32], &px2).unwrap();
        store.write("mod_b", t_b, [0, 0], [32, 32], &px_b).unwrap();
        store.bind("mod_a", 10, "", Some(t1)).unwrap();
        store.bind("mod_a", 10, "glow", Some(t2)).unwrap();
        store.bind("mod_b", 11, "head", Some(t_b)).unwrap();

        let region1 = [0, 0, 1024, 1024];
        let mut patches = Vec::new();
        store.flush_patches(region1, [2048, 2048], &mut patches);
        assert_eq!(patches.len(), 3);
        patches.clear();
        store.flush_patches(region1, [2048, 2048], &mut patches);
        assert!(patches.is_empty());

        // Resource pack reload rebuilds the atlas at a new region offset.
        store.mark_all_dirty();
        let region2 = [1024, 2048, 1024, 1024];
        store.flush_patches(region2, [4096, 4096], &mut patches);
        assert_eq!(patches.len(), 3);
        assert!(patches.iter().all(|p| p.x >= 1024 && p.y >= 2048));

        // Unloading generation 1 of mod_a drops t1 and its "default" binding, keeping generation 2 and mod_b.
        store.release_instance("mod_a", 1);
        assert!(store.read_back(t1).is_none());
        assert!(store.read_back(t2).is_some());
        assert!(store.read_back(t_b).is_some());
        let ov = store.overrides_for(10, region2, [4096, 4096]).unwrap();
        assert!(ov.get("default").is_none());
        assert!(ov.get("glow").is_some());
        assert!(store.overrides_for(11, region2, [4096, 4096]).unwrap().get("head").is_some());

        // Unbinding explicitly removes the remaining slot.
        store.bind("mod_a", 10, "glow", None).unwrap();
        assert!(store.overrides_for(10, region2, [4096, 4096]).is_none());
    }
}
