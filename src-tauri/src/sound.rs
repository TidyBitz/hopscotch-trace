//! The sound cues, played by the app itself rather than the webview.
//!
//! WebKitGTK routes Web Audio through GStreamer, and inside an AppImage that
//! path proved to stall after its first buffer. So the page asks the shell to
//! play a cue, and the shell synthesizes the same tones the page's Web Audio
//! code does (three packs, pitch by round trip, pan by map position) and
//! writes them to the system's audio output through cpal: ALSA on Linux
//! (which PipeWire and PulseAudio both serve), WASAPI on Windows, CoreAudio
//! on macOS. The browser demo keeps using Web Audio.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Clone, Copy)]
enum Wave {
    Sine,
    Triangle,
    Square,
    Saw,
}

/// One oscillator with the page's envelope: 8 ms attack, exponential decay to silence at `d`.
struct Tone {
    f: f32,
    to: Option<f32>,
    at: f32,
    d: f32,
    wave: Wave,
    g: f32,
    pan: f32,
}

fn tone(f: f32, d: f32, g: f32) -> Tone {
    Tone { f, to: None, at: 0.0, d, wave: Wave::Sine, g, pan: 0.0 }
}
fn tri(f: f32, d: f32, g: f32) -> Tone {
    Tone { wave: Wave::Triangle, ..tone(f, d, g) }
}
fn sq(f: f32, d: f32, g: f32) -> Tone {
    Tone { wave: Wave::Square, ..tone(f, d, g) }
}
fn saw(f: f32, d: f32, g: f32) -> Tone {
    Tone { wave: Wave::Saw, ..tone(f, d, g) }
}
fn at(t: Tone, at: f32) -> Tone {
    Tone { at, ..t }
}
fn to(t: Tone, to: f32) -> Tone {
    Tone { to: Some(to), ..t }
}
fn pan(t: Tone, pan: f32) -> Tone {
    Tone { pan, ..t }
}

/// The cues, pack by pack, mirroring `Sound` in ui/index.html.
fn cue(pack: &str, name: &str, rtt: f32, p: f32) -> Vec<Tone> {
    let rtt = rtt.min(320.0);
    match (pack, name) {
        ("retro", "start") => [523.0, 659.0].iter().enumerate().map(|(i, &f)| at(sq(f, 0.06, 0.07), i as f32 * 0.06)).collect(),
        ("retro", "hop") => {
            let f = 1200.0 - rtt * 2.0;
            vec![pan(sq(f, 0.045, 0.07), p), pan(at(sq(f * 1.5, 0.045, 0.06), 0.05), p)]
        }
        ("retro", "timeout") => vec![sq(220.0, 0.07, 0.07), at(sq(165.0, 0.1, 0.07), 0.09)],
        ("retro", "route") => [523.0, 659.0, 784.0, 1047.0].iter().enumerate().map(|(i, &f)| at(sq(f, 0.07, 0.07), i as f32 * 0.06)).collect(),
        ("retro", "trace") => [523.0, 659.0, 784.0, 1047.0, 784.0, 1047.0, 1319.0].iter().enumerate().map(|(i, &f)| at(sq(f, 0.09, 0.065), i as f32 * 0.07)).collect(),
        ("retro", "done") => [784.0, 523.0].iter().enumerate().map(|(i, &f)| at(sq(f, 0.1, 0.07), i as f32 * 0.08)).collect(),
        ("retro", "error") => vec![sq(160.0, 0.16, 0.08)],
        ("retro", "failure") => [330.0, 262.0, 196.0, 131.0].iter().enumerate().map(|(i, &f)| at(sq(f, 0.12, 0.07), i as f32 * 0.1)).collect(),

        ("marimba", "start") => [392.0, 523.0].iter().enumerate().flat_map(|(i, &f)| [at(tone(f, 0.35, 0.16), i as f32 * 0.1), at(tone(f * 4.0, 0.06, 0.05), i as f32 * 0.1)]).collect(),
        ("marimba", "hop") => {
            let f = 880.0 - rtt * 1.4;
            vec![pan(tone(f, 0.32, 0.17), p), pan(tone(f * 4.0, 0.05, 0.05), p)]
        }
        ("marimba", "timeout") => vec![tone(175.0, 0.4, 0.16), tone(175.0 * 3.9, 0.05, 0.03)],
        ("marimba", "route") => [523.0, 659.0, 784.0].iter().enumerate().flat_map(|(i, &f)| [at(tone(f, 0.4, 0.15), i as f32 * 0.12), at(tone(f * 4.0, 0.05, 0.04), i as f32 * 0.12)]).collect(),
        ("marimba", "trace") => [523.0, 659.0, 784.0, 1047.0].iter().enumerate().flat_map(|(i, &f)| [at(tone(f, 0.6, 0.13), i as f32 * 0.13), at(tone(f * 4.0, 0.05, 0.04), i as f32 * 0.13)]).collect(),
        ("marimba", "done") => vec![tone(659.0, 0.45, 0.14), at(tone(440.0, 0.5, 0.12), 0.14)],
        ("marimba", "error") => vec![tone(294.0, 0.3, 0.15), at(tone(233.0, 0.4, 0.15), 0.16)],
        ("marimba", "failure") => [262.0, 220.0, 175.0].iter().enumerate().map(|(i, &f)| at(tone(f, 0.45, 0.14), i as f32 * 0.16)).collect(),

        // chirp, the default, and anything unknown
        (_, "start") => vec![tri(523.0, 0.08, 0.22), at(tri(784.0, 0.12, 0.22), 0.08)],
        (_, "hop") => {
            let f = 1500.0 - rtt * 2.6;
            vec![pan(to(tone(f * 0.8, 0.08, 0.2), f * 1.3), p), pan(at(to(tone(f * 1.6, 0.05, 0.045), f * 2.3), 0.012), p)]
        }
        (_, "timeout") => vec![to(tri(196.0, 0.12, 0.22), 150.0)],
        (_, "route") => [659.0, 784.0, 988.0].iter().enumerate().map(|(i, &f)| at(tri(f, 0.17, 0.2), i as f32 * 0.075)).collect(),
        (_, "trace") => {
            let mut v = vec![tri(523.0, 0.12, 0.16)];
            v.extend([523.0, 659.0, 784.0, 1047.0].iter().map(|&f| at(tone(f, 0.65, 0.1), 0.13)));
            v
        }
        (_, "done") => vec![tone(880.0, 0.35, 0.16), tone(1320.0, 0.25, 0.05)],
        (_, "error") => vec![tri(392.0, 0.11, 0.22), at(tri(294.0, 0.17, 0.22), 0.12)],
        (_, "failure") => [247.0, 220.0, 165.0].iter().enumerate().map(|(i, &f)| at(saw(f, 0.21, 0.07), i as f32 * 0.13)).collect(),
        _ => Vec::new(),
    }
}

/// Render a cue to interleaved stereo samples at `rate`.
fn render(tones: &[Tone], rate: f32, master: f32) -> Vec<f32> {
    const TAIL: f32 = 0.03;
    let len = tones.iter().map(|t| t.at + 0.005 + t.d + TAIL).fold(0.0f32, f32::max);
    let n = (len * rate).ceil() as usize + 1;
    let mut out = vec![0.0f32; n * 2];
    for t in tones {
        let t0 = t.at + 0.005;
        let start = (t0 * rate) as usize;
        let end = (((t0 + t.d + TAIL) * rate) as usize).min(n);
        let (l, r) = {
            let a = (t.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            (a.cos(), a.sin())
        };
        let mut phase = 0.0f32;
        let ramp = t.d * 0.85;
        for i in start..end {
            let s = (i - start) as f32 / rate; // seconds into the tone
            let f = match t.to {
                Some(to) if ramp > 0.0 => t.f * (to / t.f).powf((s / ramp).min(1.0)),
                _ => t.f,
            };
            phase = (phase + f / rate).fract();
            let w = match t.wave {
                Wave::Sine => (phase * std::f32::consts::TAU).sin(),
                Wave::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
                Wave::Square => if phase < 0.5 { 1.0 } else { -1.0 },
                Wave::Saw => 2.0 * phase - 1.0,
            };
            // the page's envelope: exponential ramps, which read as linear in decibels
            let env = if s < 0.008 {
                0.0001 * (t.g / 0.0001f32).powf(s / 0.008)
            } else if s < t.d {
                t.g * (0.0001 / t.g).powf((s - 0.008) / (t.d - 0.008).max(0.001))
            } else {
                0.0
            };
            let v = w * env * master;
            out[i * 2] += v * l;
            out[i * 2 + 1] += v * r;
        }
    }
    out
}

struct Clip {
    samples: Vec<f32>,
    pos: usize,
}

type Queue = Arc<Mutex<Vec<Clip>>>;

struct Output {
    queue: Queue,
    rate: f32,
}

static OUTPUT: OnceLock<Result<Output, String>> = OnceLock::new();

/// Sum the queued clips into one callback buffer of `channels` interleaved samples.
fn mix(queue: &Queue, data: &mut [f32], channels: usize) {
    data.iter_mut().for_each(|x| *x = 0.0);
    let Ok(mut q) = queue.lock() else { return };
    let frames = data.len() / channels.max(1);
    for clip in q.iter_mut() {
        for fr in 0..frames {
            let i = clip.pos + fr;
            if i * 2 + 1 >= clip.samples.len() {
                break;
            }
            let (l, r) = (clip.samples[i * 2], clip.samples[i * 2 + 1]);
            match channels {
                1 => data[fr] += (l + r) * 0.5,
                _ => {
                    data[fr * channels] += l;
                    data[fr * channels + 1] += r;
                }
            }
        }
        clip.pos += frames;
    }
    q.retain(|c| c.pos * 2 < c.samples.len());
    data.iter_mut().for_each(|x| *x = x.clamp(-1.0, 1.0));
}

/// Open the default output once, on a thread that keeps the stream alive for the life of the app.
fn output() -> Result<&'static Output, String> {
    OUTPUT
        .get_or_init(|| {
            let queue: Queue = Arc::new(Mutex::new(Vec::new()));
            let (tx, rx) = mpsc::channel::<Result<f32, String>>();
            let q = queue.clone();
            std::thread::Builder::new()
                .name("hopscotch-sound".into())
                .spawn(move || {
                    let open = || -> Result<(cpal::Stream, f32), String> {
                        let host = cpal::default_host();
                        let device = host.default_output_device().ok_or("no audio output device")?;
                        let supported = device.default_output_config().map_err(|e| e.to_string())?;
                        let rate = supported.sample_rate().0 as f32;
                        let channels = supported.channels() as usize;
                        let format = supported.sample_format();
                        let config: cpal::StreamConfig = supported.into();
                        let err = |e: cpal::StreamError| eprintln!("sound: {e}");
                        let stream = match format {
                            cpal::SampleFormat::F32 => {
                                let q = q.clone();
                                device.build_output_stream(&config, move |data: &mut [f32], _| mix(&q, data, channels), err, None)
                            }
                            cpal::SampleFormat::I16 => {
                                let q = q.clone();
                                let mut scratch: Vec<f32> = Vec::new();
                                device.build_output_stream(
                                    &config,
                                    move |data: &mut [i16], _| {
                                        scratch.resize(data.len(), 0.0);
                                        mix(&q, &mut scratch, channels);
                                        for (d, s) in data.iter_mut().zip(&scratch) {
                                            *d = (s * i16::MAX as f32) as i16;
                                        }
                                    },
                                    err,
                                    None,
                                )
                            }
                            other => return Err(format!("unsupported sample format {other:?}")),
                        }
                        .map_err(|e| e.to_string())?;
                        stream.play().map_err(|e| e.to_string())?;
                        Ok((stream, rate))
                    };
                    match open() {
                        Ok((stream, rate)) => {
                            let _ = tx.send(Ok(rate));
                            let _keep = stream; // dropped only when the app exits
                            loop {
                                std::thread::park();
                            }
                        }
                        Err(e) => {
                            let _ = tx.send(Err(e));
                        }
                    }
                })
                .map_err(|e| e.to_string())?;
            let rate = rx.recv().map_err(|_| "sound thread died".to_string())??;
            Ok(Output { queue, rate })
        })
        .as_ref()
        .map_err(|e| e.clone())
}

/// Play one cue. `vol` is the page's 0..100 slider; `rtt` and `pan` only matter for `hop`.
pub fn play(name: &str, pack: &str, rtt: f32, pan: f32, vol: f32) -> Result<(), String> {
    let out = output()?;
    let master = (vol.clamp(0.0, 100.0) / 100.0).powf(1.6) * 0.9;
    if master <= 0.0 {
        return Ok(());
    }
    let samples = render(&cue(pack, name, rtt, pan), out.rate, master);
    if samples.is_empty() {
        return Ok(());
    }
    let mut q = out.queue.lock().map_err(|_| "sound queue poisoned".to_string())?;
    if q.len() > 12 {
        q.remove(0); // a burst of hops should not pile up forever
    }
    q.push(Clip { samples, pos: 0 });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_cue_renders_in_every_pack() {
        for pack in ["chirp", "retro", "marimba"] {
            for name in ["start", "hop", "timeout", "route", "trace", "done", "error", "failure"] {
                let s = render(&cue(pack, name, 40.0, 0.3), 48_000.0, 0.5);
                assert!(!s.is_empty(), "{pack}/{name}");
                assert!(s.iter().any(|v| v.abs() > 0.001), "{pack}/{name} is silent");
                assert!(s.iter().all(|v| v.abs() <= 1.0), "{pack}/{name} clips");
            }
        }
    }
    #[test]
    fn hop_pitch_falls_with_round_trip() {
        let fast = cue("chirp", "hop", 10.0, 0.0);
        let slow = cue("chirp", "hop", 300.0, 0.0);
        assert!(fast[0].f > slow[0].f);
    }
}
