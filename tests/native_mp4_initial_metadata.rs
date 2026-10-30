#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
#[path = "support/audio_transitions.rs"]
mod transitions;
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/native-mp4-initial-metadata")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
#[test]
fn metadata_inspection_reports_the_first_actual_setup_without_requiring_constant_descriptions() {
    let mut wrong = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let p = path(n);
        for mode in 0..3 {
            let info = match mode {
                0 => fvid::native_media::audio_source_info_selected(&p, None),
                1 => fvid::native_media::audio_source_info_selected_with_options(
                    &p,
                    Some(1),
                    &Default::default(),
                ),
                _ => fvid::native_media::audio_source_info_selected_with_interval_and_options(
                    &p,
                    Some(1),
                    Some((Duration::ZERO, Duration::from_millis(10))),
                    &fvid::media::CopyOptions {
                        max_packets: Some(1),
                        ..Default::default()
                    },
                ),
            };
            match info {
                Ok(i) => assert_eq!(
                    (i.stream_index, i.sample_rate, i.channels, i.codec),
                    (
                        1,
                        c["rate"].as_u64().unwrap() as u32,
                        c["channels"].as_u64().unwrap() as u16,
                        c["codec"].as_str().unwrap()
                    ),
                    "{n} mode{mode}"
                ),
                Err(e) => wrong.push(format!("{n} mode{mode}: {e}")),
            }
        }
        fvid::native_plan::decode_audio(&p, &Default::default()).unwrap();
    }
    assert!(
        wrong.is_empty(),
        "metadata unnecessarily requires constant decoder setup: {wrong:?}"
    );
}
#[test]
fn the_witnesses_play_exact_pcm_transitions_and_all_video_frames() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut s = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(bytes(n)),
            Default::default(),
        )
        .unwrap();
        for _ in 0..2 {
            let got = transitions::collect(&mut s);
            let frames = c["frames"].as_array().unwrap();
            assert_eq!(got.len(), frames.len());
            for (i, (a, e)) in got.iter().zip(frames).enumerate() {
                assert_eq!(
                    (a.0, a.1, a.2),
                    (
                        e["rate"].as_u64().unwrap() as u32,
                        e["channels"].as_u64().unwrap() as u16,
                        Duration::from_millis(i as u64 * 20)
                    )
                );
                assert_eq!(a.3, bytes(e["oracle"].as_str().unwrap()), "{n} frame{i}");
            }
            s.rewind();
        }
        let mut v =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes(n)), 32 << 20).unwrap();
        for _ in 0..6 {
            assert!(v.read_frame_raw().unwrap().is_some());
        }
        assert!(v.read_frame_raw().unwrap().is_none());
    }
}
#[test]
fn inspection_keeps_limits_and_constant_route_admission_separate() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let p = path(n);
        assert_eq!(
            fvid::native_media::is_owned_audio_source(&p).unwrap(),
            !c["changing"].as_bool().unwrap(),
            "{n}: constant native route admission changed"
        );
        let cancelled = fvid::media::CancelFlag::new();
        cancelled.cancel();
        for (options, reason) in [
            fvid::media::CopyOptions {
                cancel: Some(cancelled),
                ..Default::default()
            },
            fvid::media::CopyOptions {
                max_packet_bytes: 0,
                ..Default::default()
            },
            fvid::media::CopyOptions {
                max_packets: Some(0),
                ..Default::default()
            },
            fvid::media::CopyOptions {
                max_controlled_bytes: Some(1),
                ..Default::default()
            },
            fvid::media::CopyOptions {
                max_rss_bytes: Some(0),
                ..Default::default()
            },
        ]
        .into_iter()
        .zip([
            "cancelled",
            "packet byte limit",
            "packet work limit",
            "metadata exceeds budget",
            "rss",
        ]) {
            let error =
                fvid::native_media::audio_source_info_selected_with_options(&p, None, &options)
                    .err()
                    .expect("inspection ignored a limit")
                    .to_string();
            assert!(
                error.contains(reason),
                "{n}: expected {reason}, got {error}"
            );
        }
        let error = fvid::native_media::audio_source_info_selected(&p, Some(9))
            .err()
            .expect("invalid stream accepted");
        assert!(
            error
                .to_string()
                .contains("selected stream is absent or is not audio"),
            "{n}: {error}"
        );
    }
}
#[test]
fn explicit_constant_inspection_preserves_each_admission_variant() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let p = path(n);
        for mode in 0..3 {
            let result=match mode {
                0=>fvid::native_media::constant_audio_source_info_selected(&p,None),
                1=>fvid::native_media::constant_audio_source_info_selected_with_options(&p,Some(1),&Default::default()),
                _=>fvid::native_media::constant_audio_source_info_selected_with_interval_and_options(&p,Some(1),Some((Duration::ZERO,Duration::from_millis(10))),&fvid::media::CopyOptions{max_packets:Some(1),..Default::default()}),
            };
            if c["changing"].as_bool().unwrap() {
                let error = result
                    .err()
                    .expect("constant inspection admitted a changing configuration");
                assert!(
                    error
                        .to_string()
                        .contains("sample description changes require decoder reconfiguration"),
                    "{n}: {error}"
                );
            } else {
                let info = result.unwrap();
                assert_eq!(
                    (info.sample_rate, info.channels),
                    (
                        c["rate"].as_u64().unwrap() as u32,
                        c["channels"].as_u64().unwrap() as u16
                    )
                );
            }
        }
        let options = fvid::media::CopyOptions {
            max_controlled_bytes: Some(1),
            ..Default::default()
        };
        let error = fvid::native_media::constant_audio_source_info_selected_with_options(
            &p, None, &options,
        )
        .err()
        .expect("strict admission ignored metadata budget");
        assert!(
            error.to_string().contains("metadata exceeds budget"),
            "{n}: {error}"
        );
    }
}
