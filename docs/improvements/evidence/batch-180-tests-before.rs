use fvid::container::audio_timeline::AudioTimeline;
use fvid::container::mp4::{Edit, Limits as RootLimits, Mp4Reader as RootMp4Reader};
use fvid::playback_native::NativeReader;
use fvid_media::decode_video;
use fvid_media::owned_mp4::{Limits as OwnedLimits, Mp4Reader as OwnedMp4Reader};
use fvid_media::owned_mp4_probe::probe_mp4;
use fvid_media::owned_video_timeline::map_edits;
use std::{io::Cursor, path::PathBuf};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-elst-validation")
}

fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(fixture_dir().join(n)).unwrap()
}

fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}

#[test]
fn reproduction_pre_fix_accepts_empty_zero_duration_and_blank_only_edit_lists() {
    // 1. Pre-fix RootMp4Reader and OwnedMp4Reader accept `elst` with entry_count == 0
    // and silently discard the presence of `edts/elst`.
    let empty_elst = bytes("empty-elst-v0.mp4");
    let root_empty = RootMp4Reader::open(Cursor::new(&empty_elst), RootLimits::default()).unwrap();
    let owned_empty =
        OwnedMp4Reader::open(Cursor::new(&empty_elst), OwnedLimits::default()).unwrap();
    assert!(root_empty.tracks()[0].edits.is_empty());
    assert!(owned_empty.tracks()[0].edits.is_empty());

    // 2. Pre-fix RootMp4Reader, OwnedMp4Reader, probe_mp4, and AudioTimeline::new
    // accept `elst` entries with segment_duration == 0.
    let zero_v0 = bytes("zero-duration-v0.mp4");
    let root_zero = RootMp4Reader::open(Cursor::new(&zero_v0), RootLimits::default()).unwrap();
    let owned_zero = OwnedMp4Reader::open(Cursor::new(&zero_v0), OwnedLimits::default()).unwrap();
    assert_eq!(root_zero.tracks()[0].edits[0].duration, 0);
    assert_eq!(owned_zero.tracks()[0].edits[0].duration, 0);
    let info = probe_mp4(&fixture_dir().join("zero-duration-v0.mp4")).unwrap();
    assert_eq!(info.duration_us, Some(0));
    let schedule =
        AudioTimeline::new(&[Edit { duration: 0, media_time: 0 }], 3072, 48_000, 48_000, 48_000)
            .unwrap();
    assert_eq!(schedule.sample_frames, 0);

    // 3. Pre-fix map_edits, NativeReader, and decode_video silently discard
    // blank-only edit lists (`[(1024, -1), (1024, -1)]`) and play all 3 unedited frames!
    let mapped = map_edits([(1024, -1), (1024, -1)], 48_000, 48_000).unwrap();
    assert!(mapped.is_empty());
    let blank_only = bytes("blank-only-video.mp4");
    let mut reader = NativeReader::software(Cursor::new(&blank_only), 16 << 20).unwrap();
    let mut count = 0;
    while reader.read_frame_raw().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 3);
    let stats = decode_video(&fixture_dir().join("blank-only-video.mp4")).unwrap();
    assert_eq!(stats.video_frames, 3);
}

#[test]
#[ignore = "enable after fixing empty, zero-duration, and blank-only edit lists (FVID-0187)"]
fn empty_zero_duration_and_blank_only_edit_lists_are_rejected_and_valid_edits_play_exact_windows() {
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let expected = c["expected_error"].as_str().unwrap();
        let data = bytes(name);
        if name == "blank-only-video.mp4" {
            let root = RootMp4Reader::open(Cursor::new(&data), RootLimits::default()).unwrap();
            assert_eq!(root.tracks()[0].edits.len(), 2);
            let err = match NativeReader::software(Cursor::new(&data), 16 << 20) {
                Ok(_) => panic!("{name}: expected NativeReader refusal `{expected}`"),
                Err(e) => e,
            };
            assert!(
                matches!(err, fvid::Error::Invalid(ref msg) if msg.contains(expected)),
                "{name}: {err}"
            );
            let decode_err = decode_video(&fixture_dir().join(name)).unwrap_err();
            assert!(decode_err.contains(expected), "{name}: {decode_err}");
        } else {
            let root_err = match RootMp4Reader::open(Cursor::new(&data), RootLimits::default()) {
                Ok(_) => panic!("root {name}: expected error `{expected}`"),
                Err(e) => e,
            };
            assert!(
                matches!(root_err, fvid::Error::Invalid(ref msg) if msg.contains(expected)),
                "root {name}: {root_err}"
            );
            let owned_err = match OwnedMp4Reader::open(Cursor::new(&data), OwnedLimits::default()) {
                Ok(_) => panic!("owned {name}: expected error `{expected}`"),
                Err(e) => e,
            };
            assert!(
                !owned_err.is_unsupported() && owned_err.to_string().contains(expected),
                "owned {name}: {owned_err}"
            );
            let probe_err = probe_mp4(&fixture_dir().join(name)).unwrap_err();
            assert!(probe_err.contains(expected), "probe {name}: {probe_err}");
            let decode_err = decode_video(&fixture_dir().join(name)).unwrap_err();
            assert!(decode_err.contains(expected), "decode {name}: {decode_err}");
        }
    }

    assert!(
        AudioTimeline::new(
            &[Edit {
                duration: 0,
                media_time: 0
            }],
            3072,
            48_000,
            48_000,
            48_000
        )
        .is_err()
    );
    assert!(map_edits([(1024, -1), (1024, -1)], 48_000, 48_000).is_err());

    let trimmed_pcm_ref = bytes("expected-trimmed.f32le");
    let mut leading_silence_ref = vec![0u8; 1024 * 4];
    leading_silence_ref.extend_from_slice(&trimmed_pcm_ref);

    for c in cases("controls") {
        let name = c["file"].as_str().unwrap();
        let expected_frames = c["expected_frames"].as_u64().unwrap();
        let expected_samples = c["expected_samples"].as_u64().unwrap();
        let data = bytes(name);
        let path = fixture_dir().join(name);

        let root = RootMp4Reader::open(Cursor::new(&data), RootLimits::default()).unwrap();
        let owned = OwnedMp4Reader::open(Cursor::new(&data), OwnedLimits::default()).unwrap();
        assert_eq!(root.tracks().len(), 2, "{name}");
        assert_eq!(owned.tracks().len(), 2, "{name}");

        let stats = decode_video(&path).unwrap();
        assert_eq!(stats.video_frames, expected_frames, "{name}");

        let mut reader = NativeReader::software(Cursor::new(&data), 16 << 20).unwrap();
        let mut played = 0u64;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [16, 16], "{name}");
            played += 1;
        }
        assert_eq!(played, expected_frames, "{name}");

        let mut pcm = Vec::new();
        let audio_stats = fvid::native_media::decode_mp4_aac_pcm(&data, &mut pcm).unwrap();
        assert_eq!(audio_stats.sample_frames, expected_samples, "{name}");
        let expected_pcm = if expected_samples == 3072 {
            &leading_silence_ref
        } else {
            &trimmed_pcm_ref
        };
        assert_eq!(pcm.len(), expected_pcm.len(), "{name}");
        for (idx, (a, b)) in pcm
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected_pcm.as_chunks::<4>().0.iter())
            .enumerate()
        {
            let va = f32::from_le_bytes(*a);
            let vb = f32::from_le_bytes(*b);
            assert!((va - vb).abs() <= 1e-7, "{name} sample {idx}: {va} vs {vb}");
        }
    }
}
