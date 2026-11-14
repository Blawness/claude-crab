//! Two deliberately different sounds per style: "needs you" is high and urgent, "finished" is
//! lower and resolves upward. The synthesized styles are built in memory so nothing depends on
//! files; the "windows" style plays the system sounds the user picked in Windows settings.

use std::f32::consts::TAU;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ALIAS, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 22_050;
const VOLUME: f32 = 0.55;

#[derive(Clone, Copy)]
pub enum Cue {
    NeedsInput,
    Done,
}

/// Style ids as saved in settings, with their menu labels. "windows" stays last: every style
/// before it is synthesized, and its index picks the cached buffer.
pub const STYLES: [(&str, &str); 7] = [
    ("chime", "Chime"),
    ("glass", "Glass (soft)"),
    ("marimba", "Marimba (warm)"),
    ("harp", "Harp"),
    ("bell", "Bell (louder)"),
    ("arcade", "Arcade (8-bit)"),
    ("windows", "Windows sounds"),
];
const SYNTH_STYLES: usize = STYLES.len() - 1;
static STYLE: AtomicUsize = AtomicUsize::new(0);

pub fn style() -> &'static str {
    STYLES[STYLE.load(Ordering::Relaxed)].0
}
pub fn set_style(name: &str) -> bool {
    match STYLES.iter().position(|(id, _)| *id == name) {
        Some(i) => {
            STYLE.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

#[derive(Clone, Copy)]
enum Wave {
    /// Sine with a little second harmonic; `ring` decays like a struck chime.
    Soft { ring: bool },
    /// A bell: long decay and an inharmonic partial, louder than the chime.
    Bell,
    /// A square wave, like an old game console.
    Square,
    /// Pure and glassy: slow attack, long shimmer.
    Glass,
    /// Warm and woody: a bright partial that dies away fast over a soft body.
    Marimba,
    /// Plucked string: quick attack, gentle decay.
    Harp,
}

impl Wave {
    /// How long a note keeps ringing past its slot, overlapping the next ones like a chord.
    /// Zero for the original styles, so they sound exactly as before.
    fn tail_ms(self) -> u32 {
        match self {
            Wave::Glass => 900,
            Wave::Marimba => 350,
            Wave::Harp => 700,
            _ => 0,
        }
    }
}

/// One note: frequency in Hz, length in ms, silence after it in ms.
type Note = (f32, u32, u32);

fn notes(style: usize, cue: Cue) -> (Wave, &'static [Note]) {
    match (STYLES[style].0, cue) {
        // Glass: a rising fourth, twice; finished is a shimmering Cmaj9.
        ("glass", Cue::NeedsInput) => (Wave::Glass, &[(880.0, 150, 60), (1175.0, 300, 260), (880.0, 150, 60), (1175.0, 300, 0)]),
        ("glass", Cue::Done) => (Wave::Glass, &[(523.0, 170, 0), (659.0, 170, 0), (988.0, 170, 0), (1175.0, 400, 0)]),
        // Marimba: three quick rising notes, twice; finished settles on a warm G-C-E.
        ("marimba", Cue::NeedsInput) => (Wave::Marimba, &[(1047.0, 90, 40), (1319.0, 90, 40), (1568.0, 120, 220), (1047.0, 90, 40), (1319.0, 90, 40), (1568.0, 160, 0)]),
        ("marimba", Cue::Done) => (Wave::Marimba, &[(392.0, 130, 20), (523.0, 130, 20), (659.0, 260, 0)]),
        // Harp: a bright upward run, twice; finished is a full C major arpeggio.
        ("harp", Cue::NeedsInput) => (Wave::Harp, &[(784.0, 80, 0), (988.0, 80, 0), (1175.0, 80, 0), (1568.0, 160, 240), (784.0, 80, 0), (988.0, 80, 0), (1175.0, 80, 0), (1568.0, 220, 0)]),
        ("harp", Cue::Done) => (Wave::Harp, &[(523.0, 85, 0), (659.0, 85, 0), (784.0, 85, 0), (988.0, 85, 0), (1047.0, 300, 0)]),
        _ => old_notes(style, cue),
    }
}

/// The original three styles, unchanged.
fn old_notes(style: usize, cue: Cue) -> (Wave, &'static [Note]) {
    let style = match STYLES[style].0 {
        "chime" => 0,
        "bell" => 1,
        _ => 2,
    };
    match (style, cue) {
        // Three quick rising beeps, played twice. Hard to miss.
        (0, Cue::NeedsInput) => (Wave::Soft { ring: false }, &[(988.0, 85, 45), (1319.0, 85, 45), (1760.0, 150, 170), (988.0, 85, 45), (1319.0, 85, 45), (1760.0, 190, 0)]),
        // A soft three-note chime that settles on a long final note.
        (0, Cue::Done) => (Wave::Soft { ring: true }, &[(523.0, 150, 15), (659.0, 150, 15), (784.0, 520, 0)]),
        (1, Cue::NeedsInput) => (Wave::Bell, &[(1047.0, 260, 60), (1047.0, 260, 60), (1397.0, 700, 0)]),
        (1, Cue::Done) => (Wave::Bell, &[(659.0, 220, 30), (784.0, 220, 30), (1047.0, 1100, 0)]),
        (_, Cue::NeedsInput) => (Wave::Square, &[(880.0, 70, 30), (880.0, 70, 30), (1175.0, 70, 30), (1175.0, 70, 120), (880.0, 70, 30), (1175.0, 160, 0)]),
        (_, Cue::Done) => (Wave::Square, &[(523.0, 90, 10), (659.0, 90, 10), (784.0, 90, 10), (1047.0, 320, 0)]),
    }
}

fn wav(wave: Wave, notes: &[Note]) -> Vec<u8> {
    // Notes are mixed, not appended, so a ringing note can carry on under the next one.
    let tail = wave.tail_ms();
    let mut mix: Vec<f32> = Vec::new();
    let mut cursor = 0usize;
    for &(freq, ms, gap) in notes {
        let slot = (RATE * ms / 1000) as usize;
        let count = (RATE * (ms + tail) / 1000) as usize;
        if mix.len() < cursor + count {
            mix.resize(cursor + count, 0.0);
        }
        for i in 0..count {
            let t = i as f32 / RATE as f32;
            let progress = i as f32 / count as f32;
            // Short fade-in avoids a click; glass swells in more slowly.
            let rise = if matches!(wave, Wave::Glass) { 0.012 } else { 0.004 };
            let attack = (i as f32 / (RATE as f32 * rise)).min(1.0);
            let flat = ((1.0 - progress) * 12.0).min(1.0); // holds, then cuts off softly
            let (sample, shape, gain) = match wave {
                Wave::Glass => (
                    (TAU * freq * t).sin() + 0.3 * (TAU * freq * 2.0 * t).sin() + 0.08 * (TAU * freq * 3.0 * t).sin(),
                    (-3.0 * progress).exp(),
                    0.42,
                ),
                Wave::Marimba => (
                    (TAU * freq * t).sin() + 0.35 * (-40.0 * t).exp() * (TAU * freq * 4.0 * t).sin(),
                    (-6.0 * progress).exp(),
                    0.6,
                ),
                Wave::Harp => (
                    (TAU * freq * t).sin() + 0.45 * (TAU * freq * 2.0 * t).sin() * (-8.0 * t).exp() + 0.15 * (TAU * freq * 3.0 * t).sin() * (-14.0 * t).exp(),
                    (-4.5 * progress).exp(),
                    0.45,
                ),
                Wave::Soft { ring } => (
                    (TAU * freq * t).sin() + if ring { 0.25 } else { 0.4 } * (TAU * freq * 2.0 * t).sin(),
                    if ring { (-3.2 * progress).exp() } else { flat },
                    0.7,
                ),
                Wave::Bell => (
                    (TAU * freq * t).sin() + 0.5 * (TAU * freq * 2.76 * t).sin() + 0.25 * (TAU * freq * 5.4 * t).sin(),
                    (-2.4 * progress).exp(),
                    0.85,
                ),
                Wave::Square => ((TAU * freq * t).sin().signum(), flat, 0.35),
            };
            mix[cursor + i] += sample * attack * shape * VOLUME * gain;
        }
        cursor += slot + (RATE * gap / 1000) as usize;
        if mix.len() < cursor {
            mix.resize(cursor, 0.0);
        }
    }
    // The ringing styles get a soft room echo: two quieter repeats.
    if tail > 0 {
        let len = mix.len();
        mix.resize(len + (RATE as usize * 3) / 10, 0.0);
        for (delay_ms, level) in [(130usize, 0.28f32), (260, 0.12)] {
            let d = RATE as usize * delay_ms / 1000;
            for i in (0..len).rev() {
                mix[i + d] += mix[i] * level;
            }
        }
    }
    // Overlapping notes can add up past full scale; scale down instead of clipping.
    let peak = mix.iter().fold(0f32, |m, s| m.max(s.abs()));
    let scale = if peak > 0.95 { 0.95 / peak } else { 1.0 };
    let pcm: Vec<i16> = mix.iter().map(|s| (s * scale * i16::MAX as f32) as i16).collect();

    let data_len = (pcm.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 2).to_le_bytes()); // bytes per second
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in pcm {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

pub fn play(cue: Cue) {
    let style = STYLE.load(Ordering::Relaxed);
    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    if STYLES[style].0 == "windows" {
        // The sounds set under Windows Settings > Sound for these two events.
        let alias = wide(match cue {
            Cue::NeedsInput => "SystemExclamation",
            Cue::Done => "SystemAsterisk",
        });
        unsafe { PlaySoundW(alias.as_ptr(), std::ptr::null_mut(), SND_ALIAS | SND_ASYNC | SND_NODEFAULT) };
        return;
    }
    // One buffer per style and cue, built once. They live for the whole run: an async sound
    // keeps reading its memory after this returns.
    static BUFFERS: [OnceLock<Vec<u8>>; SYNTH_STYLES * 2] = [const { OnceLock::new() }; SYNTH_STYLES * 2];
    let slot = style * 2 + matches!(cue, Cue::Done) as usize;
    let buffer = BUFFERS[slot].get_or_init(|| {
        let (wave, notes) = notes(style, cue);
        wav(wave, notes)
    });
    unsafe {
        PlaySoundW(buffer.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_synth_style_builds_a_wav_for_both_cues() {
        assert_eq!(STYLES[SYNTH_STYLES].0, "windows");
        for style in 0..SYNTH_STYLES {
            for cue in [Cue::NeedsInput, Cue::Done] {
                let (wave, notes) = notes(style, cue);
                let out = wav(wave, notes);
                assert_eq!(&out[..4], b"RIFF");
                assert!(out.len() > 44 + 2000, "style {style} is too short");
                let loudest = out[44..].chunks(2).map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs()).max().unwrap();
                assert!(loudest < i16::MAX as u16 && loudest > 3000, "style {style}: peak {loudest} clips or is near silent");
            }
        }
        // CRAB_SOUND_DIR=<folder> writes every sound as a .wav there, to listen without the app.
        if let Some(dir) = std::env::var_os("CRAB_SOUND_DIR") {
            for style in 0..SYNTH_STYLES {
                for (cue, name) in [(Cue::Done, "finished"), (Cue::NeedsInput, "needs-you")] {
                    let (wave, notes) = notes(style, cue);
                    std::fs::write(std::path::Path::new(&dir).join(format!("{}-{name}.wav", STYLES[style].0)), wav(wave, notes)).unwrap();
                }
            }
        }
        assert!(set_style("arcade") && style() == "arcade");
        assert!(!set_style("nope") && style() == "arcade");
    }
}
