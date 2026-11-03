use fvid_media::{AudioDecodeTransform, CopyOptions};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-edit-selected-admission")
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(
        &std::fs::read(root().join("manifest.json")).unwrap(),
    )
    .unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn dormant_descriptions_do_not_expand_the_finite_edit_packet_budget() {
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let expected = std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
        for cap in [400, 1024] {
            let opts = CopyOptions {
                max_packet_bytes: cap,
                ..Default::default()
            };
            let mut pcm = Vec::new();
            match fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(std::fs::read(root().join(name)).unwrap()),
                &mut pcm,
                None,
                &opts,
            ) {
                Ok(stats) => {
                    assert_eq!(pcm, expected, "{name}");
                    assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap(), "{name}");
                    assert_eq!(stats.decoded_frames, c["work"].as_u64().unwrap(), "{name}");
                }
                Err(e) => failures.push(format!("{name}:cap{cap}:{e}")),
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} admission failures: {failures:?}",
        failures.len()
    );
}
#[test]
fn original_edits_and_reached_large_packets_keep_their_controls() {
    for codec in ["pcm", "alac"] {
        for cap in [400, 1024] {
            let opts = CopyOptions {
                max_packet_bytes: cap,
                ..Default::default()
            };
            let name = format!("{codec}-full-control.mp4");
            let mut pcm = Vec::new();
            let result = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(std::fs::read(root().join(&name)).unwrap()),
                &mut pcm,
                None,
                &opts,
            );
            if cap == 400 {
                let e = result.unwrap_err().to_string();
                assert!(
                    e.contains("packet") && (e.contains("budget") || e.contains("limit")),
                    "{e}"
                );
                assert!(pcm.is_empty());
            } else {
                assert_eq!(result.unwrap().sample_frames, 480);
                assert_eq!(
                    pcm,
                    std::fs::read(root().join(format!("{codec}-full-control.f32le"))).unwrap()
                );
            }
        }
    }
    for c in cases() {
        let opts = CopyOptions {
            max_packet_bytes: 400,
            ..Default::default()
        };
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap()),
            &mut pcm,
            None,
            &opts,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap());
        assert_eq!(
            pcm,
            std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap()
        );
    }
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!("fvid-edit-selected-{}", std::process::id()));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn public_plans_exports_f64_and_resource_refusals_keep_edit_boundaries() {
    let dir = Temp::new();
    for (i, c) in cases().into_iter().enumerate() {
        let name = c["file"].as_str().unwrap();
        let source = root().join(name);
        let expected = std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
        for memory in [None, Some(64 << 20)] {
            let opts = CopyOptions {
                max_packet_bytes: 400,
                max_controlled_bytes: memory,
                ..Default::default()
            };
            fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
                .unwrap();
            let out = dir.0.join(format!("{i}-{}.f32le", memory.is_some()));
            let stats = fvid::media::decode_audio(&source, &out, &opts).unwrap();
            assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap());
            assert_eq!(std::fs::read(out).unwrap(), expected);
            let mut pcm = Vec::new();
            let precise = fvid_media::owned_mp4_audio::decode_mp4_pcm_f64(
                std::fs::File::open(&source).unwrap(),
                &mut pcm,
                None,
                &opts,
            );
            if name.starts_with("pcm-") {
                precise.unwrap();
                let expected64: Vec<_> = expected
                    .chunks_exact(4)
                    .flat_map(|v| (f32::from_le_bytes(v.try_into().unwrap()) as f64).to_le_bytes())
                    .collect();
                assert_eq!(pcm, expected64);
            } else {
                // The public precise API is PCM-only; ALAC is a refusal control.
                assert_eq!(
                    precise.unwrap_err().to_string(),
                    "selected MP4 audio stream is not supported PCM"
                );
                assert!(pcm.is_empty());
            }
            let tiny = CopyOptions {
                max_packet_bytes: 1,
                ..opts.clone()
            };
            let result =
                fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &tiny);
            if c["work"] == 0 {
                result.unwrap();
            } else {
                let e = result.unwrap_err();
                assert!(e.contains("packet"), "{e}");
            }
        }
        let tiny = CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        assert!(
            fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &tiny)
                .is_err()
        );
        let cancel = fvid::media_control::CancelFlag::new();
        cancel.cancel();
        let opts = CopyOptions {
            cancel: Some(cancel),
            ..Default::default()
        };
        assert!(
            fvid_media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
                .unwrap_err()
                .contains("cancel")
        );
    }
}
#[cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn actual_native_pcm_workers_seek_and_video_keep_the_selected_timeline() {
    use fvid::audio::AudioStream;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let load = |n: &str| std::fs::read(root().join(n)).unwrap();
        let open = |n: &str| {
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(load(n)), Default::default())
                .unwrap()
        };
        let mut reader = open(name);
        let mut control = open(c["control"].as_str().unwrap());
        let full = rational::collect(&mut control);
        assert_eq!(
            full.iter()
                .flat_map(|p| p.data.iter().copied())
                .collect::<Vec<_>>(),
            load(c["native_oracle"].as_str().unwrap()),
            "{name}"
        );
        assert_eq!(reader.duration(), control.duration());
        rational::exact(&rational::collect(&mut reader), &full, name);
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(10),
            Duration::MAX,
        ] {
            control.seek_time(at);
            let expected = rational::collect(&mut control);
            reader.seek_time(at);
            rational::exact(&rational::collect(&mut reader), &expected, name);
        }
        reader.rewind();
        rational::exact(&rational::collect(&mut reader), &full, name);
        reader.validate_eof().unwrap();
        for at in [Duration::ZERO, Duration::from_millis(10), Duration::MAX] {
            control.seek_time(at);
            let expected = rational::collect(&mut control);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(Box::new(open(name)), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            rational::exact(&raw.lock().unwrap().packets, &expected, name);
        }
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(load(name)), 32 << 20)
                .unwrap();
        for _ in 0..3 {
            assert_eq!(
                video
                    .read_frame_raw()
                    .unwrap()
                    .unwrap()
                    .into_rgb(32 << 20)
                    .unwrap(),
                vec![130; 16 * 16 * 3]
            );
        }
        assert!(video.read_frame_raw().unwrap().is_none());
        video.rewind().unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.seek_raw(Duration::from_millis(25)).unwrap().is_some());
    }
}
