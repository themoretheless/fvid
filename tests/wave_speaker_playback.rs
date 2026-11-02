#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::{AudioBackend, AudioError, AudioPacket, AudioSpec, AudioStream};
use rational::{Packet, collect, exact};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/wave-speaker-playback")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn source(c: &serde_json::Value) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes(&format!("{}.wav", c["name"].as_str().unwrap()))),
            Default::default(),
        )
        .unwrap(),
    )
}
fn weights(c: &serde_json::Value) -> Vec<[f32; 2]> {
    let n = c["channels"].as_u64().unwrap() as usize;
    let mask = c["mask"].as_u64().unwrap_or(3);
    let k = std::f32::consts::FRAC_1_SQRT_2;
    if mask == 0 {
        return vec![[1. / n as f32; 2]; n];
    }
    let mut roles: Vec<_> = (0..32)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| match bit {
            0 => [1., 0.],
            1 => [0., 1.],
            2 => [k, k],
            3 => [0., 0.],
            4 => [k, 0.],
            5 => [0., k],
            _ => panic!("unexpected role"),
        })
        .collect();
    roles.resize(n, [0., 0.]);
    roles
}
fn folded(full: &[Packet], c: &serde_json::Value) -> Vec<Packet> {
    let w = weights(c);
    full.iter()
        .map(|p| {
            let mut data = Vec::new();
            for f in p.data.chunks_exact(w.len() * 4) {
                let mut sum = [0f32; 2];
                for (x, weight) in f.chunks_exact(4).zip(&w) {
                    let x = f32::from_le_bytes(x.try_into().unwrap());
                    sum[0] += x * weight[0];
                    sum[1] += x * weight[1];
                }
                data.extend(sum.into_iter().flat_map(f32::to_le_bytes));
            }
            Packet {
                data,
                channels: 2,
                ..p.clone()
            }
        })
        .collect()
}
#[test]
fn native_pcm_video_and_refusal_controls_are_independent_of_stereo_negotiation() {
    for c in cases() {
        let mut s = source(&c);
        let full = collect(&mut *s);
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes(&format!("{}.f32le", c["name"].as_str().unwrap()))
        );
        assert_eq!(full.iter().map(Packet::frames).sum::<usize>(), 4099);
        assert_eq!(full[0].point(0), (0, 1));
        assert_eq!(
            full.last().unwrap().point(full.last().unwrap().frames()),
            (4099, 44100)
        );
        s.rewind();
        exact(&collect(&mut *s), &full, "rewind");
    }
    for name in ["precise-s16.aiff", "precise-s16.au", "precise-f32.au"] {
        let mut source: Box<dyn AudioStream> = if name.ends_with("aiff") {
            Box::new(
                fvid::playback_aiff::AiffAudioReader::open(
                    Cursor::new(bytes(name)),
                    Default::default(),
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_au::AuAudioReader::open(
                    Cursor::new(bytes(name)),
                    Default::default(),
                )
                .unwrap(),
            )
        };
        let full = collect(&mut *source);
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes("precise.f32le"),
            "{name}"
        );
        assert_eq!(
            full.last().unwrap().point(full.last().unwrap().frames()),
            (4099, 44100)
        );
        source.rewind();
        exact(&collect(&mut *source), &full, name);
    }
    let err = fvid::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("too-many-speakers.wav")),
        Default::default(),
    )
    .err()
    .unwrap();
    assert!(err.to_string().contains("channel mask"));
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        let data: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(data, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(v.read_frame().unwrap().is_none());
}
#[test]
fn extensible_mask_reaches_device_even_for_rear_stereo_and_lfe_mono() {
    for c in cases() {
        let s = source(&c);
        let mask = c["mask"].as_u64().map(|v| v as u32);
        assert_eq!(s.speaker_mask().unwrap(), mask, "{}", c["name"]);
        let r = s.stereo_router().unwrap();
        if mask.is_none() {
            assert!(r.is_none());
            continue;
        }
        let r = r.expect("declared WAVE speakers need a device plan");
        assert_eq!(r.source_mask(), mask);
        assert_eq!(r.coefficients(), weights(&c), "{}", c["name"]);
        let n = s.channels() as usize;
        let basis: Vec<_> = (0..n)
            .flat_map(|active| {
                (0..n).flat_map(move |i| {
                    if i == active {
                        1f32.to_le_bytes()
                    } else {
                        0f32.to_le_bytes()
                    }
                })
            })
            .collect();
        assert_eq!(
            r.route(&basis, n * 8).unwrap(),
            weights(&c).into_iter().flatten().collect::<Vec<_>>()
        );
        assert!(r.route(&basis, n * 8 - 1).is_err());
        assert!(r.route(&basis[..basis.len() - 1], n * 8).is_err());
    }
}
struct StereoSink {
    inner: rational::RawCapture,
    router: Option<fvid_media::owned_stereo_router::StereoRouter>,
}
impl AudioBackend for StereoSink {
    fn start(&mut self, s: AudioSpec) -> Result<(), AudioError> {
        if s.channels != 2 {
            return Err(AudioError::InitFailed(format!(
                "stereo-only synthetic sink rejects {} channels",
                s.channels
            )));
        }
        self.inner.start(s)
    }
    fn start_with_router(
        &mut self,
        s: AudioSpec,
        r: Option<fvid_media::owned_stereo_router::StereoRouter>,
    ) -> Result<(), AudioError> {
        if let Some(r) = &r {
            assert_eq!(r.input_channels(), s.channels);
        }
        let channels = if r.is_some() { 2 } else { s.channels };
        self.router = r;
        self.start(AudioSpec { channels, ..s })
    }
    fn push(&mut self, mut p: AudioPacket) -> Result<(), AudioError> {
        if let Some(r) = &self.router {
            p.data = r
                .route(&p.data, 32 << 20)
                .map_err(AudioError::PlaybackError)?
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect();
        }
        self.inner.push(p)
    }
    fn position(&self) -> Duration {
        self.inner.position()
    }
    fn flush(&mut self, at: Duration) -> Result<(), AudioError> {
        self.inner.flush(at)
    }
    fn pause(&mut self) -> Result<(), AudioError> {
        self.inner.pause()
    }
    fn resume(&mut self) -> Result<(), AudioError> {
        self.inner.resume()
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        self.inner.stop()
    }
    fn begin_drain(&mut self) -> Result<(), AudioError> {
        self.inner.begin_drain()
    }
    fn drain_status(&self) -> Result<bool, AudioError> {
        self.inner.drain_status()
    }
    fn end_drain(&mut self) -> Result<(), AudioError> {
        self.inner.end_drain()
    }
}

#[test]
fn actual_wave_workers_preserve_stereo_pcm_and_native_clock_across_seek() {
    use fvid::audio_thread::AudioPlayback;
    use transitions::{Capture, Trace, ended};
    for c in cases() {
        let full = collect(&mut *source(&c));
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        let stereo = folded(&full, &c);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::from_nanos(2048 * 1_000_000_000 / 44100 + 1),
            Duration::MAX,
        ] {
            let expected = rational::suffix(&stereo, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(Trace::default()));
            let mut worker = AudioPlayback::start(source(&c), move || {
                Box::new(StereoSink {
                    inner: rational::RawCapture {
                        inner: Capture::new(trace),
                        raw: copy,
                    },
                    router: None,
                })
            });
            worker.seek(at);
            worker.play();
            ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            exact(&got.packets, &expected, c["name"].as_str().unwrap());
            assert_eq!(got.anchors, vec![rational::anchor(&expected, end)]);
        }
    }
}

#[test]
fn duration_seek_selects_the_first_native_sample_at_or_after_requested_time() {
    for c in cases() {
        let full = collect(&mut *source(&c));
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        for at in [
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::from_nanos(2048 * 1_000_000_000 / 44100 + 1),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, end);
            for warm in [false, true] {
                let mut s = source(&c);
                if warm {
                    exact(&collect(&mut *s), &full, "warm full control");
                }
                let landing = s.seek_time(at);
                assert_eq!(
                    landing as u128,
                    (at.as_nanos() * 44100).div_ceil(1_000_000_000).min(4099),
                    "{} at{at:?}",
                    c["name"]
                );
                exact(&collect(&mut *s), &wanted, "precise native suffix");
                assert!(s.next_packet().unwrap().is_none());
                // Raw coordinates retain their existing exact-frame API.
                assert_eq!(s.seek_to(2049), 2049);
                let mut raw = full.clone();
                raw = rational::suffix(
                    &raw,
                    Duration::from_nanos(2049 * 1_000_000_000 / 44100),
                    end,
                );
                exact(&collect(&mut *s), &raw, "raw seek following precise");
                s.rewind();
                exact(&collect(&mut *s), &full, "rewind after precise");
            }
        }
    }
}

#[test]
fn predictive_window_duration_seek_restores_history_before_presenting_fractional_targets() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/coded-seek-history");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for c in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "wav" || c["kind"] == "aiff")
    {
        let data = std::fs::read(base.join(c["file"].as_str().unwrap())).unwrap();
        let open = || -> Box<dyn AudioStream> {
            if c["kind"] == "aiff" {
                Box::new(
                    fvid::playback_aiff::AiffAudioReader::open(
                        Cursor::new(data.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_wav::WavAudioReader::open(
                        Cursor::new(data.clone()),
                        Default::default(),
                    )
                    .unwrap(),
                )
            }
        };
        let full = collect(&mut *open());
        let last = full.last().unwrap();
        let end = last.point(last.frames());
        for at in [
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::from_nanos(
                c["grain"].as_u64().unwrap() * 1_000_000_000 / u64::from(last.rate) + 1,
            ),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, end);
            for warm in [false, true] {
                let mut s = open();
                if warm {
                    exact(&collect(&mut *s), &full, "predictive full control");
                }
                let target = (at.as_nanos() * u128::from(last.rate))
                    .div_ceil(1_000_000_000)
                    .min(end.0 * u128::from(last.rate) / end.1);
                assert_eq!(
                    s.seek_time(at) as u128,
                    target,
                    "{} fractional landing",
                    c["name"]
                );
                exact(&collect(&mut *s), &wanted, "predictive precise suffix");
                s.rewind();
                exact(&collect(&mut *s), &full, "predictive rewind");
            }
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker = fvid::audio_thread::AudioPlayback::start(open(), move || {
                Box::new(rational::RawCapture {
                    inner: transitions::Capture::new(trace),
                    raw: copy,
                })
            });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            exact(&got.packets, &wanted, "predictive worker suffix");
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}

#[test]
fn aiff_and_au_duration_seeks_share_the_unquantized_native_sample_contract() {
    for name in ["precise-s16.aiff", "precise-s16.au", "precise-f32.au"] {
        let open = || -> Box<dyn AudioStream> {
            if name.ends_with("aiff") {
                Box::new(
                    fvid::playback_aiff::AiffAudioReader::open(
                        Cursor::new(bytes(name)),
                        Default::default(),
                    )
                    .unwrap(),
                )
            } else {
                Box::new(
                    fvid::playback_au::AuAudioReader::open(
                        Cursor::new(bytes(name)),
                        Default::default(),
                    )
                    .unwrap(),
                )
            }
        };
        let full = collect(&mut *open());
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes("precise.f32le")
        );
        let last = full.last().unwrap();
        let end = last.point(last.frames());
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::from_nanos(2048 * 1_000_000_000 / 44100 + 1),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, end);
            for warm in [false, true] {
                let mut s = open();
                if warm {
                    exact(&collect(&mut *s), &full, name);
                }
                assert_eq!(
                    s.seek_time(at) as u128,
                    (at.as_nanos() * 44100).div_ceil(1_000_000_000).min(4099),
                    "{name} at{at:?}"
                );
                exact(&collect(&mut *s), &wanted, name);
                assert!(s.next_packet().unwrap().is_none());
                s.rewind();
                exact(&collect(&mut *s), &full, name);
            }
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker = fvid::audio_thread::AudioPlayback::start(open(), move || {
                Box::new(rational::RawCapture {
                    inner: transitions::Capture::new(trace),
                    raw: copy,
                })
            });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            exact(&got.packets, &wanted, name);
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
