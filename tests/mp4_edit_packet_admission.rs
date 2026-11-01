use fvid::media::{self, AudioDecodeTransform, CopyOptions};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-edit-packet-admission")
}
struct Dir(PathBuf);
impl Dir {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "fvid-edit-admission-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn full_source_and_explicit_prefix_prove_valid_packet_and_exact_pcm() {
    let d = Dir::new("controls");
    for suffix in ["m4a", "mp4"] {
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            for edited in [false, true] {
                let name = format!("{}.{suffix}", if edited { "early" } else { "full" });
                let out = d.0.join(format!("{name}-{i}.f32le"));
                let opts = CopyOptions {
                    max_packet_bytes: if edited { 320 } else { 640 },
                    max_packets: edited.then_some(1),
                    max_controlled_bytes: memory,
                    ..Default::default()
                };
                media::plan_decode_audio(
                    &root().join(&name),
                    &AudioDecodeTransform::default(),
                    &opts,
                )
                .unwrap();
                let stats = media::decode_audio(&root().join(&name), &out, &opts).unwrap();
                assert_eq!(stats.sample_frames, if edited { 160 } else { 480 });
                assert_eq!(stats.decoded_frames, if edited { 1 } else { 3 });
                assert_eq!(
                    std::fs::read(out).unwrap(),
                    std::fs::read(root().join(if edited { "early.f32le" } else { "full.f32le" }))
                        .unwrap()
                );
            }
        }
    }
}
#[test]
fn natural_edit_endpoint_excludes_unread_packet_from_admission() {
    let d = Dir::new("natural");
    let mut failures = Vec::new();
    for suffix in ["m4a", "mp4"] {
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let source = root().join(format!("early.{suffix}"));
            let opts = CopyOptions {
                max_packet_bytes: 320,
                max_controlled_bytes: memory,
                ..Default::default()
            };
            if let Err(e) =
                media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
            {
                failures.push(format!("{suffix}:{memory:?}:plan:{e}"));
            }
            let out = d.0.join(format!("{suffix}-{i}.f32le"));
            match media::decode_audio(&source, &out, &opts) {
                Ok(stats) => {
                    assert_eq!(stats.decoded_frames, 1);
                    assert_eq!(stats.sample_frames, 160);
                    assert_eq!(
                        std::fs::read(out).unwrap(),
                        std::fs::read(root().join("early.f32le")).unwrap()
                    );
                }
                Err(e) => {
                    assert!(!out.exists());
                    failures.push(format!("{suffix}:{memory:?}:decode:{e}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
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
fn larger_alac_and_pcm_packets_are_valid_and_still_refused_when_reached() {
    let d = Dir::new("larger");
    for codec in ["pcm", "alac"] {
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let source = root().join(format!("{codec}-full-control.mp4"));
            let opts = CopyOptions {
                max_packet_bytes: 1024,
                max_controlled_bytes: memory,
                ..Default::default()
            };
            media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts).unwrap();
            let out = d.0.join(format!("{codec}-{i}.f32le"));
            let stats = media::decode_audio(&source, &out, &opts).unwrap();
            assert_eq!(stats.sample_frames, 480);
            assert_eq!(stats.decoded_frames, 3);
            assert_eq!(
                std::fs::read(out).unwrap(),
                std::fs::read(root().join(format!("{codec}-full-control.f32le"))).unwrap()
            );
            let tight = CopyOptions {
                max_packet_bytes: 400,
                ..opts
            };
            let e = media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &tight)
                .unwrap_err();
            assert!(
                e.contains("packet") && (e.contains("budget") || e.contains("limit")),
                "{e}"
            );
            let out = d.0.join(format!("{codec}-{i}-refuse.f32le"));
            assert!(media::decode_audio(&source, &out, &tight).is_err());
            assert!(!out.exists());
        }
    }
}
#[test]
fn exact_movie_clocks_repeated_edits_preroll_and_silence_scope_packet_admission() {
    let d = Dir::new("clocks");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let opts = CopyOptions {
                max_packet_bytes: c["cap"].as_u64().unwrap() as usize,
                max_controlled_bytes: memory,
                ..Default::default()
            };
            let source = root().join(name);
            if let Err(e) =
                media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &opts)
            {
                failures.push(format!("{name}:{memory:?}:plan:{e}"));
            }
            let out = d.0.join(format!("{name}-{i}.f32le"));
            match media::decode_audio(&source, &out, &opts) {
                Ok(stats) => {
                    assert_eq!(stats.decoded_frames, c["work"].as_u64().unwrap(), "{name}");
                    assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap(), "{name}");
                    assert_eq!(
                        std::fs::read(out).unwrap(),
                        std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap(),
                        "{name}"
                    );
                }
                Err(e) => {
                    assert!(!out.exists());
                    failures.push(format!("{name}:{memory:?}:decode:{e}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
#[cfg(feature = "player")]
#[test]
fn native_pictures_seek_rewind_and_eof_survive_audio_admission_fix() {
    use std::{io::Cursor, time::Duration};
    let mut r = fvid::playback_native::NativeReader::software(
        Cursor::new(std::fs::read(root().join("pcm-clock1001-v1-repeat.mp4")).unwrap()),
        32 << 20,
    )
    .unwrap();
    for _ in 0..3 {
        assert_eq!(
            r.read_frame_raw()
                .unwrap()
                .unwrap()
                .into_rgb(32 << 20)
                .unwrap(),
            vec![130; 16 * 16 * 3]
        );
    }
    assert!(r.read_frame_raw().unwrap().is_none());
    r.rewind().unwrap();
    assert!(r.read_frame_raw().unwrap().is_some());
    assert_eq!(
        r.seek_raw(Duration::from_millis(25))
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        vec![130; 16 * 16 * 3]
    );
    assert_eq!(r.frame_interval(), Some((1, 2, 50)));
}
#[test]
fn zero_read_edits_still_admit_initial_decoder_and_preserve_cancellation() {
    let source = root().join("pcm-clock8000-v0-silence.mp4");
    let options = CopyOptions {
        max_packet_bytes: 1024,
        max_controlled_bytes: Some(16 << 10),
        ..Default::default()
    };
    let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        std::fs::File::open(&source).unwrap(),
        &mut std::io::sink(),
        None,
        &options,
    )
    .unwrap_err()
    .to_string();
    assert!(e.contains("controlled memory"), "{e}");
    let cancel = fvid::media_control::CancelFlag::new();
    cancel.cancel();
    let options = CopyOptions {
        cancel: Some(cancel),
        ..Default::default()
    };
    let e =
        media::plan_decode_audio(&source, &AudioDecodeTransform::default(), &options).unwrap_err();
    assert!(e.contains("cancel"), "{e}");
}
