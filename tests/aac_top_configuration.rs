use fvid::{
    codec::{aac_native::NativeAacDecoder, config::AacConfig},
    container::webm::WebmReader,
};
use std::{io::Cursor, time::Duration};

fn cases() -> serde_json::Value {
    serde_json::from_slice(include_bytes!(
        "fixtures/playback-errors/aac-top-config-generated.json"
    ))
    .unwrap()
}
fn artifact(case: &serde_json::Value, suffix: &str) -> Vec<u8> {
    let file = case["artifacts"][suffix]["file"].as_str().unwrap();
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(file),
    )
    .unwrap()
}

#[test]
fn indexed_top_configuration_matches_independent_pcm_at_both_frame_sizes() {
    for (name, case) in cases()["cases"].as_object().unwrap() {
        let source = artifact(case, ".mka");
        let oracle = artifact(case, ".f32le");
        let config =
            AacConfig::parse(&artifact(case, ".asc")).unwrap_or_else(|e| panic!("{name}: {e}"));
        let size = case["samples"].as_u64().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        assert_eq!(
            (
                config.channel_configuration,
                config.channels,
                u64::from(config.frame_samples),
                u64::from(config.sample_rate)
            ),
            (14, 8, size, rate)
        );
        let mut actual = Vec::new();
        let stats =
            fvid::native_media::decode_matroska_aac_pcm_interval(&source, &mut actual, None)
                .unwrap();
        assert_eq!(
            (stats.channels, stats.sample_frames, stats.decoded_frames),
            (8, size * 6, 6),
            "{name}"
        );
        assert_eq!(actual.len(), oracle.len(), "{name}");
        let mut peak = 0.0f64;
        let mut energy = [0.0f64; 8];
        for (i, (a, b)) in actual
            .as_chunks::<4>()
            .0
            .iter()
            .zip(oracle.as_chunks::<4>().0)
            .enumerate()
        {
            let a = f32::from_le_bytes(*a);
            let b = f32::from_le_bytes(*b);
            assert!(a.is_finite());
            peak = peak.max(f64::from((a - b).abs()));
            energy[i % 8] += f64::from(b).powi(2);
        }
        assert!(energy.iter().all(|e| *e > 1e-6), "{name}: {energy:?}");
        assert!(peak < 2e-6, "{name}: peak {peak}");
        // Fractional sample boundaries exercise preroll and the public timeline.
        let from = Duration::from_micros(30001);
        let to = Duration::from_micros(65001);
        let mut part = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&source, &mut part, Some((from, to)))
            .unwrap();
        let first = (from.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize;
        let last = (to.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize;
        assert_eq!(part, actual[first * 32..last * 32], "{name}");
    }
}

#[test]
fn indexed_top_mask_reset_and_checkpoint_preserve_every_channel() {
    for (name, case) in cases()["cases"].as_object().unwrap() {
        let mut reader =
            WebmReader::open(Cursor::new(artifact(case, ".mka")), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let mut decoder = NativeAacDecoder::new(&artifact(case, ".asc")).unwrap();
        assert_eq!(decoder.channel_mask(), 0x503f, "{name}");
        assert!(decoder.channel_positions().unwrap().is_none());
        assert_eq!(reader.packets.len(), 6);
        let mut horizontal_asc = artifact(case, ".asc");
        horizontal_asc[1] = (horizontal_asc[1] & 0x87) | (12 << 3);
        let mut horizontal = NativeAacDecoder::new(&horizontal_asc).unwrap();
        assert!(
            horizontal
                .decode(&reader.read_packet(0).unwrap())
                .unwrap_err()
                .to_string()
                .contains("element order differs")
        );
        assert!(
            decoder
                .restore(&horizontal.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("configuration mismatch")
        );
        let mut expected = Vec::new();
        for i in 0..6 {
            expected.push(decoder.decode(&reader.read_packet(i).unwrap()).unwrap());
        }
        decoder.reset();
        assert_eq!(
            decoder.decode(&reader.read_packet(0).unwrap()).unwrap(),
            expected[0]
        );
        let checkpoint = decoder.checkpoint();
        for i in 1..6 {
            assert_eq!(
                decoder.decode(&reader.read_packet(i).unwrap()).unwrap(),
                expected[i]
            );
        }
        decoder.restore(&checkpoint).unwrap();
        let packet = reader.read_packet(1).unwrap();
        for length in 0..packet.len() {
            assert!(
                decoder.decode(&packet[..length]).is_err(),
                "{name} length {length}"
            );
            assert_eq!(decoder.decode(&packet).unwrap(), expected[1]);
            decoder.restore(&checkpoint).unwrap();
        }
    }
}

#[test]
fn synthetic_video_matches_all_six_audio_intervals() {
    for case in cases()["cases"].as_object().unwrap().values() {
        let video = artifact(case, ".y4m");
        let mut output = Vec::new();
        let stats = fvid::y4m::process(
            Cursor::new(&video),
            &mut output,
            Default::default(),
            1 << 20,
        )
        .unwrap();
        assert_eq!(stats.frames, 6);
        assert_eq!(output, video);
    }
}

#[test]
fn explicit_pce_baselines_prove_the_same_nonzero_packets_and_speakers() {
    for (name, case) in cases()["cases"].as_object().unwrap() {
        let baseline = artifact(case, ".baseline.mka");
        let mut reader = WebmReader::open(Cursor::new(&baseline), Default::default()).unwrap();
        reader.scan_all().unwrap();
        let decoder = NativeAacDecoder::new(&reader.tracks[0].codec_private).unwrap();
        assert_eq!(decoder.channel_mask(), 0x503f, "{name}");
        let mut pcm = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&baseline, &mut pcm, None).unwrap();
        let oracle = artifact(case, ".f32le");
        assert_eq!(pcm.len(), oracle.len());
        let peak = pcm
            .as_chunks::<4>()
            .0
            .iter()
            .zip(oracle.as_chunks::<4>().0)
            .map(|(a, b)| (f32::from_le_bytes(*a) - f32::from_le_bytes(*b)).abs())
            .fold(0.0f32, f32::max);
        assert!(peak < 2e-6, "{name}: {peak}");
        assert!(
            pcm.as_chunks::<4>()
                .0
                .iter()
                .any(|b| f32::from_le_bytes(*b).abs() > 0.00001)
        );
        let mut indexed =
            WebmReader::open(Cursor::new(artifact(case, ".mka")), Default::default()).unwrap();
        indexed.scan_all().unwrap();
        for i in 0..6 {
            assert_eq!(
                indexed.read_packet(i).unwrap(),
                reader.read_packet(i).unwrap()
            );
        }
    }
}

#[test]
fn owned_wave_export_preserves_top_front_mask_and_continuous_pcm() {
    let directory =
        std::env::temp_dir().join(format!("fvid-top-config-export-{}", std::process::id()));
    std::fs::create_dir(&directory).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    for (name, case) in cases()["cases"].as_object().unwrap() {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["artifacts"][".mka"]["file"].as_str().unwrap());
        let output = directory.join(format!("{name}.wav"));
        fvid_media::decode_audio(&source, &output, &Default::default()).unwrap();
        let mut file = std::fs::File::open(&output).unwrap();
        let wave = fvid::native_pcm::inspect(&mut file, None).unwrap();
        assert_eq!(
            (wave.channels, wave.channel_mask, wave.sample_frames),
            (8, 0x503f, case["samples"].as_u64().unwrap() * 6),
            "{name}"
        );
        let mut pcm = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(
            &artifact(case, ".mka"),
            &mut pcm,
            None,
        )
        .unwrap();
        assert!(std::fs::read(output).unwrap().ends_with(&pcm), "{name}");
    }
}
