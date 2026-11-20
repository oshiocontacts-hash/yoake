# `render_settings`: every function (horizon-client-render/src/settings.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `render_settings::<Opt as Clone>::clone` | 731 | &self | derived (not counted) | - |
| `render_settings::<Opt as Debug>::fmt` | 731 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `render_settings::<Opt as PartialEq>::eq` | 731 | &self | derived (not counted) | - |
| `render_settings::<Settings as Clone>::clone` | 100 | &self | derived (not counted) | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value] |
| `render_settings::<Settings as Debug>::fmt` | 100 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `render_settings::<Settings as Default>::default` | 216 | - | handle-required | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value] |
| `render_settings::Settings::chat_options` | 716 | &self | bound (b, mechanical) | - |
| `render_settings::Settings::client_information` | 515 | &self | handle-required | external type `horizon_protocol::packets::configuration::ClientInformation` (crate not loaded) [return value] |
| `render_settings::Settings::describe_for_crashes` | 380 | &self | bound (a) | - |
| `render_settings::Settings::final_volume` | 692 | &self | bound (a) | - |
| `render_settings::Settings::from_file` | 420 | - | handle-required | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value]; field `values` of `OptionsFile` is private to another module |
| `render_settings::Settings::load` | 337 | - | handle-required | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value]; std type `std::path::Path` has no codec yet [&argument] |
| `render_settings::Settings::narrator` | 700 | &self | handle-required | external type `horizon_narrator::Status` (crate not loaded) [return value] |
| `render_settings::Settings::parse` | 416 | - | handle-required | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value] |
| `render_settings::Settings::save` | 668 | &self | handle-required | std type `std::path::Path` has no codec yet [&argument] |
| `render_settings::Settings::serialize` | 665 | &self | bound (a) | - |
| `render_settings::Settings::set_narrator` | 703 | &mut self | handle-required | external type `horizon_narrator::Status` (crate not loaded) |
| `render_settings::Settings::text_background` | 709 | &self | bound (a) | - |
| `render_settings::Settings::to_file` | 529 | &self | handle-required | field `values` of `OptionsFile` is private to another module [return value] |
| `render_settings::Settings::video_mode_index` | 329 | &self | bound (a) | - |
| `render_settings::Settings::volume` | 679 | &self | bound (a) | - |
| `render_settings::Settings::with_option` | 642 | &self | handle-required | external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) [return value] |
| `render_settings::default_language` | 13 | - | bound (a) | - |
| `render_settings::put_b` | 311 | - | handle-required | field `values` of `OptionsFile` is private to another module [&mut argument] |
| `render_settings::put_f` | 301 | - | handle-required | field `values` of `OptionsFile` is private to another module [&mut argument] |
| `render_settings::put_i` | 306 | - | handle-required | field `values` of `OptionsFile` is private to another module [&mut argument] |
| `render_settings::put_name` | 316 | - | handle-required | field `values` of `OptionsFile` is private to another module [&mut argument] |
| `render_settings::put_s` | 321 | - | handle-required | field `values` of `OptionsFile` is private to another module [&mut argument] |
| `render_settings::set_source_volumes` | 20 | - | bound (a) | - |
| `render_settings::sound_source` | 25 | - | bound (b, mechanical) | - |
| `render_settings::source_volume` | 92 | - | bound (b, mechanical) | - |

## Fields

| field | status |
|---|---|
| `render_settings::Settings.fov` | get + set |
| `render_settings::Settings.render_distance` | get + set |
| `render_settings::Settings.simulation_distance` | get + set |
| `render_settings::Settings.sensitivity` | get + set |
| `render_settings::Settings.master_volume` | get + set |
| `render_settings::Settings.volumes` | get + set |
| `render_settings::Settings.sound_device` | get + set |
| `render_settings::Settings.gui_scale` | get + set |
| `render_settings::Settings.view_bobbing` | get + set |
| `render_settings::Settings.gamma` | get + set |
| `render_settings::Settings.max_fps` | get + set |
| `render_settings::Settings.vsync` | get + set |
| `render_settings::Settings.fullscreen` | get + set |
| `render_settings::Settings.smooth_lighting` | get + set |
| `render_settings::Settings.clouds` | get + set |
| `render_settings::Settings.cloud_range` | get + set |
| `render_settings::Settings.particles` | get + set |
| `render_settings::Settings.entity_distance` | get + set |
| `render_settings::Settings.entity_shadows` | get + set |
| `render_settings::Settings.biome_blend` | get + set |
| `render_settings::Settings.graphics_preset` | get + set |
| `render_settings::Settings.mipmap_levels` | get + set |
| `render_settings::Settings.weather_radius` | get + set |
| `render_settings::Settings.chunk_fade` | get + set |
| `render_settings::Settings.vignette` | get + set |
| `render_settings::Settings.autosave_indicator` | get + set |
| `render_settings::Settings.cutout_leaves` | get + set |
| `render_settings::Settings.improved_transparency` | get + set |
| `render_settings::Settings.attack_indicator` | get + set |
| `render_settings::Settings.inactivity_fps_limit` | get + set |
| `render_settings::Settings.prioritize_chunk_updates` | get + set |
| `render_settings::Settings.menu_blur` | get + set |
| `render_settings::Settings.invert_y` | get + set |
| `render_settings::Settings.mouse_wheel_sensitivity` | get + set |
| `render_settings::Settings.discrete_scroll` | get + set |
| `render_settings::Settings.toggle_crouch` | get + set |
| `render_settings::Settings.toggle_sprint` | get + set |
| `render_settings::Settings.auto_jump` | get + set |
| `render_settings::Settings.language` | get + set |
| `render_settings::Settings.force_unicode` | get + set |
| `render_settings::Settings.japanese_glyphs` | get + set |
| `render_settings::Settings.chat_visibility` | get + set |
| `render_settings::Settings.chat_colors` | get + set |
| `render_settings::Settings.chat_links` | get + set |
| `render_settings::Settings.chat_links_prompt` | get + set |
| `render_settings::Settings.chat_opacity` | get + set |
| `render_settings::Settings.chat_delay` | get + set |
| `render_settings::Settings.text_background_opacity` | get + set |
| `render_settings::Settings.chat_scale` | get + set |
| `render_settings::Settings.chat_width` | get + set |
| `render_settings::Settings.chat_height_focused` | get + set |
| `render_settings::Settings.chat_height_unfocused` | get + set |
| `render_settings::Settings.chat_line_spacing` | get + set |
| `render_settings::Settings.auto_suggestions` | get + set |
| `render_settings::Settings.reduced_debug_info` | get + set |
| `render_settings::Settings.high_contrast` | get + set |
| `render_settings::Settings.show_subtitles` | get + set |
| `render_settings::Settings.notification_display_time` | get + set |
| `render_settings::Settings.main_hand_left` | get + set |
| `render_settings::Settings.resource_packs` | get + set |
| `render_settings::Settings.keys` | no accessor: external type `horizon_client_input::keybinds::KeyBinds` (crate not loaded) |
| `render_settings::Settings.file` | no accessor: field `values` of `OptionsFile` is private to another module |
| `render_settings::Settings.invert_x` | get + set |
| `render_settings::Settings.allow_cursor_changes` | get + set |
| `render_settings::Settings.toggle_attack` | get + set |
| `render_settings::Settings.toggle_use` | get + set |
| `render_settings::Settings.sprint_window` | get + set |
| `render_settings::Settings.operator_items_tab` | get + set |
| `render_settings::Settings.quit_shortcuts` | get + set |
| `render_settings::Settings.ctrl_click_emulates_right_click` | get + set |
| `render_settings::Settings.horizon` | get + set |
| `render_settings::Settings.startup_api` | get + set |
| `render_settings::Settings.video_modes` | no accessor: external type `horizon_client_input::window::VideoMode` (crate not loaded) |
| `render_settings::Settings.controller` | no accessor: external type `horizon_client_input::gamepad::PadSettings` (crate not loaded) |
| `render_settings::Settings.extras` | no accessor: external type `horizon_cosmetics::prefs::Others` (crate not loaded) |
| `render_settings::Settings.mods` | get + set |
| `render_settings::Settings.shader_packs` | no accessor: external type `horizon_shader_api::manifest::PackOption` (crate not loaded) |
| `render_settings::Settings.shader_options` | no accessor: external type `horizon_shaders::options::PackOption` (crate not loaded) |
