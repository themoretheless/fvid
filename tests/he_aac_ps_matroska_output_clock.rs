//! Own PS Matroska export keeps source timing while draining delayed PCM.
use fvid_control::CopyOptions;
use fvid_media::owned_matroska_aac::decode_matroska_aac_pcm;
use std::{io::Cursor, time::Duration};
#[test]
fn matroska_output_sampling_frequency_selects_independent_ps_pcm_and_wav_clock() {
    let mut manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-output-clock-matroska-oracles.json"
    ))
    .unwrap();
    let asc_clock: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-asc-clock-matroska-oracles.json"
    ))
    .unwrap();
    manifest["cases"]
        .as_array_mut()
        .unwrap()
        .extend(asc_clock["cases"].as_array().unwrap().iter().cloned());
    let oracle = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    for c in manifest["cases"].as_array().unwrap() {
        let file = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["file"].as_str().unwrap()),
        )
        .unwrap();
        let reader =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&file), Default::default())
                .unwrap();
        assert_eq!(
            reader
                .tracks
                .iter()
                .find(|t| t.codec == "A_AAC")
                .unwrap()
                .sample_rate,
            48000
        );
        let frames = c["slots"].as_u64().unwrap() as usize * 128;
        let mut expected = vec![0f32; (4800 + frames) * 2];
        for frame in 0..3 {
            let start = c["frame_starts"][frame].as_u64().unwrap() as usize;
            for channel in 0..2 {
                let offset = c["pcm"][channel][0].as_u64().unwrap() as usize;
                for n in 0..frames {
                    let at = offset + (frame * frames + n) * 8;
                    expected[(start + n) * 2 + channel] =
                        f64::from_le_bytes(oracle[at..at + 8].try_into().unwrap()) as f32;
                }
            }
        }
        for (interval, options, from, to, count) in [
            (None, CopyOptions::default(), 0, 4800 + frames, 3),
            (
                Some((Duration::from_millis(1), Duration::from_millis(118))),
                CopyOptions::default(),
                48,
                5664,
                3,
            ),
            (
                None,
                CopyOptions {
                    max_packets: Some(2),
                    ..Default::default()
                },
                0,
                2400 + frames,
                2,
            ),
            (
                None,
                CopyOptions {
                    max_controlled_bytes: Some(64 * 1024 * 1024),
                    ..Default::default()
                },
                0,
                4800 + frames,
                3,
            ),
        ] {
            let mut output = vec![];
            let stats =
                decode_matroska_aac_pcm(Cursor::new(&file), &mut output, interval, &options)
                    .unwrap();
            assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
            assert_eq!(stats.decoded_frames, count);
            assert_eq!(stats.sample_frames, (to - from) as u64);
            assert_eq!(output.len(), (to - from) * 8);
            if options.max_packets.is_none() {
                let reader =
                    fvid::container::webm::WebmReader::open(Cursor::new(&file), Default::default())
                        .unwrap();
                let mut root = vec![];
                let root_stats =
                    fvid::native_media::decode_matroska_aac_reader(reader, &mut root, interval)
                        .unwrap();
                assert_eq!(root_stats.sample_frames, stats.sample_frames);
                assert_eq!(root, output);
            }
            for (bytes, e) in output.chunks_exact(4).zip(&expected[from * 2..to * 2]) {
                let a = f32::from_le_bytes(bytes.try_into().unwrap());
                assert!(
                    (a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                    "{}: {a} != {e}",
                    c["file"]
                );
            }
        }
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["file"].as_str().unwrap());
        let destination = std::env::temp_dir().join(format!(
            "fvid-ps-export-{}-{}.wav",
            std::process::id(),
            frames
        ));
        let _ = std::fs::remove_file(&destination);
        let stats =
            fvid_media::decode_audio(&source, &destination, &CopyOptions::default()).unwrap();
        assert_eq!(stats.sample_frames, (4800 + frames) as u64);
        let mut wave = std::fs::File::open(&destination).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut wave, None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels, info.sample_frames),
            (48000, 2, (4800 + frames) as u64)
        );
        std::fs::remove_file(destination).unwrap();
        for budget in [1, 10 * 1024 * 1024] {
            let mut output = vec![];
            let error = decode_matroska_aac_pcm(
                Cursor::new(&file),
                &mut output,
                None,
                &CopyOptions {
                    max_controlled_bytes: Some(budget),
                    ..Default::default()
                },
            )
            .unwrap_err();
            assert!(error.to_string().contains("controlled memory budget"));
            assert!(output.is_empty());
        }
    }
}

#[test]
fn invalid_matroska_output_sampling_frequency_is_refused_before_decode() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-output-clock-matroska-oracles.json"
    ))
    .unwrap();
    for row in manifest["invalid"].as_array().unwrap() {
        let file = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(row["file"].as_str().unwrap()),
        )
        .unwrap();
        let owned = match fvid_media::owned_webm::WebmReader::open(
            Cursor::new(&file),
            Default::default(),
        ) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("invalid output frequency accepted"),
        };
        let root =
            match fvid::container::webm::WebmReader::open(Cursor::new(&file), Default::default()) {
                Err(e) => e.to_string(),
                Ok(_) => panic!("invalid output frequency accepted"),
            };
        assert!(owned.contains(row["error"].as_str().unwrap()));
        assert!(root.contains(row["error"].as_str().unwrap()));
    }
}

#[test]
fn asc_clock_inference_preserves_explicit_output_conflicts_and_unspecified_lc() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-asc-clock-matroska-oracles.json"
    ))
    .unwrap();
    for c in manifest["controls"].as_array().unwrap() {
        let file = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors")
                .join(c["file"].as_str().unwrap()),
        )
        .unwrap();
        let owned =
            fvid_media::owned_webm::WebmReader::open(Cursor::new(&file), Default::default())
                .unwrap();
        let root = fvid::container::webm::WebmReader::open(Cursor::new(&file), Default::default())
            .unwrap();
        for rate in [
            owned
                .tracks
                .iter()
                .find(|t| t.codec == "A_AAC")
                .unwrap()
                .sample_rate,
            root.tracks
                .iter()
                .find(|t| t.codec == "A_AAC")
                .unwrap()
                .sample_rate,
        ] {
            assert_eq!(rate, c["sample_rate"].as_u64().unwrap(), "{}", c["file"]);
        }
    }
}
