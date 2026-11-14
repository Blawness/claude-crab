//! Two deliberately different sounds, synthesized in memory so nothing depends on files:
//! "needs you" is high and staccato, "finished" is lower, slower and resolves upward.

use std::f32::consts::TAU;
use std::sync::OnceLock;

use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};

const RATE: u32 = 22_050;
const VOLUME: f32 = 0.55;

#[derive(Clone, Copy)]
pub enum Cue {
    NeedsInput,
    Done,
}

/// One note: frequency in Hz, length in ms, silence after it in ms, and whether it rings out.
type Note = (f32, u32, u32, bool);

/// Three quick rising beeps, played twice. Hard to miss.
const NEEDS_INPUT: [Note; 6] = [
    (988.0, 85, 45, false),
    (1319.0, 85, 45, false),
    (1760.0, 150, 170, false),
    (988.0, 85, 45, false),
    (1319.0, 85, 45, false),
    (1760.0, 190, 0, false),
];

/// A soft three-note chime that settles on a long final note.
const DONE: [Note; 3] = [(523.0, 150, 15, true), (659.0, 150, 15, true), (784.0, 520, 0, true)];

fn wav(notes: &[Note]) -> Vec<u8> {
    let mut pcm: Vec<i16> = Vec::new();
    for &(freq, ms, gap, ring) in notes {
        let count = RATE * ms / 1000;
        for i in 0..count {
            let t = i as f32 / RATE as f32;
            let progress = i as f32 / count as f32;
            // Short fade-in avoids a click; ringing notes decay, beeps stay flat then cut off softly.
            let attack = (i as f32 / (RATE as f32 * 0.004)).min(1.0);
            let shape = if ring { (-3.2 * progress).exp() } else { ((1.0 - progress) * 12.0).min(1.0) };
            // A little second harmonic makes the chime rounder and the beep sharper.
            let wave = (TAU * freq * t).sin() + if ring { 0.25 } else { 0.4 } * (TAU * freq * 2.0 * t).sin();
            pcm.push((wave * attack * shape * VOLUME * 0.7 * i16::MAX as f32) as i16);
        }
        pcm.extend(std::iter::repeat(0).take((RATE * gap / 1000) as usize));
    }

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
    static NEEDS: OnceLock<Vec<u8>> = OnceLock::new();
    static FINISHED: OnceLock<Vec<u8>> = OnceLock::new();
    // The buffers live for the whole run: an async sound keeps reading its memory after this returns.
    let buffer = match cue {
        Cue::NeedsInput => NEEDS.get_or_init(|| wav(&NEEDS_INPUT)),
        Cue::Done => FINISHED.get_or_init(|| wav(&DONE)),
    };
    unsafe {
        PlaySoundW(buffer.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}
