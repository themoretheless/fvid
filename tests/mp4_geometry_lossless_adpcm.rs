use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-geometry-lossless-adpcm")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&read("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn compare(actual: &[u8], expected: &[u8], name: &str) {
    assert_eq!(actual.len(), expected.len(), "{name}");
    let mut peak = 0f32;
    for (a, b) in actual.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        peak = peak.max(b.abs());
        assert!((a - b).abs() < 1e-6, "{name}: {a} != {b}");
    }
    assert!(peak > 1e-4);
}
#[test]
fn alac_raw_rice_and_adpcm_duration_clocks_normalize_the_exact_authored_signal() {
    for case in cases() {
        let name = case["name"].as_str().unwrap();
        let input = read(&format!("{name}.mp4"));
        let expected = read(case["oracle"].as_str().unwrap());
        let rate = case["rate"].as_u64().unwrap();
        let channels = case["channels"].as_u64().unwrap();
        for interval in [
            None,
            Some((Duration::from_micros(5501), Duration::from_micros(85507))),
        ] {
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&input),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap();
            let at = |d: Duration| {
                ((d.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize
                    * channels as usize
                    * 4)
                .min(expected.len())
            };
            let wanted = if let Some((a, b)) = interval {
                &expected[at(a)..at(b)]
            } else {
                &expected
            };
            compare(&pcm, wanted, name);
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (
                    rate as u32,
                    channels as u16,
                    (wanted.len() / 4) as u64 / channels
                )
            );
        }
    }
}
#[test]
fn alac_and_adpcm_file_exports_publish_only_the_normalized_pcm() {
    for case in cases().into_iter().filter(|c| {
        matches!(
            c["name"].as_str().unwrap(),
            "alac-rice-edits" | "alac-raw-third" | "adpcm-regular-2000-edits" | "adpcm-trim-48000"
        )
    }) {
        let name = case["name"].as_str().unwrap();
        let path = root().join(format!("{name}.mp4"));
        let plan =
            fvid_media::plan_decode_audio(&path, &Default::default(), &Default::default()).unwrap();
        assert_eq!(
            plan.streams[0].codec,
            if case["family"] == "alac" {
                "alac"
            } else {
                "adpcm_ima_qt"
            }
        );
        let dest = std::env::temp_dir().join(format!(
            "fvid-geometry-codecs-{}-{name}.wav",
            std::process::id()
        ));
        fvid_media::decode_audio(&path, &dest, &Default::default()).unwrap();
        let wave = std::fs::read(&dest).unwrap();
        std::fs::remove_file(dest).unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
        assert_eq!(
            (info.sample_rate, info.channels),
            (
                case["rate"].as_u64().unwrap() as u32,
                case["channels"].as_u64().unwrap() as u16
            )
        );
        compare(
            &wave[info.data_offset as usize
                ..(info.data_offset + u64::from(info.data_bytes)) as usize],
            &read(case["oracle"].as_str().unwrap()),
            name,
        );
    }
}
