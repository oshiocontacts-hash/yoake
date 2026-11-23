//! A data-pack biome is in the registry a server sends, with its data, at the
//! place the chunk encoder numbers it by, and a client rebuilds it from that
//! data.
use horizon_client::{
    biome::{BiomeColors, Biomes},
    chunk::{Chunk, LevelShapes},
    lan::data::{BIOME_REGISTRY, SyncData, client_biomes},
    save,
    scoreboard::json_to_tag,
};
use horizon_world::{nbt::Tag, registry::BlockRegistry};
use serde_json::Value;
use std::path::PathBuf;

/// Removes the folder when the test ends, passed or not.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// A resource folder with two biomes and 1x1 colormaps, loaded.
fn write_base(root: &std::path::Path) -> Biomes {
    let maps = root.join("assets/minecraft/textures/colormap");
    let biome_dir = root.join("data/minecraft/worldgen/biome");
    std::fs::create_dir_all(&maps).unwrap();
    std::fs::create_dir_all(&biome_dir).unwrap();
    std::fs::write(root.join("registries.json"), "{}").unwrap();
    for name in ["grass", "foliage", "dry_foliage"] {
        let file = std::fs::File::create(maps.join(format!("{name}.png"))).unwrap();
        let mut png = png::Encoder::new(file, 1, 1);
        png.set_color(png::ColorType::Rgb);
        png.set_depth(png::BitDepth::Eight);
        let mut writer = png.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3]).unwrap();
    }
    for name in ["plains", "river"] {
        std::fs::write(biome_dir.join(format!("{name}.json")), "{}").unwrap();
    }
    Biomes::load(root).unwrap()
}

#[test]
fn a_pack_biome_is_sent_with_its_data_and_read_back_by_a_client() {
    let root = std::env::temp_dir().join(format!("horizon-sync-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let pack: Value = serde_json::json!({
        "effects": {"grass_color": "#123456"},
        "attributes": {
            "minecraft:visual/sky_color": "#010203",
            "minecraft:visual/fog_color": 0x040506,
            "minecraft:visual/water_fog_color": 0x070809,
            "minecraft:visual/ambient_particles": [{
                "particle": {"type": "minecraft:firefly"}, "probability": 0.5
            }],
            "minecraft:audio/ambient_sounds": {"loop": "mypack:wind"},
            "minecraft:audio/background_music": {"default": {
                "sound": "mypack:song", "min_delay": 20, "max_delay": 40
            }}
        }
    });
    let world = base.with_packs([("minecraft:tundra_x", &pack)]);
    let sync =
        SyncData::load_with(&root, &horizon_world::loot::PackOverlay::NONE, Some(&world)).unwrap();

    // The encoder numbers a biome by its place in this list: none may be
    // missing, and the pack's is not the first.
    let registry = &sync.dynamic.registries[BIOME_REGISTRY];
    let names: Vec<_> = registry.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(
        names,
        ["minecraft:plains", "minecraft:river", "minecraft:tundra_x"]
    );
    assert!(world.list.iter().all(|b| names.contains(&b.name.as_str())));

    // A client that knows the vanilla pack still gets the pack biome's data.
    let sent = sync.registry_packets(true, &LevelShapes::VANILLA);
    let biomes = sent.iter().find(|r| r.registry == BIOME_REGISTRY).unwrap();
    assert!(biomes.entries[0].data.is_none(), "a known one is not sent");
    assert!(biomes.entries[2].data.is_some(), "the pack's biome is sent");

    // The client builds its table from what arrived: the server's id for
    // the name, and the pack's colour.
    let received: Vec<_> = biomes
        .entries
        .iter()
        .map(|e| (e.id.clone(), e.data.clone()))
        .collect();
    let client = client_biomes(&base, &received).expect("the pack's biome is new to the client");
    let id = client.id("minecraft:tundra_x");
    assert_eq!(id, world.id("minecraft:tundra_x"));
    let received_biome = client.get(id);
    assert_eq!(received_biome.grass, 0x123456);
    assert_eq!(
        (received_biome.sky, received_biome.fog, received_biome.water_fog),
        (Some(0x010203), Some(0x040506), 0x070809)
    );
    assert_eq!(
        received_biome.particles,
        [(horizon_client::particles::ParticleType::Firefly, 0.5)]
    );
    assert_eq!(
        received_biome.ambient.as_ref().unwrap().r#loop.as_deref(),
        Some("mypack:wind")
    );
    assert_eq!(
        received_biome
            .music
            .as_ref()
            .unwrap()
            .default
            .as_ref()
            .unwrap()
            .sound,
        "mypack:song"
    );
    assert_ne!(
        received_biome.grass,
        client.get(client.id("minecraft:plains")).grass
    );

    // Nothing new to a client that knows them all: it keeps its own table.
    // even when the server sends data for them (another grass colour here).
    let other: Value = serde_json::from_str(r##"{"effects":{"grass_color":"#654321"}}"##).unwrap();
    let other = json_to_tag(&other);
    let known: Vec<_> = received
        .iter()
        .filter(|(n, _)| base.contains(n))
        .map(|(n, _)| (n.clone(), Some(other.clone())))
        .collect();
    assert!(client_biomes(&base, &known).is_none());

    // Beside a new biome, a known one keeps the game's colour.
    let mut mixed = known.clone();
    mixed.extend(received.iter().filter(|(n, _)| !base.contains(n)).cloned());
    let client = client_biomes(&base, &mixed).unwrap();
    let plains = base.get(base.id("minecraft:plains")).grass;
    assert_ne!(plains, 0x654321);
    assert_eq!(client.get(client.id("minecraft:plains")).grass, plains);
    assert_eq!(client.get(client.id("minecraft:tundra_x")).grass, 0x123456);
}

#[test]
fn recolouring_twice_starts_from_the_biome_files_each_time() {
    let root = std::env::temp_dir().join(format!("horizon-recolor-biomes-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let grass = |b: &Biomes| b.get(b.id("minecraft:plains")).grass;
    let darker = |_: &str, c: &mut BiomeColors| c.grass -= 0x10;
    let once = base.recolored_from(&base, darker).expect("colours changed");
    assert_eq!(grass(&once), grass(&base) - 0x10);
    // The same colours again, asked from the base: nothing to rebuild.
    assert!(base.recolored_from(&once, darker).is_none());
    // Other colours replace the first ones rather than add to them.
    let lighter = |_: &str, c: &mut BiomeColors| c.grass += 0x10;
    let next = base.recolored_from(&once, lighter).unwrap();
    assert_eq!(grass(&next), grass(&base) + 0x10);
}

#[test]
fn a_pack_biome_chunk_saves_its_name_and_loads_with_its_id() {
    let root = std::env::temp_dir().join(format!("horizon-save-biome-{}", std::process::id()));
    let _scratch = Scratch(root.clone());
    let base = write_base(&root);
    let pack = serde_json::json!({"effects": {"water_color": 0x123456}});
    let biomes = base.with_packs([("mypack:azure_caves", &pack)]);
    let id = biomes.id("mypack:azure_caves");
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("states", Tag::List(vec![Tag::String("minecraft:air".into())])),
        (
            "defaults",
            Tag::compound([("minecraft:air", Tag::Int(0))]),
        ),
    ]))
    .unwrap();
    let mut chunk = Chunk::empty(2, -3);
    for qy in 0..96 {
        for qz in 0..4 {
            for qx in 0..4 {
                chunk.set_biome_cell(qx, qy, qz, id);
            }
        }
    }
    let encoded = save::encode(&chunk, &registry, &biomes);
    let sections = encoded
        .tag
        .get("sections")
        .and_then(|tag| match tag {
            Tag::List(list) => Some(list),
            _ => None,
        })
        .unwrap();
    assert!(sections.iter().any(|section| {
        section
            .get("biomes")
            .and_then(|b| b.get("palette"))
            .and_then(|tag| match tag {
                Tag::List(list) => Some(list),
                _ => None,
            })
            .is_some_and(|palette| {
                palette.iter().any(|entry| {
                    matches!(entry, Tag::String(name) if name.to_string_lossy() == "mypack:azure_caves")
                })
            })
    }));
    let decoded = save::decode(&encoded, &registry, &biomes).unwrap();
    assert_eq!(decoded.biome(32, 64, -48), id);
}
