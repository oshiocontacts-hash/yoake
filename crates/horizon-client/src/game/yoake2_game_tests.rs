//! MX3K1 end to end with a real `Game`: a Rust guest, compiled to a wasm32 core module with the SDK, registers
//! the server-tick hook, and in `Game::yoake2_server_tick` (called from the integrated-server tick start) finds
//! a dropped item by id through a Game path, reads its position and writes a writable field; the change is
//! visible in the `Game`. A guest that did not register the hook is not entered.
//!
//! Needs the game resources (the other ignored tests in this crate use the same variable):
//!
//!   HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3 \
//!   cargo test --offline -j 4 -p horizon-client --lib --features yoake2-slice yoake2_game -- --ignored

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use horizon_client::entities::ItemEntity;
use horizon_client::items::ItemStack;
use horizon_client::mod_api::NativeAuthority;
use horizon_core::geometry::Vec3;
use yoake_slice::runtime::{self, ModConfig};

use crate::game::{Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources};

#[path = "../../../yoake-guest-sdk/tests/support/build.rs"]
mod guest_build;

const ITEM: u32 = 4242;

fn server_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"));
    let settings = Settings::default();
    let loaded = load_resources(root.clone(), None, &root, &settings, Instant::now(), false, true, &Progress::new(), &mut |_, _| unreachable!("no menu handoff")).unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::IntegratedServer;
    options.dedicated = Some(Vec::new());
    let mut game = Game::new(&loaded.resources, loaded.pack, loaded.gui.unwrap(), options, settings, Audio::silent(&root), None).unwrap();
    game.native_lifecycle.begin_open(NativeAuthority::IntegratedServer);
    game.native_lifecycle.world_open_ready();
    test(&mut game);
}

fn item(id: u32, x: f64) -> ItemEntity {
    let at = Vec3::new(x, 64.0, 0.5);
    ItemEntity {
        join: Default::default(),
        no_gravity: false,
        id,
        thrower: None,
        target: None,
        stack: ItemStack { item: Arc::from("minecraft:stone"), count: 3, damage: 0, extra: None },
        health: 5,
        fire_ticks: 0,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        on_ground: true,
        vertical_collision: false,
        horizontal_collision: false,
        age: 0,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
    }
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]
fn a_rust_guest_in_the_server_tick_hook_edits_a_dropped_item_of_a_real_game() {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_guest_e2e");
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let guest = guest_build::build_guest(&rlib, &sdk.join("tests/guests/game_tick.rs"), &scratch, "game_tick");
    let other = guest_build::build_guest(&rlib, &sdk.join("tests/guests/only_outer.rs"), &scratch, "only_outer");

    server_game(|game| {
        super::native_client_items::yoake2_install_dispatch();
        game.dropped.push(item(ITEM, 12.5));
        game.dropped.push(item(7, 99.0));

        // 1. no mod installed: the tick works and costs nothing
        let _ = runtime::uninstall();
        game.native_lifecycle_server_tick_start(true);
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 0);

        // 2. a guest that registered a different hook is never entered
        runtime::install(&other, ModConfig::new("")).unwrap();
        game.native_lifecycle_server_tick_start(true);
        assert_eq!(runtime::stats().entries, 0, "the server tick is not hook 1");
        assert!(runtime::take_reports().unwrap().is_empty());
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 0);

        // 3. the server-tick guest: finds item 4242 by id, reads its position, writes fire_ticks
        runtime::install(&guest, ModConfig::new("")).unwrap();
        assert_eq!(runtime::stats().registered, vec![(runtime::HOOK_SERVER_TICK, 0)]);
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 1), [i64::from(runtime::HOOK_SERVER_TICK)], "the guest ran in the server-tick hook");
        assert_eq!(slot(&r, 2), Vec::<i64>::new(), "the handle was made");
        assert_eq!(f64::from_bits(slot(&r, 3)[0] as u64), 12.5, "the position of item 4242");
        assert_eq!(f64::from_bits(slot(&r, 4)[0] as u64), 64.0);
        assert_eq!(slot(&r, 5), [1], "fire_ticks is writable");
        assert_eq!(slot(&r, 6), [1], "an item that does not exist is Stale, not a trap");
        let edited = game.dropped.iter().find(|d| d.id == ITEM).unwrap();
        assert_eq!(edited.fire_ticks, 55, "the write is visible in the Game");
        assert_eq!(game.dropped.iter().find(|d| d.id == 7).unwrap().fire_ticks, 0, "the other item is untouched");
        let stats = runtime::stats();
        assert!(!stats.disabled, "{:?}", stats.trap);
        assert_eq!(stats.entries, 1);
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        assert_eq!(yoake_slice::activation::live_path_handles(), 0);

        // 4. the item is gone: the same guest gets Stale for it and the game goes on
        game.dropped.retain(|d| d.id != ITEM);
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 2).len(), 1, "no handle for a removed item");
        assert!(!runtime::stats().disabled);
        let _ = runtime::uninstall();
    });
}
