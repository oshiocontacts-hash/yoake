//! Client PCM streams (@API:positional_pcm_streams@) and opt-in capture (@API:microphone_input@).

use std::collections::HashMap;

use horizon_client::{
    audio::NativeSoundError,
    mod_api,
    native_audio_stream::{NativeAudioStreamId, NativeAudioStreamLimits, NativeStreamAnchor},
    native_mic::{NativeMicId, NativeMicrophones},
};

use super::entity::{Entity as _, entity_lists_of, for_each_entity};
use super::{Game, NativeSoundCategory};

struct FollowingStream {
    owner: usize,
    anchor: NativeStreamAnchor,
}

#[derive(Default)]
pub(super) struct NativeStreams {
    streams: HashMap<NativeAudioStreamId, FollowingStream>,
    microphones: Option<NativeMicrophones>,
}

fn stream_location(
    anchor: NativeStreamAnchor,
    entity_position: impl FnOnce(u32) -> Option<[f64; 3]>,
) -> (Option<[f64; 3]>, bool) {
    match anchor {
        NativeStreamAnchor::Entity(id) => (entity_position(id), false),
        NativeStreamAnchor::Position(position) => (Some(position), false),
        NativeStreamAnchor::Listener => (Some([0.0; 3]), true),
    }
}

fn sound_source(category: NativeSoundCategory) -> usize {
    match category {
        NativeSoundCategory::Master => 0,
        NativeSoundCategory::Music => 1,
        NativeSoundCategory::Records => 2,
        NativeSoundCategory::Weather => 3,
        NativeSoundCategory::Blocks => 4,
        NativeSoundCategory::Hostile => 5,
        NativeSoundCategory::Neutral => 6,
        NativeSoundCategory::Players => 7,
        NativeSoundCategory::Ambient => 8,
        NativeSoundCategory::Voice => 9,
        NativeSoundCategory::Ui => 10,
    }
}

impl Game<'_> {
    fn stream_caller(&self) -> Result<usize, NativeSoundError> {
        let owner = mod_api::local_sound_caller().ok_or(NativeSoundError::OutsideClientTick)?;
        if !self.options.execution_role.is_client() {
            return Err(NativeSoundError::OutsideClientTick);
        }
        if !self.stream_world_open() {
            return Err(NativeSoundError::WorldUnavailable);
        }
        Ok(owner)
    }

    fn stream_world_open(&self) -> bool {
        self.options.execution_role.is_client()
            && self.player.is_some()
            && self.remote.as_ref().is_some_and(|remote| remote.logged_in)
    }

    pub(super) fn streams_limits(&self) -> NativeAudioStreamLimits {
        self.audio.audio_stream_limits()
    }

    pub(super) fn streams_open(
        &mut self,
        anchor: NativeStreamAnchor,
        sample_rate: u32,
        channels: u16,
        category: NativeSoundCategory,
    ) -> Result<NativeAudioStreamId, NativeSoundError> {
        let owner = self.stream_caller()?;
        let (position, relative) = stream_location(anchor, |id| {
            self.stream_entity_position(id, self.clock.fraction(), self.clock.camera_fraction())
        });
        let id = self.audio.open_audio_stream(
            owner,
            sample_rate,
            channels,
            sound_source(category),
            position,
            relative,
        )?;
        self.native_streams
            .get_or_insert_with(NativeStreams::default)
            .streams
            .insert(id, FollowingStream { owner, anchor });
        Ok(id)
    }

    pub(super) fn streams_push(
        &mut self,
        id: NativeAudioStreamId,
        samples: &[f32],
    ) -> Result<usize, NativeSoundError> {
        let owner = self.stream_caller()?;
        self.audio.push_audio_stream(owner, id, samples)
    }

    pub(super) fn streams_push_i16(
        &mut self,
        id: NativeAudioStreamId,
        samples: &[i16],
    ) -> Result<usize, NativeSoundError> {
        let owner = self.stream_caller()?;
        self.audio.push_audio_stream_i16(owner, id, samples)
    }

    pub(super) fn streams_queued(&mut self, id: NativeAudioStreamId) -> Option<usize> {
        let owner = self.stream_caller().ok()?;
        self.audio.queued_audio_stream(owner, id)
    }

    pub(super) fn streams_dropped(&mut self, id: NativeAudioStreamId) -> Option<u64> {
        let owner = self.stream_caller().ok()?;
        self.audio.dropped_audio_stream(owner, id)
    }

    pub(super) fn streams_move(
        &mut self,
        id: NativeAudioStreamId,
        anchor: NativeStreamAnchor,
    ) -> Result<(), NativeSoundError> {
        let owner = self.stream_caller()?;
        let (position, relative) = stream_location(anchor, |entity_id| {
            self.stream_entity_position(
                entity_id,
                self.clock.fraction(),
                self.clock.camera_fraction(),
            )
        });
        self.audio
            .place_audio_stream(owner, id, position, relative)?;
        if let Some(stream) = self
            .native_streams
            .as_mut()
            .and_then(|state| state.streams.get_mut(&id))
        {
            stream.anchor = anchor;
        }
        Ok(())
    }

    pub(super) fn streams_close(&mut self, id: NativeAudioStreamId) -> bool {
        let Ok(owner) = self.stream_caller() else {
            return false;
        };
        if !self.audio.close_audio_stream(owner, id) {
            return false;
        }
        if let Some(state) = self.native_streams.as_mut() {
            state.streams.remove(&id);
        }
        self.prune_native_streams();
        true
    }

    pub(super) fn mic_devices(&self) -> Vec<String> {
        if !self.options.execution_role.is_client() {
            return Vec::new();
        }
        NativeMicrophones::devices()
    }

    pub(super) fn mic_open(
        &mut self,
        device: Option<&str>,
        sample_rate: u32,
    ) -> Result<NativeMicId, String> {
        let owner = self.stream_caller().map_err(|error| error.to_string())?;
        let state = self
            .native_streams
            .get_or_insert_with(NativeStreams::default);
        let result = state
            .microphones
            .get_or_insert_with(NativeMicrophones::default)
            .open(owner, device, sample_rate);
        self.prune_native_streams();
        result
    }

    pub(super) fn mic_read(&mut self, id: NativeMicId, output: &mut [f32]) -> usize {
        let Ok(owner) = self.stream_caller() else {
            return 0;
        };
        self.native_streams
            .as_mut()
            .and_then(|state| state.microphones.as_mut())
            .map_or(0, |microphones| microphones.read(owner, id, output))
    }

    pub(super) fn mic_close(&mut self, id: NativeMicId) -> bool {
        let Ok(owner) = self.stream_caller() else {
            return false;
        };
        let closed = self
            .native_streams
            .as_mut()
            .and_then(|state| state.microphones.as_mut())
            .is_some_and(|microphones| microphones.close(owner, id));
        self.prune_native_streams();
        closed
    }

    fn prune_native_streams(&mut self) {
        if let Some(state) = self.native_streams.as_mut() {
            if state
                .microphones
                .as_ref()
                .is_some_and(NativeMicrophones::is_empty)
            {
                state.microphones = None;
            }
            if state.streams.is_empty() && state.microphones.is_none() {
                self.native_streams = None;
            }
        }
    }

    pub(super) fn update_native_streams(&mut self, alpha: f64, camera_alpha: f64) {
        if self.native_streams.is_none() {
            return;
        }
        if !self.stream_world_open() {
            self.clear_native_streams();
            return;
        }
        let Some(mut state) = self.native_streams.take() else {
            return;
        };
        state.streams.retain(|id, stream| {
            let (position, relative) = stream_location(stream.anchor, |entity_id| {
                self.stream_entity_position(entity_id, alpha, camera_alpha)
            });
            if self
                .audio
                .place_audio_stream(stream.owner, *id, position, relative)
                .is_ok()
            {
                true
            } else {
                self.audio.close_audio_stream(stream.owner, *id);
                false
            }
        });
        self.native_streams = Some(state);
        self.prune_native_streams();
    }

    pub(super) fn maintain_native_streams(&mut self) {
        if self.native_streams.is_none() {
            return;
        }
        if !self.stream_world_open() {
            self.clear_native_streams();
            return;
        }
        let statuses = crate::mod_guard::mod_statuses();
        let enabled = |owner| {
            statuses
                .iter()
                .any(|status| status.index == owner && status.enabled)
        };
        if let Some(state) = self.native_streams.as_mut() {
            state.streams.retain(|id, stream| {
                if enabled(stream.owner) {
                    true
                } else {
                    self.audio.close_audio_stream(stream.owner, *id);
                    false
                }
            });
            if let Some(microphones) = state.microphones.as_mut() {
                microphones.retain_owners(enabled);
            }
        }
        self.prune_native_streams();
    }

    pub(super) fn clear_native_streams(&mut self) {
        self.audio.clear_audio_streams();
        self.native_streams = None;
    }

    fn stream_entity_position(&self, id: u32, alpha: f64, camera_alpha: f64) -> Option<[f64; 3]> {
        if self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.entity_id as u32 == id)
        {
            return self.player.as_ref().map(|player| {
                let at = self.previous_position.lerp(player.position, camera_alpha);
                [at.x, at.y, at.z]
            });
        }
        let mut matched = false;
        let mut position = None;
        for_each_entity!(entity_lists_of!(self), entity => {
            if entity.id() == id {
                matched = true;
                if !entity.native_query_removed() {
                    let at = entity.previous.lerp(entity.position, alpha);
                    position = Some([at.x, at.y, at.z]);
                }
            }
        });
        if matched {
            return position;
        }
        for guest in self.lan.as_ref().map_or(&[][..], |host| &host.hub.guests) {
            if guest.entity_id as u32 == id && guest.placed && guest.dimension == self.dim.id() {
                return Some(std::array::from_fn(|axis| {
                    guest.previous_position[axis]
                        + (guest.position[axis] - guest.previous_position[axis]) * alpha
                }));
            }
        }
        if let Some(dragon) = self.dragon.as_ref().filter(|dragon| dragon.id == id) {
            let at = dragon.previous.lerp(dragon.position, alpha);
            return Some([at.x, at.y, at.z]);
        }
        if let Some(fireball) = self
            .dragon_fireballs
            .iter()
            .find(|fireball| fireball.id == id)
        {
            let at = fireball.previous.lerp(fireball.position, alpha);
            return Some([at.x, at.y, at.z]);
        }
        if let Some(position) = self.potions.native_stream_position(id, alpha) {
            return Some(position);
        }
        for ((eye_id, _), (previous, current)) in
            self.end.eye_positions().zip(self.end.eye_motions())
        {
            if eye_id == id {
                let at = previous.lerp(current, alpha);
                return Some([at.x, at.y, at.z]);
            }
        }
        if let Some(hook) = self.hook.as_ref().filter(|hook| hook.id() == id) {
            let at = hook.previous.lerp(hook.position, alpha);
            return Some([at.x, at.y, at.z]);
        }
        if let Some(hook) = self.remote_hooks.iter().find(|hook| hook.id() == id) {
            let at = hook.previous.lerp(hook.position, alpha);
            return Some([at.x, at.y, at.z]);
        }
        if let Some(display) = self.text_displays.get(&(id as i32)) {
            let (current, previous) = display.seat();
            return Some(previous.lerp(current, alpha).to_array());
        }
        if let Some(interaction) = self.interactions.get(&(id as i32)) {
            let at = interaction.position;
            return Some([at.x, at.y, at.z]);
        }
        if let Some(cushion) = self.cushions.iter().find(|cushion| cushion.id == id) {
            let at = cushion.position;
            return Some([at.x, at.y, at.z]);
        }
        if let Some(bolt) = self.bolts.iter().find(|bolt| bolt.id == id) {
            return Some(bolt.at);
        }
        if let Some(position) = self.remote_knots.get(&id) {
            return Some(position.map(|coordinate| coordinate as f64 + 0.5));
        }
        self.level_entity(id as i32)
            .and_then(|target| self.target_position(target))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_mod_allocates_no_stream_or_microphone_registry() {
        let state = NativeStreams::default();
        assert!(state.streams.is_empty());
        assert_eq!(state.streams.capacity(), 0);
        assert!(state.microphones.is_none());
    }

    #[test]
    fn listener_and_fixed_positions_do_not_query_entities() {
        assert_eq!(
            stream_location(NativeStreamAnchor::Listener, |_| panic!("entity lookup")),
            (Some([0.0; 3]), true)
        );
        assert_eq!(
            stream_location(NativeStreamAnchor::Position([3.0, 4.0, 5.0]), |_| {
                panic!("entity lookup")
            }),
            (Some([3.0, 4.0, 5.0]), false)
        );
    }

    #[test]
    fn entity_anchor_follows_interpolation_and_survives_unload() {
        let anchor = NativeStreamAnchor::Entity(17);
        for alpha in [0.0, 0.25, 0.75, 1.0] {
            let expected = [2.0 + 8.0 * alpha, 4.0, 6.0];
            assert_eq!(
                stream_location(anchor, |id| {
                    assert_eq!(id, 17);
                    Some(expected)
                }),
                (Some(expected), false)
            );
        }
        assert_eq!(stream_location(anchor, |_| None), (None, false));
        assert_eq!(anchor, NativeStreamAnchor::Entity(17));
        assert_eq!(
            stream_location(anchor, |_| Some([10.0, 4.0, 6.0])),
            (Some([10.0, 4.0, 6.0]), false)
        );
    }

    #[test]
    fn voice_uses_the_existing_voice_slider() {
        assert_eq!(sound_source(NativeSoundCategory::Voice), 9);
        assert_eq!(sound_source(NativeSoundCategory::Master), 0);
        assert_eq!(sound_source(NativeSoundCategory::Ui), 10);
    }
}
