use super::*;

#[derive(Default)]
struct Registry {
    next_generation: u64,
    hosts: HashMap<usize, HashSet<u64>>,
    shaders: HashMap<(usize, String), u64>,
    screens: HashMap<usize, u64>,
    item_sprites: HashMap<String, (usize, u64)>,
    block_sprites: HashMap<String, (usize, u64)>,
    pending: Vec<Cleanup>,
}

struct Cleanup {
    id: &'static str,
    owner: usize,
    generation: u64,
    streams: Vec<NativeAudioStreamId>,
    shaders: Vec<String>,
    screen: bool,
    item_sprites: Vec<String>,
    block_sprites: Vec<String>,
}

impl Registry {
    fn register(&mut self, owner: usize) -> u64 {
        self.next_generation = self.next_generation.wrapping_add(1);
        let generation = self.next_generation;
        self.hosts.entry(owner).or_default().insert(generation);
        generation
    }

    fn release(&mut self, mut cleanup: Cleanup) {
        if let Some(hosts) = self.hosts.get_mut(&cleanup.owner) {
            hosts.remove(&cleanup.generation);
            if hosts.is_empty() {
                self.hosts.remove(&cleanup.owner);
            }
        }
        cleanup.shaders = self
            .shaders
            .iter()
            .filter_map(|((owner, id), generation)| {
                (*owner == cleanup.owner && *generation == cleanup.generation).then(|| id.clone())
            })
            .collect();
        cleanup.screen = self.screens.get(&cleanup.owner) == Some(&cleanup.generation);
        cleanup.item_sprites = self
            .item_sprites
            .iter()
            .filter_map(|(id, &(owner, generation))| {
                (owner == cleanup.owner && generation == cleanup.generation).then(|| id.clone())
            })
            .collect();
        cleanup.block_sprites = self
            .block_sprites
            .iter()
            .filter_map(|(id, &(owner, generation))| {
                (owner == cleanup.owner && generation == cleanup.generation).then(|| id.clone())
            })
            .collect();
        if !cleanup.streams.is_empty()
            || !cleanup.shaders.is_empty()
            || cleanup.screen
            || !cleanup.item_sprites.is_empty()
            || !cleanup.block_sprites.is_empty()
        {
            self.pending.push(cleanup);
        }
    }

    fn take(&mut self) -> Vec<Cleanup> {
        let mut pending = std::mem::take(&mut self.pending);
        for cleanup in &mut pending {
            cleanup.shaders.retain(|id| {
                let key = (cleanup.owner, id.clone());
                if self.shaders.get(&key) == Some(&cleanup.generation) {
                    self.shaders.remove(&key);
                    true
                } else {
                    false
                }
            });
            cleanup.screen &= self.screens.get(&cleanup.owner) == Some(&cleanup.generation);
            if cleanup.screen {
                self.screens.remove(&cleanup.owner);
            }
            cleanup.item_sprites.retain(|id| {
                if self.item_sprites.get(id) == Some(&(cleanup.owner, cleanup.generation)) {
                    self.item_sprites.remove(id);
                    true
                } else {
                    false
                }
            });
            cleanup.block_sprites.retain(|id| {
                if self.block_sprites.get(id) == Some(&(cleanup.owner, cleanup.generation)) {
                    self.block_sprites.remove(id);
                    true
                } else {
                    false
                }
            });
        }
        pending
    }
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
static PENDING: AtomicBool = AtomicBool::new(false);

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

pub(super) struct Registration {
    id: &'static str,
    owner: usize,
    generation: u64,
    released: bool,
}

impl Registration {
    pub(super) fn new(id: &'static str, owner: usize) -> Self {
        Self {
            id,
            owner,
            generation: registry().register(owner),
            released: false,
        }
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn claim_shader(&self, id: String) {
        registry().shaders.insert((self.owner, id), self.generation);
    }

    pub(super) fn remove_shader(&self, id: &str) {
        let mut state = registry();
        let key = (self.owner, id.to_owned());
        if state.shaders.get(&key) == Some(&self.generation) {
            state.shaders.remove(&key);
        }
    }

    pub(super) fn claim_screen(&self) {
        registry().screens.insert(self.owner, self.generation);
    }

    pub(super) fn close_screen(&self) {
        let mut state = registry();
        if state.screens.get(&self.owner) == Some(&self.generation) {
            state.screens.remove(&self.owner);
        }
    }

    pub(super) fn claim_item_sprite(&self, id: String) {
        registry()
            .item_sprites
            .insert(id, (self.owner, self.generation));
    }

    pub(super) fn remove_item_sprite(&self, id: &str) {
        let mut state = registry();
        if state.item_sprites.get(id) == Some(&(self.owner, self.generation)) {
            state.item_sprites.remove(id);
        }
    }

    pub(super) fn claim_block_sprite(&self, id: String) {
        registry()
            .block_sprites
            .insert(id, (self.owner, self.generation));
    }

    pub(super) fn remove_block_sprite(&self, id: &str) {
        let mut state = registry();
        if state.block_sprites.get(id) == Some(&(self.owner, self.generation)) {
            state.block_sprites.remove(id);
        }
    }

    pub(super) fn release(&mut self, streams: Vec<NativeAudioStreamId>) {
        if self.released {
            return;
        }
        self.released = true;
        let mut state = registry();
        state.release(Cleanup {
            id: self.id,
            owner: self.owner,
            generation: self.generation,
            streams,
            shaders: Vec::new(),
            screen: false,
            item_sprites: Vec::new(),
            block_sprites: Vec::new(),
        });
        PENDING.store(!state.pending.is_empty(), Ordering::Release);
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.release(Vec::new());
    }
}

pub(super) fn owners() -> HashSet<usize> {
    REGISTRY
        .get()
        .map(|state| {
            state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .hosts
                .keys()
                .copied()
                .collect()
        })
        .unwrap_or_default()
}

fn apply(host: &mut dyn Host, pending: Vec<Cleanup>) {
    for cleanup in pending {
        let _caller = crate::crash::enter_native_mod(crate::crash::NativeModCall {
            id: Some(cleanup.id),
            index: cleanup.owner,
            hook: "wasm-client-cleanup",
        });
        let _sound = crate::mod_api::enter_local_sound_caller(cleanup.owner);
        for stream in cleanup.streams {
            host.audio_stream_close(stream);
        }
        for shader in cleanup.shaders {
            let _ = host.unregister_post_shader(&shader);
        }
        if cleanup.screen {
            host.mod_screen_close(cleanup.id);
        }
        for item_id in cleanup.item_sprites {
            let _ = host.item_sprite_reset(&item_id);
        }
        for texture_id in cleanup.block_sprites {
            let _ = host.block_sprite_reset(&texture_id);
        }
    }
}

pub(super) fn drain_with(host: &mut dyn Host) {
    if !PENDING.load(Ordering::Acquire) {
        return;
    }
    let pending = {
        let mut state = registry();
        let pending = state.take();
        PENDING.store(false, Ordering::Release);
        pending
    };
    apply(host, pending);
}

pub(super) fn drain() {
    if PENDING.load(Ordering::Acquire) {
        let _ = service(|invocation| {
            let host = invocation.host.write().ok_or(w::HostError::Unsupported)?;
            drain_with(host);
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleanup(owner: usize, generation: u64) -> Cleanup {
        Cleanup {
            id: "test",
            owner,
            generation,
            streams: Vec::new(),
            shaders: Vec::new(),
            screen: false,
            item_sprites: Vec::new(),
            block_sprites: Vec::new(),
        }
    }

    #[test]
    fn unload_releases_only_owned_resources_once() {
        let mut state = Registry::default();
        let first = state.register(1);
        let other = state.register(2);
        state.shaders.insert((1, "bloom".into()), first);
        state.shaders.insert((2, "bloom".into()), other);
        state.screens.insert(1, first);
        state.release(cleanup(1, first));
        assert!(!state.hosts.contains_key(&1));
        assert!(state.hosts.contains_key(&2));
        let pending = state.take();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].shaders, ["bloom"]);
        assert!(pending[0].screen);
        assert_eq!(state.shaders.len(), 1);
        assert!(state.take().is_empty());
    }

    #[test]
    fn delayed_cleanup_does_not_remove_reloaded_resources() {
        let mut state = Registry::default();
        let old = state.register(1);
        state.shaders.insert((1, "bloom".into()), old);
        state.screens.insert(1, old);
        state.release(cleanup(1, old));
        let new = state.register(1);
        state.shaders.insert((1, "bloom".into()), new);
        state.screens.insert(1, new);
        let pending = state.take();
        assert!(pending[0].shaders.is_empty());
        assert!(!pending[0].screen);
        assert_eq!(state.shaders[&(1, "bloom".into())], new);
        assert_eq!(state.hosts[&1], HashSet::from([new]));
    }

    #[test]
    fn cleanup_stops_pcm_without_closing_another_mods_stream() {
        struct AudioHost {
            audio: crate::audio::Audio,
            shaders: HashSet<String>,
            screen: bool,
        }

        impl Host for AudioHost {
            fn audio_stream_close(&mut self, id: NativeAudioStreamId) -> bool {
                self.audio
                    .close_audio_stream(crate::mod_api::local_sound_caller().unwrap(), id)
            }

            fn unregister_post_shader(&mut self, id: &str) -> Result<(), String> {
                assert!(self.shaders.remove(id));
                Ok(())
            }

            fn mod_screen_close(&mut self, id: &str) {
                assert_eq!(id, "test");
                self.screen = false;
            }
        }

        let mut audio = crate::audio::Audio::silent(std::path::Path::new("unused-audio-assets"));
        audio.enable_native_mix();
        audio.begin_fixed_audio_mix().unwrap();
        let first = audio
            .open_audio_stream(1, 48_000, 2, 0, None, false)
            .unwrap();
        let other = audio
            .open_audio_stream(2, 48_000, 2, 0, None, false)
            .unwrap();
        audio.push_audio_stream(1, first, &[0.25; 32]).unwrap();
        audio.push_audio_stream(2, other, &[0.25; 32]).unwrap();
        let mut host = AudioHost {
            audio,
            shaders: HashSet::from(["bloom".into()]),
            screen: true,
        };
        let mut state = Registry::default();
        let generation = state.register(1);
        state.shaders.insert((1, "bloom".into()), generation);
        state.screens.insert(1, generation);
        let mut cleanup = cleanup(1, generation);
        cleanup.streams.push(first);
        state.release(cleanup);
        let caller = crate::mod_api::local_sound_caller();
        apply(&mut host, state.take());
        assert_eq!(host.audio.queued_audio_stream(1, first), None);
        assert!(host.audio.push_audio_stream(1, first, &[0.25; 32]).is_err());
        assert_eq!(host.audio.queued_audio_stream(2, other), Some(32));
        assert!(host.shaders.is_empty());
        assert!(!host.screen);
        assert_eq!(crate::mod_api::local_sound_caller(), caller);
        apply(&mut host, state.take());
        assert_eq!(host.audio.queued_audio_stream(2, other), Some(32));
        host.audio.end_fixed_audio_mix();
    }

    #[test]
    fn cleanup_resets_only_sprites_still_owned_by_unloading_mod() {
        #[derive(Default)]
        struct SpriteHost {
            item_resets: Vec<String>,
            block_resets: Vec<String>,
        }

        impl Host for SpriteHost {
            fn item_sprite_reset(&mut self, item_id: &str) -> Result<(), String> {
                self.item_resets.push(item_id.to_owned());
                Ok(())
            }

            fn block_sprite_reset(&mut self, texture_id: &str) -> Result<(), String> {
                self.block_resets.push(texture_id.to_owned());
                Ok(())
            }
        }

        let mut state = Registry::default();
        let gen_a = state.register(1);
        let gen_b = state.register(2);
        state
            .item_sprites
            .insert("minecraft:diamond".into(), (1, gen_a));
        state
            .item_sprites
            .insert("minecraft:apple".into(), (1, gen_a));
        // Mod B overwrites diamond after Mod A.
        state
            .item_sprites
            .insert("minecraft:diamond".into(), (2, gen_b));
        state
            .block_sprites
            .insert("mymod:block/pan".into(), (1, gen_a));

        let mut host = SpriteHost::default();
        state.release(cleanup(1, gen_a));
        apply(&mut host, state.take());
        assert_eq!(host.item_resets, ["minecraft:apple"]);
        assert_eq!(host.block_resets, ["mymod:block/pan"]);

        // Now Mod B unloads and resets diamond cleanly.
        state.release(cleanup(2, gen_b));
        apply(&mut host, state.take());
        assert_eq!(host.item_resets, ["minecraft:apple", "minecraft:diamond"]);
    }
}
