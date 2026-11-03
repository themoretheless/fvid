use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-admission-catalog")
}
#[test]
fn catalog_size_and_selected_id_preserve_compact_index_and_exact_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let expected = std::fs::read(root().join("expected.f32le")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let source = root().join(c["file"].as_str().unwrap());
        let reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(std::fs::read(&source).unwrap()),
            fvid_media::owned_mp4::Limits {
                samples: 4,
                ..Default::default()
            },
        )
        .unwrap();
        let t = &reader.tracks()[1];
        assert_eq!(t.samples.len(), 16384);
        assert_eq!(t.samples.cost(), 1);
        assert_eq!(
            t.sample_descriptions.len(),
            c["descriptions"].as_u64().unwrap() as usize
        );
        for i in [0, 8192, 16383] {
            assert_eq!(
                t.samples.get(i).unwrap().description_index,
                c["selected"].as_u64().unwrap() as u32
            );
        }
        let opts = fvid_media::CopyOptions {
            max_controlled_bytes: Some(64 << 20),
            ..Default::default()
        };
        fvid_media::plan_decode_audio(&source, &Default::default(), &opts).unwrap();
        let mut out = Vec::new();
        let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::fs::File::open(&source).unwrap(),
            &mut out,
            None,
            &opts,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, 16384);
        assert_eq!(out, expected);
    }
}
