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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/avi-extensible-pcm")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases(key: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[key]
        .as_array()
        .unwrap()
        .clone()
}
fn source(c: &serde_json::Value) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes(&format!("{}.avi", c["name"].as_str().unwrap()))),
            Default::default(),
        )
        .expect("valid extensible AVI PCM must play"),
    )
}
fn weights(c: &serde_json::Value) -> Vec<[f32; 2]> {
    let n = c["channels"].as_u64().unwrap() as usize;
    let mask = c["mask"].as_u64().unwrap();
    let k = std::f32::consts::FRAC_1_SQRT_2;
    match mask {
        4 => vec![[1., 1.]],
        48 => vec![[k, 0.], [0., k]],
        3 => vec![[1., 0.], [0., 1.]],
        63 => vec![[1., 0.], [0., 1.], [k, k], [0., 0.], [k, 0.], [0., k]],
        0 => vec![[1. / n as f32; 2]; n],
        _ => panic!("unrecognized authored role"),
    }
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
fn container_geometry_wave_pcm_and_neutral_video_are_valid_before_avi_dispatch() {
    for c in cases("cases") {
        let name = c["name"].as_str().unwrap();
        let avi =
            fvid::container::avi::Avi::parse(&bytes(&format!("{name}.avi")), &Default::default())
                .unwrap();
        let data = bytes(&format!("{name}.avi"));
        for limits in [
            fvid::container::avi::Limits {
                file_bytes: data.len() - 1,
                ..Default::default()
            },
            fvid::container::avi::Limits {
                channels: c["channels"].as_u64().unwrap() as usize - 1,
                ..Default::default()
            },
            fvid::container::avi::Limits {
                records: c["parts"].as_array().unwrap().len() - 1,
                ..Default::default()
            },
        ] {
            assert!(fvid::container::avi::Avi::parse(&data, &limits).is_err());
        }
        let a = &avi.audio()[0];
        assert_eq!(a.format.tag, 65534);
        assert_eq!(a.format.channels as u64, c["channels"].as_u64().unwrap());
        assert_eq!(a.records.len(), c["parts"].as_array().unwrap().len());
        let mut wave = fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes(&format!("{name}.wav"))),
            Default::default(),
        )
        .unwrap();
        let full = collect(&mut wave);
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes(&format!("{name}.f32le"))
        );
        assert_eq!(full.iter().map(Packet::frames).sum::<usize>(), 4099);
    }
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        let got: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(got, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(v.read_frame().unwrap().is_none());
}
#[test]
fn extensible_avi_decodes_exact_pcm_native_clocks_speakers_and_bounded_windows() {
    for c in cases("cases") {
        let name = c["name"].as_str().unwrap();
        let mut s = source(&c);
        let full = collect(&mut *s);
        assert_eq!(s.bits_per_sample() as u64, c["bits"].as_u64().unwrap());
        assert_eq!(
            s.speaker_mask().unwrap(),
            Some(c["mask"].as_u64().unwrap() as u32)
        );
        let r = s.stereo_router().unwrap().unwrap();
        assert_eq!(r.coefficients(), weights(&c));
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes(&format!("{name}.f32le"))
        );
        let start = c["start"].as_u64().unwrap();
        assert_eq!(full[0].num * 8000, full[0].den * u128::from(start));
        assert!(full.iter().all(|p| p.frames() <= 2048));
        for pair in full.windows(2) {
            assert_eq!(pair[0].point(pair[0].frames()), pair[1].point(0));
        }
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        assert_eq!(
            s.duration(),
            Some(Duration::from_nanos((start + 4099) * 125000))
        );
        s.rewind();
        exact(&collect(&mut *s), &full, name);
        for target in [0, 1, 3, 2048, 2051, 4098, 4099, 100000, i64::MAX as u64] {
            let target = target.saturating_add(start).min(i64::MAX as u64);
            let actual = s.seek_to(target as i64);
            assert_eq!(actual, target.min(start + 4099) as i64);
            exact(
                &collect(&mut *s),
                &rational::suffix(
                    &full,
                    Duration::new(target / 8000, ((target % 8000) * 125000) as u32),
                    end,
                ),
                name,
            );
        }
        assert!(s.next_packet().unwrap().is_none());
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
fn actual_avi_workers_fold_extensible_speakers_after_seek_and_at_eof() {
    for c in cases("cases") {
        let full = collect(&mut *source(&c));
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        let folded = folded(&full, &c);
        let start = c["start"].as_u64().unwrap();
        for tick in [0, 1, 2048, 4099] {
            let at = Duration::from_nanos((start + tick) * 125000);
            let wanted = rational::suffix(&folded, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker = fvid::audio_thread::AudioPlayback::start(source(&c), move || {
                Box::new(StereoSink {
                    inner: rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    },
                    router: None,
                })
            });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            exact(&got.packets, &wanted, c["name"].as_str().unwrap());
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
#[test]
fn malformed_extensible_avi_is_a_refusal_separate_from_successful_pcm() {
    for c in cases("malformed") {
        let error = fvid::playback_avi_audio::AviAudioReader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .err()
        .unwrap();
        if c["kind"] == "invalid" {
            assert!(matches!(error, fvid::Error::Invalid(_)));
            assert!(
                error.to_string().contains(c["needle"].as_str().unwrap()),
                "{error}"
            );
        } else {
            assert!(matches!(error, fvid::Error::Unsupported(_)));
        }
    }
}
