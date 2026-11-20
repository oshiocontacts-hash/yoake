//! W9c: a hook on a fn that takes `&Game` reaches the level (`Game.world`) and the settings (`Game.settings`)
//! through the generated bindings of this crate, read-only. The `Game` here is a stand-in with the same two paths
//! (the real one needs a window and a GPU); the bindings under test, `world::yoake_glue` and
//! `settings::yoake_glue`, are the generated ones. The paths of the real `Game` are the generated
//! `impl PathRootRef for Game` in horizon-client (checked to compile by `cargo check --features yoake2-slice`).
#![cfg(feature = "yoake2-slice")]

use horizon_client_render::settings::{self, Settings};
use horizon_client_render::world::{self, World};
use horizon_world::registry::BlockStateId;
use yoake_slice::activation::{self, Accessor, ArgSet, PathRootRef, Status};
use yoake_slice::codec::{CallError, Decode, Decoder};

struct Game {
    world: World,
    settings: Settings,
}

const WORLD: u32 = 9;
const SETTINGS: u32 = 10;

impl PathRootRef for Game {
    fn resolve_ref(&self, path: &[Accessor], f: &mut dyn FnMut(&dyn std::any::Any)) -> Result<(), Status> {
        match path {
            [Accessor::Field(9)] => {
                f(&self.world);
                Ok(())
            }
            [Accessor::Field(10)] => {
                f(&self.settings);
                Ok(())
            }
            _ => Err(Status::BadField),
        }
    }
}

fn id_of(bindings: &[(u32, &str)], key: &str) -> u32 {
    bindings.iter().find(|(_, k)| *k == key).unwrap_or_else(|| panic!("no binding {key}")).0
}

fn call(module: fn(u64, u32, &[u8], &mut Vec<u8>) -> Result<(), CallError>, handle: u64, id: u32) -> Result<Vec<u8>, CallError> {
    let mut out = Vec::new();
    module(handle, id, &[], &mut out)?;
    Ok(out)
}

#[test]
fn a_shared_game_hook_reads_the_level_and_the_settings_and_cannot_write() {
    let mut world = World::new(BlockStateId(0), 42);
    world.day_time = 6000;
    let mut settings = Settings::default();
    settings.fov = 90.0;
    let game = Game { world, settings };

    let day_time = id_of(world::yoake_glue::BINDINGS, "render_world::World.day_time#get");
    let day_time_set = id_of(world::yoake_glue::BINDINGS, "render_world::World.day_time#set");
    let fov = id_of(settings::yoake_glue::BINDINGS, "render_settings::Settings.fov#get");
    let fov_set = id_of(settings::yoake_glue::BINDINGS, "render_settings::Settings.fov#set");

    let set = ArgSet::new().root_shared(&game);
    let (seen, finished) = activation::scope_n(set, &mut |_| 0, |root| {
        let w = activation::open_path(root, vec![Accessor::Field(WORLD)]).unwrap();
        let s = activation::open_path(root, vec![Accessor::Field(SETTINGS)]).unwrap();

        let bytes = call(world::yoake_glue::call, w, day_time).unwrap();
        let day = i64::decode(&mut Decoder::new(&bytes)).unwrap();
        let bytes = call(settings::yoake_glue::call, s, fov).unwrap();
        let field_of_view = f32::decode(&mut Decoder::new(&bytes)).unwrap();

        // read-only: setters are refused (the fields are not in `writable`), and a handle of one module is the wrong type for the other
        assert!(matches!(call(world::yoake_glue::call, w, day_time_set), Err(CallError::ReadOnly(_))));
        assert!(matches!(call(settings::yoake_glue::call, s, fov_set), Err(CallError::ReadOnly(_))));
        assert!(matches!(call(settings::yoake_glue::call, w, fov), Err(CallError::Handle(Status::WrongType))));
        // a `&mut` borrow through a shared root is refused
        assert_eq!(activation::with_arg::<World, _>(w, |_| ()), Err(Status::ReadOnly));
        (day, field_of_view)
    })
    .unwrap();
    assert_eq!(seen, (6000, 90.0));
    let _ = finished;
    assert_eq!(game.world.day_time, 6000, "nothing was written");
}
