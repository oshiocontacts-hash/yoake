use super::*;

fn rect_for(x: u32, y: u32, w: u32, h: u32, atlas_w: u32, atlas_h: u32) -> Rect {
    Rect {
        u0: x as f32 / atlas_w as f32,
        v0: y as f32 / atlas_h as f32,
        u1: (x + w) as f32 / atlas_w as f32,
        v1: (y + h) as f32 / atlas_h as f32,
        w,
        h,
    }
}

fn fill_rgba(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        out.extend_from_slice(&color);
    }
    out
}

#[test]
fn item_sprite_set_overwrite_and_reset_updates_only_subrect() {
    let (aw, ah) = (64, 64);
    let mut atlas_pixels = fill_rgba(aw, ah, [10, 20, 30, 255]);
    let mut sprites = HashMap::new();
    sprites.insert("item/diamond".into(), rect_for(16, 16, 16, 16, aw, ah));

    let mut store = SpriteOverrideStore::default();
    assert!(store.is_empty());
    let mut patches = Vec::new();
    let red = fill_rgba(16, 16, [255, 0, 0, 255]);
    let key = store
        .set_item(
            "mod_a",
            "diamond",
            &red,
            [16, 16],
            aw,
            ah,
            &mut atlas_pixels,
            &sprites,
            |_| vec!["item/diamond".into()],
            &mut patches,
        )
        .unwrap();
    assert_eq!(key, "minecraft:diamond");
    assert_eq!(store.item_owner("minecraft:diamond"), Some("mod_a"));
    assert_eq!(patches.len(), 1);
    assert_eq!((patches[0].x, patches[0].y, patches[0].width, patches[0].height), (16, 16, 16, 16));
    // Target pixel updated, neighbor pixel untouched.
    let inside = ((16 * aw + 16) * 4) as usize;
    let outside = ((15 * aw + 16) * 4) as usize;
    assert_eq!(&atlas_pixels[inside..inside + 4], &[255, 0, 0, 255]);
    assert_eq!(&atlas_pixels[outside..outside + 4], &[10, 20, 30, 255]);

    // Cross-mod replacement updates owner and pixels.
    let blue = fill_rgba(16, 16, [0, 0, 255, 255]);
    store
        .set_item(
            "mod_b",
            "minecraft:diamond",
            &blue,
            [16, 16],
            aw,
            ah,
            &mut atlas_pixels,
            &sprites,
            |_| vec!["item/diamond".into()],
            &mut patches,
        )
        .unwrap();
    assert_eq!(store.item_owner("minecraft:diamond"), Some("mod_b"));
    assert_eq!(&atlas_pixels[inside..inside + 4], &[0, 0, 255, 255]);

    // Reset restores original snapshotted pixels (from before mod_a).
    patches.clear();
    let reset_key = store
        .reset_item("minecraft:diamond", aw, &mut atlas_pixels, &sprites, &mut patches)
        .unwrap();
    assert_eq!(reset_key.as_deref(), Some("minecraft:diamond"));
    assert!(store.is_empty());
    assert_eq!(patches.len(), 1);
    assert_eq!(&atlas_pixels[inside..inside + 4], &[10, 20, 30, 255]);
}

#[test]
fn wrong_size_and_invalid_inputs_are_refused_without_mutating_atlas() {
    let (aw, ah) = (32, 32);
    let mut atlas_pixels = fill_rgba(aw, ah, [7, 8, 9, 255]);
    let before = atlas_pixels.clone();
    let mut sprites = HashMap::new();
    sprites.insert("item/apple".into(), rect_for(0, 0, 16, 16, aw, ah));
    let mut store = SpriteOverrideStore::default();
    let mut patches = Vec::new();

    // Wrong dimensions for item
    let bad_32 = fill_rgba(32, 32, [255; 4]);
    assert!(
        store
            .set_item(
                "mod_a",
                "minecraft:apple",
                &bad_32,
                [32, 32],
                aw,
                ah,
                &mut atlas_pixels,
                &sprites,
                |_| vec!["item/apple".into()],
                &mut patches,
            )
            .is_err()
    );
    // Zero size / short buffer / bad identifier
    assert!(
        store
            .set_item(
                "mod_a",
                "minecraft:apple",
                &[],
                [0, 16],
                aw,
                ah,
                &mut atlas_pixels,
                &sprites,
                |_| vec!["item/apple".into()],
                &mut patches,
            )
            .is_err()
    );
    assert!(
        store
            .set_item(
                "mod_a",
                "minecraft:apple",
                &[1, 2, 3, 4],
                [16, 16],
                aw,
                ah,
                &mut atlas_pixels,
                &sprites,
                |_| vec!["item/apple".into()],
                &mut patches,
            )
            .is_err()
    );
    assert!(canonical_id("").is_err());
    assert!(canonical_id("bad/../path").is_err());
    assert!(store.is_empty());
    assert!(patches.is_empty());
    assert_eq!(atlas_pixels, before);

    // Wrong dimensions for block
    let layers = BTreeMap::from([("minecraft:block/stone".into(), 0)]);
    assert!(
        store
            .set_block("mod_a", "minecraft:block/stone", &bad_32, [32, 32], 16, &layers)
            .is_err()
    );
    assert!(
        store
            .set_block("mod_a", "minecraft:block/missing", &fill_rgba(16, 16, [1; 4]), [16, 16], 16, &layers)
            .is_err()
    );
    assert!(store.is_empty());
}

#[test]
fn block_sprite_shared_layer_and_reset_restores_originals() {
    let mut layers = BTreeMap::new();
    layers.insert("mymod:block/a".into(), 0);
    layers.insert("mymod:block/b".into(), 0); // shared deduplicated still layer
    layers.insert("minecraft:block/stone".into(), 1);
    let mut pack_pixels = fill_rgba(16, 16, [11, 22, 33, 255]);
    pack_pixels.extend_from_slice(&fill_rgba(16, 16, [44, 55, 66, 255]));

    let mut store = SpriteOverrideStore::default();
    let pix_a = fill_rgba(16, 16, [200, 10, 10, 255]);
    let pix_b = fill_rgba(16, 16, [10, 200, 10, 255]);
    let (_, layer_a) = store
        .set_block("mod_a", "mymod:block/a", &pix_a, [16, 16], 16, &layers)
        .unwrap();
    let (_, layer_b) = store
        .set_block("mod_b", "mymod:block/b", &pix_b, [16, 16], 16, &layers)
        .unwrap();
    assert_eq!((layer_a, layer_b), (0, 0));
    assert_eq!(store.block_owner("mymod:block/b"), Some("mod_b"));

    // Resetting b leaves layer 0 still overridden by a.
    let (_, layer, pixels, still_overridden) = store
        .reset_block("mymod:block/b", 16, &layers, &pack_pixels)
        .unwrap()
        .unwrap();
    assert_eq!(layer, 0);
    assert!(still_overridden);
    assert_eq!(pixels, pix_a);

    // Resetting a restores original pack pixels and clears override flag.
    let (_, layer, pixels, still_overridden) = store
        .reset_block("mymod:block/a", 16, &layers, &pack_pixels)
        .unwrap()
        .unwrap();
    assert_eq!(layer, 0);
    assert!(!still_overridden);
    assert_eq!(&pixels[0..4], &[11, 22, 33, 255]);
}

#[test]
fn resource_pack_reload_reapplies_at_new_rect_and_skips_mismatched_sizes() {
    let mut store = SpriteOverrideStore::default();
    let (aw, ah) = (64, 64);
    let mut atlas_1 = fill_rgba(aw, ah, [1, 2, 3, 255]);
    let sprites_1 = HashMap::from([("item/diamond".into(), rect_for(0, 0, 16, 16, aw, ah))]);
    let mut patches = Vec::new();
    let override_pix = fill_rgba(16, 16, [250, 240, 230, 255]);
    store
        .set_item(
            "mod_a",
            "minecraft:diamond",
            &override_pix,
            [16, 16],
            aw,
            ah,
            &mut atlas_1,
            &sprites_1,
            |_| vec!["item/diamond".into()],
            &mut patches,
        )
        .unwrap();
    let block_layers_1 = BTreeMap::from([("minecraft:block/stone".into(), 3)]);
    store
        .set_block("mod_a", "minecraft:block/stone", &override_pix, [16, 16], 16, &block_layers_1)
        .unwrap();

    // Reload 1: diamond moved to (32, 16), stone moved to layer 9; both 16x16.
    patches.clear();
    let mut atlas_2 = fill_rgba(aw, ah, [50, 60, 70, 255]);
    let sprites_2 = HashMap::from([("item/diamond".into(), rect_for(32, 16, 16, 16, aw, ah))]);
    store.reload_items(
        aw,
        ah,
        &mut atlas_2,
        &sprites_2,
        |_| vec!["item/diamond".into()],
        &mut patches,
    );
    assert_eq!(patches.len(), 1);
    assert_eq!((patches[0].x, patches[0].y), (32, 16));
    let block_layers_2 = BTreeMap::from([("minecraft:block/stone".into(), 9)]);
    let reapplied_blocks = store.collect_block_reapply(16, &block_layers_2);
    assert_eq!(reapplied_blocks.len(), 1);
    assert_eq!(reapplied_blocks[0].0, 9);

    // Reload 2: 32x32 resource pack -> mismatched 16x16 overrides are skipped cleanly without panic.
    patches.clear();
    let mut atlas_3 = fill_rgba(128, 128, [80, 90, 100, 255]);
    let sprites_3 = HashMap::from([("item/diamond".into(), rect_for(0, 0, 32, 32, 128, 128))]);
    store.reload_items(
        128,
        128,
        &mut atlas_3,
        &sprites_3,
        |_| vec!["item/diamond".into()],
        &mut patches,
    );
    assert!(patches.is_empty());
    assert!(store.collect_block_reapply(32, &block_layers_2).is_empty());
    // Overrides remain retained if user switches back to a 16x16 pack later.
    assert!(!store.is_empty());
}

#[test]
fn item_pixels_read_the_atlas_rect_including_a_mod_override() {
    let (aw, ah) = (64, 64);
    let mut atlas_pixels = fill_rgba(aw, ah, [10, 20, 30, 255]);
    let sprites = HashMap::from([("item/diamond".to_owned(), rect_for(16, 16, 16, 8, aw, ah))]);
    let resolve = |_: &str| vec!["item/diamond".to_owned()];

    let (w, h, rgba) =
        SpriteOverrideStore::item_pixels("diamond", aw, ah, &atlas_pixels, &sprites, resolve).unwrap();
    assert_eq!((w, h), (16, 8));
    assert_eq!(rgba, fill_rgba(16, 8, [10, 20, 30, 255]));

    // After an override the read gives the new pixels.
    let mut store = SpriteOverrideStore::default();
    let red = fill_rgba(16, 8, [255, 0, 0, 255]);
    store
        .set_item("m", "diamond", &red, [16, 8], aw, ah, &mut atlas_pixels, &sprites, resolve, &mut Vec::new())
        .unwrap();
    let (_, _, rgba) =
        SpriteOverrideStore::item_pixels("minecraft:diamond", aw, ah, &atlas_pixels, &sprites, resolve).unwrap();
    assert_eq!(rgba, red);

    // Unknown item, bad id, missing pixel buffer.
    assert!(SpriteOverrideStore::item_pixels("nope", aw, ah, &atlas_pixels, &sprites, |_| Vec::new()).is_err());
    assert!(SpriteOverrideStore::item_pixels("a b", aw, ah, &atlas_pixels, &sprites, resolve).is_err());
    assert!(SpriteOverrideStore::item_pixels("diamond", aw, ah, &[], &sprites, resolve).is_err());
}

#[test]
fn block_pixels_give_override_then_shared_layer_then_pack_first_frame() {
    let mut layers = BTreeMap::new();
    layers.insert("mymod:block/a".into(), 0);
    layers.insert("mymod:block/b".into(), 0); // shared layer
    layers.insert("minecraft:block/stone".into(), 1);
    let mut pack_pixels = fill_rgba(16, 16, [11, 22, 33, 255]);
    pack_pixels.extend_from_slice(&fill_rgba(16, 16, [44, 55, 66, 255]));

    let mut store = SpriteOverrideStore::default();
    let (w, h, rgba) = store.block_pixels("minecraft:block/stone", 16, &layers, &pack_pixels).unwrap();
    assert_eq!((w, h), (16, 16));
    assert_eq!(&rgba[0..4], &[44, 55, 66, 255]);
    // Without a mod override nothing is stored.
    assert!(store.is_empty());

    let green = fill_rgba(16, 16, [0, 200, 0, 255]);
    store.set_block("m", "mymod:block/a", &green, [16, 16], 16, &layers).unwrap();
    // Both names on the shared layer read the override; another layer is untouched.
    assert_eq!(store.block_pixels("mymod:block/a", 16, &layers, &pack_pixels).unwrap().2, green);
    assert_eq!(store.block_pixels("mymod:block/b", 16, &layers, &pack_pixels).unwrap().2, green);
    assert_eq!(&store.block_pixels("stone-less", 16, &layers, &pack_pixels).err().unwrap()[..7], "unknown");
    assert_eq!(
        &store.block_pixels("minecraft:block/stone", 16, &layers, &pack_pixels).unwrap().2[0..4],
        &[44, 55, 66, 255]
    );

    store.reset_block("mymod:block/a", 16, &layers, &pack_pixels).unwrap();
    assert_eq!(&store.block_pixels("mymod:block/a", 16, &layers, &pack_pixels).unwrap().2[0..4], &[11, 22, 33, 255]);
    // Pixels dropped from the pack (`finish_without_pixels`) is an error, not a panic.
    assert!(store.block_pixels("mymod:block/a", 16, &layers, &[]).is_err());
}
