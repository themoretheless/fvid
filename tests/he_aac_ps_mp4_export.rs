//! Original PS export repeats exercise retained lookahead, source EOF and checkpoints.
use fvid_control::CopyOptions;
use std::{io::Cursor, time::Duration};
#[test]
fn original_mp4_ps_edits_export_independent_pcm_and_restore_pending_checkpoint_metadata() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-mp4-export-oracles.json"
    ))
    .unwrap();
    let references: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap();
    let data = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
    for c in manifest["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["file"].as_str().unwrap());
        let file = std::fs::read(&path).unwrap();
        let mut source = vec![];
        for frame in references["cases"][c["reference"].as_u64().unwrap() as usize]["frames"]
            .as_array()
            .unwrap()
        {
            let n = frame["Double"][0][1].as_u64().unwrap() as usize;
            for i in 0..n {
                for channel in 0..2 {
                    let off = frame["Double"][channel][0].as_u64().unwrap() as usize + i * 8;
                    source.push(
                        (f64::from_le_bytes(data[off..off + 8].try_into().unwrap()) / 32768.)
                            as f32,
                    );
                }
            }
        }
        let mut expected = vec![0.; 1600 * 2];
        expected.extend_from_slice(&source[1920 * 2..5120 * 2]);
        expected.extend_from_slice(&source[1920 * 2..5120 * 2]);
        expected.extend_from_slice(&vec![0.; 1600 * 2]);
        for (interval, options, from, to) in [
            (None, CopyOptions::default(), 0, 9600),
            (
                Some((Duration::from_millis(50), Duration::from_millis(180))),
                CopyOptions::default(),
                2400,
                8640,
            ),
            (
                None,
                CopyOptions {
                    max_packets: Some(3),
                    ..Default::default()
                },
                0,
                4800,
            ),
            (
                None,
                CopyOptions {
                    max_controlled_bytes: Some(128 * 1024 * 1024),
                    ..Default::default()
                },
                0,
                9600,
            ),
        ] {
            let mut output = vec![];
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&file),
                &mut output,
                interval,
                &options,
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (48000, 2, (to - from) as u64)
            );
            assert_eq!(output.len(), (to - from) * 8);
            for (bytes, e) in output.chunks_exact(4).zip(&expected[from * 2..to * 2]) {
                let a = f32::from_le_bytes(bytes.try_into().unwrap());
                assert!(
                    (a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                    "{}: {a} != {e}",
                    c["file"]
                );
            }
            if options.max_packets.is_none() {
                let reader =
                    fvid::container::mp4::Mp4Reader::open(Cursor::new(&file), Default::default())
                        .unwrap();
                let mut root = vec![];
                let stats =
                    fvid::native_media::decode_mp4_aac_reader(reader, &mut root, interval).unwrap();
                assert_eq!(stats.sample_frames, (to - from) as u64);
                assert_eq!(root, output);
            }
        }
        let destination = std::env::temp_dir().join(format!(
            "fvid-mp4-ps-export-{}-{}.wav",
            std::process::id(),
            c["reference"]
        ));
        let _ = std::fs::remove_file(&destination);
        let stats = fvid_media::decode_audio(&path, &destination, &CopyOptions::default()).unwrap();
        assert_eq!(stats.sample_frames, 9600);
        std::fs::remove_file(destination).unwrap();
    }
}

#[test]
fn unequal_ps_packet_windows_and_early_interval_use_original_pending_source_metadata() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-playback-oracles.json"
    ))
    .unwrap();
    let references: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap();
    let data = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
    for c in manifest["cases"].as_array().unwrap() {
        let reference = references["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| {
                r["source"]["name"] == c["source_reference"]
                    && r["zero_eof"] == true
                    && r["source"]["kind"]
                        .as_str()
                        .unwrap()
                        .starts_with("sbr-video")
            })
            .unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let file = std::fs::read(path).unwrap();
        let mut expected = vec![];
        for (frame, duration) in reference["frames"]
            .as_array()
            .unwrap()
            .iter()
            .zip(c["durations"].as_array().unwrap())
        {
            for n in 0..duration.as_u64().unwrap() as usize {
                for channel in 0..2 {
                    let at = frame["Double"][channel][0].as_u64().unwrap() as usize + n * 8;
                    expected.push(
                        (f64::from_le_bytes(data[at..at + 8].try_into().unwrap()) / 32768.) as f32,
                    );
                }
            }
        }
        for (interval, from, to) in [
            (None, 0, expected.len() / 2),
            (Some((Duration::ZERO, Duration::from_millis(20))), 0, 960),
            (
                Some((Duration::from_millis(1), Duration::from_millis(42))),
                48,
                2016,
            ),
        ] {
            let mut output = vec![];
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&file),
                &mut output,
                interval,
                &CopyOptions::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_frames, (to - from) as u64);
            assert_eq!(output.len(), (to - from) * 8);
            for (bytes, e) in output.chunks_exact(4).zip(&expected[from * 2..to * 2]) {
                let a = f32::from_le_bytes(bytes.try_into().unwrap());
                assert!((a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16);
            }
            let reader =
                fvid::container::mp4::Mp4Reader::open(Cursor::new(&file), Default::default())
                    .unwrap();
            let mut root = vec![];
            fvid::native_media::decode_mp4_aac_reader(reader, &mut root, interval).unwrap();
            assert_eq!(root, output);
        }
    }
}

#[test]
fn invalid_source_range_is_reported_only_after_pending_ps_eof_pcm_is_drained() {
    for (size, frames) in [(960, 1920usize), (1024, 2048usize)] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/he-aac-ps-worker-source-gap-{size}-synthetic.mp4"
        ));
        let file = std::fs::read(path).unwrap();
        let mut output = vec![];
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&file),
            &mut output,
            None,
            &CopyOptions::default(),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "audio edit extends outside available samples"
        );
        assert_eq!(
            output.len(),
            (frames * 3 - 720) * 8,
            "pending EOF PCM must precede the source gap refusal"
        );
        let reader =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&file), Default::default()).unwrap();
        let mut root = vec![];
        let error = fvid::native_media::decode_mp4_aac_reader(reader, &mut root, None).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("audio edit extends outside available samples")
        );
        assert_eq!(root, output);
    }
}
