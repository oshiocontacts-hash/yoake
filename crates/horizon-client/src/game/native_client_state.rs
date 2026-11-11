use super::PendingWorldEntry;
use horizon_client::{net, settings::Settings};
use std::path::Path;

/// The destination of the currently loaded client world, not its display name.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeClientWorld {
    Singleplayer {
        folder: String,
    },
    Server {
        address: String,
    },
    LanGuest {
        address: String,
    },
    Replay {
        title: String,
    },
    /// A friend connection, without exposing its secret invite address.
    Friend,
}

/// Sound-option categories, in the engine's sound-source order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NativeSoundCategory {
    Master,
    Music,
    Records,
    Weather,
    Blocks,
    Hostile,
    Neutral,
    Players,
    Ambient,
    Voice,
    Ui,
}

impl NativeSoundCategory {
    fn source(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Music => "music",
            Self::Records => "record",
            Self::Weather => "weather",
            Self::Blocks => "block",
            Self::Hostile => "hostile",
            Self::Neutral => "neutral",
            Self::Players => "player",
            Self::Ambient => "ambient",
            Self::Voice => "voice",
            Self::Ui => "ui",
        }
    }
}

// CHECK: Record every entry, including memory joins, and balance client-level notifications.
#[derive(Default)]
pub(super) struct ClientState {
    pending_world: Option<Option<NativeClientWorld>>,
    world: Option<NativeClientWorld>,
    levels: usize,
}

impl ClientState {
    pub(super) fn enter(&mut self, entry: &PendingWorldEntry) {
        match entry {
            PendingWorldEntry::Replay { title, .. } => self.enter_replay(title),
            PendingWorldEntry::Local { folder, .. } => self.enter_singleplayer(folder),
            PendingWorldEntry::Server { address, lan, .. } => {
                self.pending_world = Some(server_world(address, *lan));
            }
        }
    }

    pub(super) fn enter_singleplayer(&mut self, folder: &str) {
        self.pending_world = Some(
            Path::new(folder)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|folder| NativeClientWorld::Singleplayer {
                    folder: folder.to_owned(),
                }),
        );
    }

    pub(super) fn enter_replay(&mut self, title: &str) {
        self.pending_world = Some(Some(NativeClientWorld::Replay {
            title: title.to_owned(),
        }));
    }

    pub(super) fn enter_shared(&mut self, host: &Self) {
        self.pending_world = Some(host.world.clone());
    }

    pub(super) fn level_loaded(&mut self) {
        self.levels = self.levels.saturating_add(1);
        if let Some(world) = self.pending_world.take() {
            self.world = world;
        }
    }

    pub(super) fn level_unloaded(&mut self) {
        self.levels = self.levels.saturating_sub(1);
    }

    pub(super) fn world_key(&self, is_client: bool, title_open: bool) -> Option<NativeClientWorld> {
        if !is_client || title_open || self.levels == 0 {
            return None;
        }
        self.world.clone()
    }
}

fn server_world(address: &str, lan: bool) -> Option<NativeClientWorld> {
    if address.trim().starts_with(net::FRIEND) {
        return Some(NativeClientWorld::Friend);
    }
    let (host, port) = horizon_protocol::servers::parse_address(address)?;
    let host = host.to_ascii_lowercase();
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    Some(if lan {
        NativeClientWorld::LanGuest { address }
    } else {
        NativeClientWorld::Server { address }
    })
}

pub(super) fn sound_volume(settings: &Settings, category: NativeSoundCategory) -> f32 {
    settings.volume(category.source())
}

pub(super) fn sound_volume_effective(settings: &Settings, category: NativeSoundCategory) -> f32 {
    settings.final_volume(category.source())
}

pub(super) fn playing(
    screen_is_playing: bool,
    title_open: bool,
    sign_edit_open: bool,
    book_open: bool,
) -> bool {
    screen_is_playing && !title_open && !sign_edit_open && !book_open
}

// CHECK: These tests are authored only; run them after applying the integration patch.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Screen;

    const CATEGORIES: [NativeSoundCategory; 11] = [
        NativeSoundCategory::Master,
        NativeSoundCategory::Music,
        NativeSoundCategory::Records,
        NativeSoundCategory::Weather,
        NativeSoundCategory::Blocks,
        NativeSoundCategory::Hostile,
        NativeSoundCategory::Neutral,
        NativeSoundCategory::Players,
        NativeSoundCategory::Ambient,
        NativeSoundCategory::Voice,
        NativeSoundCategory::Ui,
    ];

    fn server_entry(address: &str, lan: bool) -> PendingWorldEntry {
        PendingWorldEntry::Server {
            address: address.to_owned(),
            name: "Display name is not the address".to_owned(),
            lan,
            cookies: None,
        }
    }

    fn loaded(entry: &PendingWorldEntry) -> ClientState {
        let mut state = ClientState::default();
        state.enter(entry);
        state.level_loaded();
        state
    }

    #[test]
    fn singleplayer_uses_the_folder_not_the_display_name_or_path() {
        let entry = PendingWorldEntry::Local {
            folder: Path::new("saves")
                .join("WebScreen World")
                .to_str()
                .unwrap()
                .to_owned(),
            name: Some("A different display name".to_owned()),
            seed: None,
            creative: false,
            create: None,
        };
        let state = loaded(&entry);
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::Singleplayer {
                folder: "WebScreen World".to_owned(),
            })
        );
    }

    #[test]
    fn replay_key_becomes_visible_only_after_a_level_is_loaded() {
        let mut state = ClientState::default();
        state.enter(&PendingWorldEntry::Replay {
            title: "Fixture".into(),
            name: "ReplayViewer".into(),
        });
        assert_eq!(state.world_key(true, false), None);
        state.level_loaded();
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::Replay { title: "Fixture".into() })
        );
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), None);
    }

    #[test]
    fn singleplayer_provenance_survives_client_level_replacement() {
        let mut state = ClientState::default();
        state.enter_singleplayer("LanHost");
        state.level_loaded();
        state.level_loaded();
        state.level_unloaded();
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::Singleplayer {
                folder: "LanHost".to_owned(),
            })
        );
    }

    #[test]
    fn split_guest_inherits_the_hosts_save_folder() {
        let mut host = ClientState::default();
        host.enter_singleplayer("Shared World");
        host.level_loaded();
        let mut guest = ClientState::default();
        guest.enter_shared(&host);
        assert_eq!(guest.world_key(true, false), None);
        guest.level_loaded();
        assert_eq!(
            guest.world_key(true, false),
            Some(NativeClientWorld::Singleplayer {
                folder: "Shared World".to_owned(),
            })
        );
    }

    #[test]
    fn server_addresses_use_the_server_list_identity() {
        for (joined, normalized) in [
            (" Example.COM ", "example.com:25565"),
            ("Example.COM:25565", "example.com:25565"),
            ("Example.COM:25566", "example.com:25566"),
            ("[2001:DB8::1]:25566", "[2001:db8::1]:25566"),
            ("[2001:DB8::1]", "[2001:db8::1]:25565"),
            ("2001:DB8::1", "[2001:db8::1]:25565"),
        ] {
            let state = loaded(&server_entry(joined, false));
            assert_eq!(
                state.world_key(true, false),
                Some(NativeClientWorld::Server {
                    address: normalized.to_owned(),
                })
            );
        }
    }

    #[test]
    fn lan_guest_keeps_the_joined_address() {
        let state = loaded(&server_entry("192.168.1.8:41234", true));
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::LanGuest {
                address: "192.168.1.8:41234".to_owned(),
            })
        );
    }

    #[test]
    fn replay_identity_follows_client_level_lifetime() {
        let mut state = ClientState::default();
        state.enter_replay("Demo replay");
        assert_eq!(state.world_key(true, false), None);
        state.level_loaded();
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::Replay {
                title: "Demo replay".to_owned(),
            })
        );
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), None);
    }

    #[test]
    fn friend_world_does_not_retain_the_invite_secret() {
        let address = format!("{}private-invite", net::FRIEND);
        let state = loaded(&server_entry(&address, false));
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::Friend)
        );
        assert!(state.pending_world.is_none());
    }

    #[test]
    fn title_loading_disconnected_and_non_client_states_have_no_world_key() {
        let mut state = ClientState::default();
        assert_eq!(state.world_key(true, false), None);
        state.enter(&server_entry("example.com", false));
        assert_eq!(state.world_key(true, false), None);
        state.level_loaded();
        assert!(state.world_key(true, false).is_some());
        assert_eq!(state.world_key(true, true), None);
        assert_eq!(state.world_key(false, false), None);
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), None);
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), None);
    }

    #[test]
    fn dimension_replacement_and_reconfiguration_keep_the_world_key() {
        let mut state = loaded(&server_entry("example.com", false));
        let expected = state.world_key(true, false);
        state.level_loaded();
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), expected);
        state.level_unloaded();
        assert_eq!(state.world_key(true, false), None);
        state.level_loaded();
        assert_eq!(state.world_key(true, false), expected);
    }

    #[test]
    fn a_new_entry_replaces_identity_only_when_its_level_loads() {
        let mut state = loaded(&server_entry("old.example", false));
        let old = state.world_key(true, false);
        state.enter(&server_entry("192.168.1.8:41234", true));
        assert_eq!(state.world_key(true, false), old);
        state.level_loaded();
        state.level_unloaded();
        assert_eq!(
            state.world_key(true, false),
            Some(NativeClientWorld::LanGuest {
                address: "192.168.1.8:41234".to_owned(),
            })
        );
    }

    #[test]
    fn invalid_destinations_do_not_reuse_an_old_world_key() {
        for address in ["", " ", "example.com:invalid", "[::1"] {
            let mut state = loaded(&server_entry("old.example", false));
            state.level_unloaded();
            state.enter(&server_entry(address, false));
            state.level_loaded();
            assert_eq!(state.world_key(true, false), None);
        }
        let mut state = ClientState::default();
        state.enter_singleplayer("");
        state.level_loaded();
        assert_eq!(state.world_key(true, false), None);
    }

    #[test]
    fn every_sound_category_maps_to_its_slider() {
        let settings = Settings {
            master_volume: 0.5,
            volumes: [
                0.9375, 0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 0.0625,
            ],
            ..Settings::default()
        };
        let sliders = [
            0.5, 0.0, 0.125, 0.25, 0.375, 0.5, 0.625, 0.75, 0.875, 1.0, 0.0625,
        ];
        for (index, category) in CATEGORIES.into_iter().enumerate() {
            let slider = sliders[index];
            assert_eq!(
                category.source(),
                horizon_client::settings::SOUND_SOURCES[index]
            );
            assert_eq!(sound_volume(&settings, category), slider);
            let effective = if category == NativeSoundCategory::Master {
                slider
            } else {
                settings.master_volume * slider
            };
            assert_eq!(sound_volume_effective(&settings, category), effective);
        }
    }

    #[test]
    fn effective_volume_obeys_master_mute_and_unit_endpoints() {
        for master_volume in [0.0, 1.0] {
            let settings = Settings {
                master_volume,
                volumes: [1.0; 11],
                ..Settings::default()
            };
            for category in CATEGORIES {
                assert_eq!(sound_volume_effective(&settings, category), master_volume);
            }
            assert_eq!(sound_volume(&settings, NativeSoundCategory::Ui), 1.0);
        }
    }

    #[test]
    fn playing_is_false_for_every_non_playing_screen() {
        for screen in [
            Screen::GameModeSwitcher,
            Screen::Paused,
            Screen::Options,
            Screen::Chat,
            Screen::Creative,
            Screen::Inventory,
            Screen::Dead,
            Screen::Stats,
            Screen::Advancements,
            Screen::WorldOptions,
            Screen::Friends,
            Screen::Social,
            Screen::CommandBlock,
            Screen::BlockEditor,
            Screen::ServerLinks,
            Screen::ConfirmLink,
            Screen::DialogWarning,
            Screen::Win,
            Screen::DebugOptions,
            Screen::Mod,
        ] {
            assert!(!playing(screen == Screen::Playing, false, false, false));
        }
        assert!(playing(true, false, false, false));
    }

    #[test]
    fn title_sign_and_book_overlays_prevent_playing() {
        for title_open in [false, true] {
            for sign_edit_open in [false, true] {
                for book_open in [false, true] {
                    assert_eq!(
                        playing(true, title_open, sign_edit_open, book_open),
                        !title_open && !sign_edit_open && !book_open
                    );
                }
            }
        }
    }
}
