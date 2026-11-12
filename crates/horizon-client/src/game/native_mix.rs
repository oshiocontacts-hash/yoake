//! Game-thread notification of a listened final-output copy ring.

use super::Game;

impl Game<'_> {
    pub(super) fn notify_native_audio_mix(&mut self) {
        if self.native_mix_cursors.is_some() {
            self.native_audio_mix_ready();
        }
    }
}
