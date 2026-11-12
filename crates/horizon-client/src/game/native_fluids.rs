use super::Game;
use horizon_client::mod_api::NativeFluidDefinition;

pub(super) fn register() -> std::io::Result<()> {
    let mut fluids = Vec::<NativeFluidDefinition>::new();
    Game::native_fluids(&mut fluids);
    horizon_world::fluid::custom::install(fluids)
}
