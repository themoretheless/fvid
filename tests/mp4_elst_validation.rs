use fvid::container::audio_timeline::AudioTimeline;
use fvid::container::mp4::{Edit, Limits as RootLimits, Mp4Reader as RootMp4Reader};
use fvid::playback_native::NativeReader;
use fvid_media::decode_video;
use fvid_media::owned_mp4::{Limits as OwnedLimits, Mp4Reader as OwnedMp4Reader};
use fvid_media::owned_mp4_probe::probe_mp4;
use fvid_media::owned_video_timeline::map_edits;
use std::{io::Cursor, path::PathBuf, time::Duration};

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
fn empty_and_zero_duration_elst_boxes_are_rejected_across_root_and_owned_parsers() {
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        if name == "blank-only-video.mp4" {
            continue;
        }
        let expected = c["expected_error"].as_str().unwrap();
        let data = bytes(name);
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

#[test]
fn blank_only_video_edit_lists_and_zero_duration_audio_timelines_are_refused() {
    let data = bytes("blank-only-video.mp4");
    let root = RootMp4Reader::open(Cursor::new(&data), RootLimits::default()).unwrap();
    let owned = OwnedMp4Reader::open(Cursor::new(&data), OwnedLimits::default()).unwrap();
    assert_eq!(root.tracks()[0].edits.len(), 2);
    assert_eq!(owned.tracks()[0].edits.len(), 2);

    let err = match NativeReader::software(Cursor::new(&data), 16 << 20) {
        Ok(_) => panic!("blank-only-video.mp4: expected NativeReader refusal"),
        Err(e) => e,
    };
    assert!(
        matches!(err, fvid::Error::Invalid(ref msg) if msg.contains("empty MP4 playback edit")),
        "{err}"
    );
    let decode_err = decode_video(&fixture_dir().join("blank-only-video.mp4")).unwrap_err();
    assert!(
        decode_err.contains("empty MP4 playback edit"),
        "{decode_err}"
    );
    assert!(map_edits([(1024, -1), (1024, -1)], 48_000, 48_000).is_err());

    let audio_err = AudioTimeline::new(
        &[Edit {
            duration: 0,
            media_time: 0,
        }],
        3072,
        48_000,
        48_000,
        48_000,
    )
    .err()
    .unwrap();
    assert!(
        matches!(audio_err, fvid::Error::Invalid(ref msg) if msg.contains("empty MP4 audio edit")),
        "{audio_err}"
    );
}

#[test]
fn valid_v0_and_v1_edit_lists_preserve_exact_video_frames_and_analytical_aac_pcm() {
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

        let info = probe_mp4(&path).unwrap();
        assert_eq!(info.streams.len(), 2, "{name}");
        assert_eq!(
            info.duration_us,
            Some((expected_samples * 1_000_000 / 48_000) as i64),
            "{name}"
        );

        let stats = decode_video(&path).unwrap();
        assert_eq!(stats.video_frames, expected_frames, "{name}");
        assert_eq!((stats.width, stats.height), (16, 16), "{name}");

        let mut reader = NativeReader::software(Cursor::new(&data), 16 << 20).unwrap();
        let mut played = 0u64;
        while reader.read_frame().unwrap() {
            assert_eq!(reader.dimensions(), [16, 16], "{name}");
            played += 1;
        }
        assert_eq!(played, expected_frames, "{name}");
        reader.rewind().unwrap();
        assert!(reader.seek(Duration::from_millis(10)).is_ok(), "{name}");

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

#[cfg(feature = "player")]
#[test]
fn player_audio_stream_handles_valid_edited_aac_and_seeks_cleanly() {
    use fvid::audio::AudioStream;
    let trimmed_pcm_ref = bytes("expected-trimmed.f32le");
    let mut leading_silence_ref = vec![0u8; 1024 * 4];
    leading_silence_ref.extend_from_slice(&trimmed_pcm_ref);

    for c in cases("controls") {
        let name = c["file"].as_str().unwrap();
        let expected_samples = c["expected_samples"].as_u64().unwrap();
        let data = bytes(name);
        let mut stream =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), RootLimits::default())
                .unwrap();
        assert_eq!(stream.sample_rate(), 48_000, "{name}");
        assert_eq!(stream.channels(), 1, "{name}");
        assert_eq!(
            stream.duration(),
            Some(Duration::from_nanos(
                expected_samples * 1_000_000_000 / 48_000
            )),
            "{name}"
        );
        let landed = stream.seek_to(1024);
        assert!(landed >= 1024, "{name}: landed {landed}");
        AudioStream::rewind(&mut stream);
        assert_eq!(stream.seek_to(0), if expected_samples == 2048 { 1024 } else { 0 });
    }
}
