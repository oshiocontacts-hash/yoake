//! Finite, client-local sound buffers owned by the dispatching native mod.

use std::collections::HashMap;

use super::Game;
use horizon_client::{
    audio::{
        NativePlaybackHandle, NativeSoundError, NativeSoundHandle, NativeSoundLimits,
        NativeSoundRegistration, NativeSoundTarget,
    },
    mod_api,
};

struct FollowingSound {
    owner: usize,
    playback: NativePlaybackHandle,
    sound: NativeSoundHandle,
    target: NativeSoundTarget,
    entity_id: u32,
    incarnation: u64,
}

#[derive(Default)]
pub(super) struct NativeAudio {
    following: Vec<FollowingSound>,
    incarnations: HashMap<u32, u64>,
    next_incarnation: u64,
}

impl NativeAudio {
    fn incarnation(&mut self, entity_id: u32) -> Result<u64, NativeSoundError> {
        if let Some(incarnation) = self.incarnations.get(&entity_id) {
            return Ok(*incarnation);
        }
        self.next_incarnation = self
            .next_incarnation
            .checked_add(1)
            .ok_or(NativeSoundError::MissingTarget)?;
        self.incarnations.insert(entity_id, self.next_incarnation);
        Ok(self.next_incarnation)
    }

    fn prune_incarnations(&mut self) {
        self.incarnations.retain(|entity_id, _| {
            self.following
                .iter()
                .any(|sound| sound.entity_id == *entity_id)
        });
    }
}

impl Game<'_> {
    fn local_sound_owner(&self) -> Result<usize, NativeSoundError> {
        let owner = mod_api::local_sound_caller().ok_or(NativeSoundError::OutsideClientTick)?;
        if !self.options.execution_role.is_client() {
            return Err(NativeSoundError::OutsideClientTick);
        }
        if self.player.is_none() || !self.remote.as_ref().is_some_and(|remote| remote.logged_in) {
            return Err(NativeSoundError::WorldUnavailable);
        }
        Ok(owner)
    }

    /// API @API:dynamic_local_sound@: process-wide limits, including all split-screen clients.
    /// This read-only query is also available outside a client tick.
    pub(super) fn native_local_sound_limits(&self) -> NativeSoundLimits {
        self.audio.local_sound_limits()
    }

    /// API @API:dynamic_local_sound@: copy signed 16-bit interleaved PCM into an owned buffer.
    /// Mono or stereo, 8,000..=192,000 Hz; stereo is averaged to mono. The cap is
    /// 8 MiB of decoded mono f32 samples (2,097,152 frames), not the input byte count.
    pub(super) fn native_register_local_sound_pcm(
        &mut self,
        samples: &[i16],
        channels: u8,
        sample_rate: u32,
    ) -> Result<NativeSoundHandle, NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio
            .register_local_sound_pcm(owner, samples, channels, sample_rate)
    }

    /// API @API:dynamic_local_sound@: copy at most 8 MiB of Ogg/Vorbis into a bounded
    /// mod-only worker queue. Validation and decoding are asynchronous; poll the ticket.
    pub(super) fn native_begin_local_sound_ogg(
        &mut self,
        bytes: &[u8],
    ) -> Result<NativeSoundRegistration, NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio.begin_local_sound_ogg(owner, bytes)
    }

    /// API @API:dynamic_local_sound@: None means pending. A ready result or error consumes
    /// the ticket; subsequent polls return StaleHandle. No decode work runs on this call.
    pub(super) fn native_poll_local_sound_registration(
        &mut self,
        registration: NativeSoundRegistration,
    ) -> Result<Option<NativeSoundHandle>, NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio
            .poll_local_sound_registration(owner, registration)
    }

    /// API @API:dynamic_local_sound@: consume a registration ticket and discard its result.
    /// An already-running decode retains its budget until the worker releases its buffers.
    pub(super) fn native_cancel_local_sound_registration(
        &mut self,
        registration: NativeSoundRegistration,
    ) -> Result<(), NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio
            .cancel_local_sound_registration(owner, registration)
    }

    /// API @API:dynamic_local_sound@: play privately on this client in the player category,
    /// subject to both mod quotas and the ordinary static channel pool. Following targets
    /// stop on removal, replacement or a level change; this never sends/captures network audio.
    pub(super) fn native_play_registered_local_sound(
        &mut self,
        sound: NativeSoundHandle,
        target: NativeSoundTarget,
        volume: f32,
        pitch: f32,
        looping: bool,
    ) -> Result<NativePlaybackHandle, NativeSoundError> {
        let owner = self.local_sound_owner()?;
        let (position, following) = match target {
            NativeSoundTarget::Position(position) => (position, None),
            _ => {
                let (entity_id, position) = self
                    .local_sound_target(target)
                    .ok_or(NativeSoundError::MissingTarget)?;
                (position, Some(entity_id))
            }
        };
        let playback = self
            .audio
            .play_registered_local_sound(owner, sound, position, volume, pitch, looping)?;
        if let Some(entity_id) = following {
            let state = self.native_audio.get_or_insert_with(NativeAudio::default);
            let incarnation = match state.incarnation(entity_id) {
                Ok(incarnation) => incarnation,
                Err(error) => {
                    let _ = self.audio.stop_local_playback(owner, playback);
                    return Err(error);
                }
            };
            state.following.push(FollowingSound {
                owner,
                playback,
                sound,
                target,
                entity_id,
                incarnation,
            });
        }
        Ok(playback)
    }

    /// API @API:dynamic_local_sound@: stop and consume an owned playback handle.
    pub(super) fn native_stop_local_playback(
        &mut self,
        playback: NativePlaybackHandle,
    ) -> Result<(), NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio.stop_local_playback(owner, playback)?;
        if let Some(state) = &mut self.native_audio {
            state.following.retain(|sound| sound.playback != playback);
            state.prune_incarnations();
        }
        Ok(())
    }

    /// API @API:dynamic_local_sound@: unregister an owned buffer and stop all its playbacks.
    /// Cancel an unfinished registration by its ticket instead.
    pub(super) fn native_unregister_local_sound(
        &mut self,
        sound: NativeSoundHandle,
    ) -> Result<(), NativeSoundError> {
        let owner = self.local_sound_owner()?;
        self.audio.unregister_local_sound(owner, sound)?;
        if let Some(state) = &mut self.native_audio {
            state.following.retain(|following| following.sound != sound);
            state.prune_incarnations();
        }
        Ok(())
    }

    fn local_sound_target(&self, target: NativeSoundTarget) -> Option<(u32, [f64; 3])> {
        let remote = self.remote.as_ref().filter(|remote| remote.logged_in)?;
        let entity_id = match target {
            NativeSoundTarget::Position(_) => return None,
            NativeSoundTarget::Player(uuid) if uuid == remote.profile() => remote.entity_id as u32,
            NativeSoundTarget::Player(uuid) => remote.player_entity(uuid)?,
            NativeSoundTarget::Entity(entity_id) => entity_id,
        };
        let position = if entity_id == remote.entity_id as u32 {
            let player = self.player.as_ref()?;
            [player.position.x, player.position.y, player.position.z]
        } else {
            remote.tracked(entity_id as i32)?.1
        };
        Some((entity_id, position))
    }

    pub(super) fn tick_native_local_sounds(&mut self) {
        let Some(mut state) = self.native_audio.take() else {
            return;
        };
        state.following.retain(|sound| {
            let target = self
                .local_sound_target(sound.target)
                .filter(|(entity_id, _)| {
                    *entity_id == sound.entity_id
                        && state.incarnations.get(entity_id) == Some(&sound.incarnation)
                });
            if let Some((_, position)) = target {
                if self
                    .audio
                    .move_local_playback(sound.owner, sound.playback, position)
                {
                    return true;
                }
            }
            let _ = self.audio.stop_local_playback(sound.owner, sound.playback);
            false
        });
        state.prune_incarnations();
        if !state.following.is_empty() {
            self.native_audio = Some(state);
        }
    }

    pub(super) fn invalidate_native_local_sound_entity(&mut self, entity_id: i32) {
        let Some(state) = &mut self.native_audio else {
            return;
        };
        let entity_id = entity_id as u32;
        state.incarnations.remove(&entity_id);
        state.following.retain(|sound| {
            if sound.entity_id != entity_id {
                return true;
            }
            let _ = self.audio.stop_local_playback(sound.owner, sound.playback);
            false
        });
    }

    pub(super) fn clear_native_local_sounds(&mut self) {
        self.audio.clear_local_sounds();
        self.native_audio = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_sounds_distinguish_reused_entity_ids() {
        let mut state = NativeAudio::default();
        let original = state.incarnation(7).unwrap();
        assert_eq!(state.incarnation(7), Ok(original));
        assert_ne!(state.incarnation(8), Ok(original));
        state.incarnations.remove(&7);
        let replacement = state.incarnation(7).unwrap();
        assert!(replacement > original);
        assert_eq!(state.incarnation(7), Ok(replacement));
    }

    #[test]
    fn incarnation_tracking_is_pruned_and_never_wraps() {
        let mut state = NativeAudio::default();
        state.incarnation(7).unwrap();
        state.prune_incarnations();
        assert!(state.incarnations.is_empty());
        state.next_incarnation = u64::MAX;
        assert_eq!(state.incarnation(7), Err(NativeSoundError::MissingTarget));
        assert!(state.incarnations.is_empty());
        assert_eq!(state.next_incarnation, u64::MAX);
    }
}
