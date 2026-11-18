use alloc::sync::Arc;

use crate::system::ma2::{Ma2Event, Ma2Voice};

pub trait AudioSink: Sync + Send {
    fn play_wave(&self, channel: u8, sampling_rate: u32, wave_data: &[i16]);
    fn midi_note_on(&self, channel_id: u8, note: u8, velocity: u8);
    fn midi_note_off(&self, channel_id: u8, note: u8, velocity: u8);
    fn midi_program_change(&self, channel_id: u8, program: u8);
    fn midi_control_change(&self, channel_id: u8, control: u8, value: u8);
    fn midi_pitch_bend(&self, channel_id: u8, value: u16);
    fn midi_sysex(&self, data: &[u8]);

    /// Silence a MIDI channel NOW — every voice on it stops, release tails included.
    ///
    /// ★ 2026-09-25 (hero2: "맵이 바뀔때 배경음악이 겹쳐지는 버그"). A plain
    /// `midi_control_change(ch, 120, 0)` is not that on every synth: oxisynth routes
    /// control 120 arriving as a control change to its all-NOTES-off, which releases each
    /// voice into its envelope's release — seconds long on GeneralUser's pads — so the old
    /// song kept fading under the new one. Only its dedicated all-sound-off event is a hard
    /// stop. [`crate::system::audio`] calls this when one song REPLACES another (or a
    /// playback is stopped outright), which is what a phone's MIDI player does on
    /// `Player.stop`. The default keeps the old behaviour for sinks that forward MIDI to an
    /// external synth, where control 120 is already the standard all-sound-off.
    fn midi_all_sound_off(&self, channel_id: u8) {
        self.midi_control_change(channel_id, 120, 0);
    }

    /// Set the level of the MUSIC (MIDI) bus, `0..=100`, as the guest's
    /// `MC_mdaClipSetVolume` gave it. Returns `true` if the sink applied it as an
    /// **output gain on rendered audio** — in which case [`SmafPlayer`] must NOT also
    /// scale note velocity, because that would apply the level twice.
    ///
    /// ★ 2026-08-16, and it is the fix for "배경음이 최대 볼륨에서도 작다" +
    /// "배경음 3단계와 효과음 3단계의 음량이 너무 차이난다".
    ///
    /// Hero3's two buses are physically different SMAF chunk types — BGM is `MTR`
    /// (a MIDI score, rendered by the host synth) and SFX is `ATR` (Yamaha ADPCM,
    /// played as PCM). The player used to apply the game's volume to each in its own
    /// domain: PCM was scaled LINEARLY on the samples, but MIDI was applied by
    /// multiplying NOTE VELOCITY. Those are not the same law and not even close:
    ///
    ///   * velocity -> loudness runs through the SoundFont's concave
    ///     velocity-to-attenuation modulator, so a 0.6x velocity is roughly a 0.36x
    ///     amplitude — twice the dB drop that 0.6x on PCM gives; and
    ///   * velocity also SELECTS the sample layer and moves the filter cutoff, so a
    ///     quieter setting made the music duller, not just softer.
    ///
    /// Measured on this game's own assets (`GeneralUser.sf3`, 20 s renders, both
    /// sliders at the default level 3): BGM landed at **-42 to -48 dBFS RMS** while
    /// SFX sat at **-8 to -24 dBFS RMS**. A 27-39 dB gap from one setting.
    ///
    /// With a host music bus the level is instead applied to the synth's OUTPUT with
    /// [`volume_gain`] — the same curve the PCM bus uses — so both buses answer to
    /// their slider identically and the notes keep their intended timbre.
    ///
    /// Sinks that hand MIDI to an external synth (`wie_cli`'s midir, `wie_web`'s
    /// WebMIDI) have no bus to scale, so they keep the default `false` and the old
    /// velocity path, unchanged.
    fn set_music_volume(&self, _volume: u8) -> bool {
        false
    }

    /// Whether this sink plays an MA-2 score — SMAF Handy Phone Standard, every hero2 song —
    /// with the FM voices the score itself carries, through the `fm_*` calls below instead of
    /// the General MIDI ones above.
    ///
    /// ★ 2026-10-01 (hero2, `HERO2_BGM_FM_PLAN.md` §3 B3). General MIDI is a lossy reading of
    /// such a score: its voices are dropped and each program number becomes a GM instrument
    /// (`HERO2_GUEST_RE.md` §8.37). Asked once per playback; the default `false` keeps the
    /// General MIDI path for every sink that has no FM synth, unchanged.
    fn fm_supported(&self) -> bool {
        false
    }

    /// A new score on FM part `part` (0 = the music, 1 = a MIDI effect played over it): its
    /// voices, and every channel of the part back to its defaults.
    fn fm_voices(&self, _part: u8, _voices: &[Ma2Voice]) {}

    /// One score event for a channel of FM part `part`, at the moment it is due.
    fn fm_event(&self, _part: u8, _event: Ma2Event) {}

    /// Silence FM part `part` NOW — every voice, release tails included. The FM counterpart of
    /// [`midi_all_sound_off`](Self::midi_all_sound_off), for the same reason.
    fn fm_all_off(&self, _part: u8) {}

    /// ★ heroZero fork (2026-10-22): start media stream `id` — an encoded file the SINK decodes
    /// and renders on its own thread (MP4/AAC, a Standard MIDI File), as `net.wie.MediaPlayer`
    /// hands over a non-WAV, non-SMAF player. One stream at a time, as a handset had one media
    /// decoder: a new `id` replaces whatever stream is playing. The same `id` again after
    /// [`media_stop`](Self::media_stop) resumes where it stopped (MIDP `start()` after `stop()`);
    /// after the stream ended it plays from the beginning. `loops` is the MIDP loop count (`-1`
    /// forever, `n > 0` passes in all); `level` is the `VolumeControl` level `0..=100`, muting
    /// already applied, for the sink to map through [`volume_gain`]. A format the sink cannot
    /// render still stops the stream it replaces. The default plays nothing.
    fn media_start(&self, _id: u32, _data: &Arc<[u8]>, _loops: i32, _level: u8) {}

    /// ★ heroZero fork (2026-10-22): pause stream `id` at its current position (MIDP `stop()`).
    /// Ignored if `id` is not the stream playing.
    fn media_stop(&self, _id: u32) {}

    /// ★ heroZero fork (2026-10-22): the `VolumeControl` level of stream `id`, live.
    fn media_level(&self, _id: u32, _level: u8) {}

    /// ★ heroZero fork (2026-10-22): stream `id` is gone (MIDP `close()`); stop it and free it.
    fn media_close(&self, _id: u32) {}
}

/// The one definition of "what does volume level N sound like", shared by both buses.
///
/// The guest sends `0..=100` (Hero3's settings are 0-5 steps, `level * 20`). A linear
/// map spends almost all of its range in the top step (-14/-8/-4.4/-1.9/0 dB), which is
/// why "3단계" of the PCM bus was still nearly full volume. Squaring it gives the taper
/// a 5-step slider wants — -28/-16/-8.9/-3.9/0 dB — while keeping 0 silent and 100 unity.
pub fn volume_gain(volume: u8) -> f32 {
    let v = volume.min(100) as f32 / 100.0;
    v * v
}

#[cfg(test)]
mod tests {
    use super::volume_gain;

    #[test]
    fn silent_at_zero_and_unity_at_full() {
        assert_eq!(volume_gain(0), 0.0);
        assert_eq!(volume_gain(100), 1.0);
        // The guest is clamped by `clip_set_volume`, but never trust that here.
        assert_eq!(volume_gain(255), 1.0);
    }

    #[test]
    fn hero3_five_steps_are_monotonic_and_evenly_tapered() {
        // level*20, i.e. the values `MC_mdaClipSetVolume` actually receives.
        let steps: [f32; 6] = [0, 20, 40, 60, 80, 100].map(volume_gain);
        for w in steps.windows(2) {
            assert!(w[1] > w[0], "levels must increase: {w:?}");
        }
        // Default setting (level 3) is a real, audible cut, not the ~-4 dB the old
        // linear PCM law gave.
        let db = 20.0 * steps[3].log10();
        assert!((db + 8.87).abs() < 0.01, "level 3 should be ~-8.9 dB, got {db}");
    }
}
