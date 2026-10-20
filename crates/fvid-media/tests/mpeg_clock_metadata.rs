use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("mpeg-clock-metadata/manifest.json")).unwrap(),
    )
    .unwrap()
}
fn same(a: &[u8], b: &[u8]) {
    assert_eq!(a.len(), b.len());
    for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 2e-6, "{a} versus {b}");
    }
}
#[test]
fn mpeg_library_nine_rate_raw_frames_and_quantized_metadata_windows_match_pcm_oracles() {
    let info = manifest();
    for base in info["bases"].as_array().unwrap() {
        let expected = std::fs::read(root().join(base["source_pcm"].as_str().unwrap())).unwrap();
        let source = root()
            .join("mpeg-clock-metadata")
            .join(base["raw"].as_str().unwrap());
        let mut pcm = Vec::new();
        let stats = fvid_media::owned_raw_mpeg::decode_raw_mpeg_pcm(
            &source,
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(stats.sample_frames, base["samples"].as_u64().unwrap());
        same(&pcm, &expected);
    }
    for case in info["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let rate = case["rate"].as_u64().unwrap();
        let width = case["channels"].as_u64().unwrap() as usize * 4;
        let start = case["start"].as_u64().unwrap() as usize;
        let end = case["end"].as_u64().unwrap() as usize;
        let golden = std::fs::read(root().join(case["source_pcm"].as_str().unwrap())).unwrap();
        let expected = &golden[start * width..end * width];
        let source = std::fs::read(
            root()
                .join("mpeg-clock-metadata")
                .join(format!("{name}.mkv")),
        )
        .unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(40), Duration::from_millis(50))),
        ] {
            let wanted = if interval.is_some() {
                &expected[(rate * 40).div_ceil(1000) as usize * width
                    ..(rate * 50).div_ceil(1000) as usize * width]
            } else {
                expected
            };
            let mut pcm = Vec::new();
            let stats = fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                Cursor::new(&source),
                &mut pcm,
                interval,
                &Default::default(),
            )
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stats.sample_frames, wanted.len() as u64 / width as u64);
            same(&pcm, wanted);
        }
    }
}

#[test]
fn fractional_mpeg_block_grids_export_exact_source_samples() {
    let info = manifest();
    for layer in [2, 3] {
        for scale in ["double", "third", "shift-third"] {
            let base = info["bases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["layer"] == layer && c["rate"] == 44100)
                .unwrap();
            let expected =
                std::fs::read(root().join(base["source_pcm"].as_str().unwrap())).unwrap();
            let stride = base["channels"].as_u64().unwrap() as usize * 4;
            let movie = std::fs::read(root().join(format!(
                "mpeg-clock-metadata/mp{layer}-44100-scaled-{scale}.mkv"
            )))
            .unwrap();
            for interval in [
                None,
                Some((Duration::from_millis(30), Duration::from_millis(40))),
            ] {
                let wanted = if interval.is_some() {
                    &expected[(30u64 * 44100).div_ceil(1000) as usize * stride
                        ..(40u64 * 44100).div_ceil(1000) as usize * stride]
                } else {
                    &expected
                };
                let mut out = Vec::new();
                let stats = fvid_media::owned_matroska_mpeg::decode_matroska_mpeg_pcm(
                    Cursor::new(&movie),
                    &mut out,
                    interval,
                    &Default::default(),
                )
                .unwrap();
                same(&out, wanted);
                assert_eq!(stats.sample_frames, wanted.len() as u64 / stride as u64);
            }
        }
    }
}
