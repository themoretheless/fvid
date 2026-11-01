use fvid::media::{self, CopyOptions};
use std::path::{Path, PathBuf};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-concat-edit-work")
}
struct Dir(PathBuf);
impl Dir {
    fn new(label: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "fvid-concat-edit-work-{label}-{}",
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
fn authored_edit_decodes_one_packet_with_exact_pcm() {
    let d = Dir::new("source");
    for memory in [None, Some(64 << 20)] {
        let out = d.0.join(format!("{memory:?}.f32le"));
        let stats = media::decode_audio(
            &root().join("early.m4a"),
            &out,
            &CopyOptions {
                max_controlled_bytes: memory,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(stats.decoded_frames, 1);
        assert_eq!(stats.sample_frames, 160);
        assert_eq!(
            std::fs::read(out).unwrap(),
            std::fs::read(root().join("early.m4a.f32le")).unwrap()
        );
    }
}
#[test]
fn finite_edit_does_not_hide_reachable_later_packet_admission() {
    let d = Dir::new("reached");
    let sources = [root().join("early.m4a"), root().join("later.m4a")];
    let mut failures = Vec::new();
    for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
        let opts = CopyOptions {
            max_packets: Some(2),
            max_packet_bytes: 320,
            max_controlled_bytes: memory,
            ..Default::default()
        };
        match media::plan_concat(&sources, &opts) {
            Ok(_) => failures.push(format!("{memory:?}:plan accepted oversized successor")),
            Err(err) => assert!(
                err.contains("packet") && (err.contains("budget") || err.contains("limit")),
                "{err}"
            ),
        };
        let out = d.0.join(format!("{i}.wav"));
        let err = media::concat(&sources, &out, &opts).unwrap_err();
        assert!(
            err.contains("packet") && (err.contains("budget") || err.contains("limit")),
            "{err}"
        );
        assert!(!out.exists());
    }
    assert!(failures.is_empty(), "{failures:?}");
}
fn wave_data(data: &[u8]) -> &[u8] {
    let mut at = 12;
    loop {
        let n = u32::from_le_bytes(data[at + 4..at + 8].try_into().unwrap()) as usize;
        if &data[at..at + 4] == b"data" {
            return &data[at + 8..at + 8 + n];
        }
        at += 8 + n + n % 2;
    }
}
#[test]
fn healthy_successor_and_exact_exhaustion_preserve_complete_pcm() {
    let d = Dir::new("controls");
    let sources = [root().join("early.m4a"), root().join("later.m4a")];
    for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
        for work in [1, 2] {
            let opts = CopyOptions {
                max_packets: Some(work),
                max_packet_bytes: if work == 1 { 320 } else { 640 },
                max_controlled_bytes: memory,
                ..Default::default()
            };
            media::plan_concat(&sources, &opts).unwrap();
            let out = d.0.join(format!("{i}-{work}.wav"));
            let stats = media::concat(&sources, &out, &opts).unwrap();
            assert_eq!(stats.packets, work);
            let mut wanted = std::fs::read(root().join("early.m4a.f32le")).unwrap();
            if work == 2 {
                wanted.extend(std::fs::read(root().join("later.m4a.f32le")).unwrap());
            }
            assert_eq!(wave_data(&std::fs::read(out).unwrap()), wanted);
        }
    }
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
fn compare_pcm(actual: &[u8], expected: &[u8]) {
    assert_eq!(actual.len(), expected.len());
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(
            a.is_finite() && b.is_finite() && (a - b).abs() < 2e-6,
            "{a}!={b}"
        );
    }
}
#[test]
fn compressed_repeats_preroll_and_gap_match_independent_pcm_and_work() {
    let d = Dir::new("codecs");
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            let out = d.0.join(format!("{name}-{i}.f32le"));
            let stats = media::decode_audio(
                &root().join(name),
                &out,
                &CopyOptions {
                    max_controlled_bytes: memory,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stats.decoded_frames, c["work"].as_u64().unwrap(), "{name}");
            assert_eq!(stats.sample_frames, c["frames"].as_u64().unwrap(), "{name}");
            compare_pcm(
                &std::fs::read(out).unwrap(),
                &std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap(),
            );
        }
    }
}
#[test]
fn edited_codec_count_keeps_later_admission_and_exact_exhaustion() {
    let d = Dir::new("codec-plans");
    let mut failures = Vec::new();
    for c in cases() {
        let name = c["file"].as_str().unwrap();
        let later = c["later"].as_str().unwrap();
        let sources = [root().join(name), root().join(later)];
        let work = c["work"].as_u64().unwrap();
        for (i, memory) in [None, Some(64 << 20)].into_iter().enumerate() {
            for (label, limit, cap) in [
                ("exhaust", work, 2048),
                ("healthy", work + 1, 4096),
                ("reached", work + 1, 2048),
            ] {
                let opts = CopyOptions {
                    max_packets: Some(limit),
                    max_packet_bytes: cap,
                    max_controlled_bytes: memory,
                    ..Default::default()
                };
                let plan = media::plan_concat(&sources, &opts);
                let out = d.0.join(format!("{name}-{i}-{label}.wav"));
                let result = media::concat(&sources, &out, &opts);
                if label == "reached" {
                    if plan.is_ok() {
                        failures.push(format!("{name}:{memory:?}:false plan acceptance"));
                    } else {
                        let e = plan.unwrap_err();
                        assert!(
                            e.contains("packet") && (e.contains("budget") || e.contains("limit")),
                            "{name}:{e}"
                        );
                    }
                    let e = result.unwrap_err();
                    assert!(
                        e.contains("packet") && (e.contains("budget") || e.contains("limit")),
                        "{name}:{e}"
                    );
                    assert!(!out.exists());
                } else {
                    if let Err(e) = plan {
                        failures.push(format!("{name}:{memory:?}:{label}:false refusal {e}"));
                        continue;
                    }
                    let stats = result.unwrap_or_else(|e| panic!("{name}:{label}:{e}"));
                    assert_eq!(stats.packets, limit, "{name}:{label}");
                    let mut expected =
                        std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
                    if label == "healthy" {
                        expected
                            .extend(std::fs::read(root().join(format!("{later}.f32le"))).unwrap());
                    }
                    compare_pcm(wave_data(&std::fs::read(out).unwrap()), &expected);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
#[cfg(feature = "player")]
#[test]
fn short_video_control_retains_exact_pictures_seek_eof_and_edited_audio() {
    use std::{io::Cursor, time::Duration};
    let bytes = std::fs::read(root().join("video-control.mp4")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes), 32 << 20).unwrap();
    let mut count = 0;
    while let Some(frame) = video.read_frame_raw().unwrap() {
        assert_eq!(frame.into_rgb(32 << 20).unwrap(), vec![130; 16 * 16 * 3]);
        count += 1;
    }
    assert!(count >= 2);
    assert!(video.read_frame_raw().unwrap().is_none());
    video.rewind().unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert_eq!(
        video
            .seek_raw(Duration::from_millis(25))
            .unwrap()
            .unwrap()
            .into_rgb(32 << 20)
            .unwrap(),
        vec![130; 16 * 16 * 3]
    );
    assert_eq!(video.frame_interval(), Some((1, 2, 50)));
    let d = Dir::new("video");
    let out = d.0.join("audio.f32le");
    let stats = media::decode_audio(
        &root().join("video-control.mp4"),
        &out,
        &CopyOptions::default(),
    )
    .unwrap();
    assert_eq!(stats.decoded_frames, 1);
    assert_eq!(stats.sample_frames, 160);
    assert_eq!(
        std::fs::read(out).unwrap(),
        std::fs::read(root().join("video-control.mp4.f32le")).unwrap()
    );
}
