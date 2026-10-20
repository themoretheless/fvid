use fvid_media::{owned_ogg::Coding, owned_ogg_audio_catalog as catalog};
#[test]
fn mixed_audio_catalogue_preserves_bos_order_and_codec_ordinals() {
    for (name, expected) in [
        ("ogg-mixed-opus-first.ogg", [Coding::Opus, Coding::Vorbis]),
        ("ogg-mixed-vorbis-first.ogg", [Coding::Vorbis, Coding::Opus]),
    ] {
        let bytes = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../tests/fixtures/playback-errors")
                .join(name),
        )
        .unwrap();
        let tracks = catalog::open(bytes.as_slice(), Default::default()).unwrap();
        assert_eq!(tracks.len(), 2);
        for (i, track) in tracks.iter().enumerate() {
            assert_eq!(track.coding, expected[i]);
            assert_eq!(track.source_index, i);
            assert_eq!(track.codec_ordinal, 0);
            assert_eq!(
                track.audio.sample_rate,
                if expected[i] == Coding::Opus {
                    48000
                } else {
                    8000
                }
            );
        }
    }
}

#[test]
fn mixed_ogg_file_export_selects_both_source_codecs() {
    struct Directory(std::path::PathBuf);
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let directory = Directory(
        std::env::temp_dir().join(format!("fvid-mixed-ogg-export-{}", std::process::id())),
    );
    std::fs::create_dir(&directory.0).unwrap();
    for (name, opus) in [
        ("ogg-mixed-opus-first.ogg", 0),
        ("ogg-mixed-vorbis-first.ogg", 1),
    ] {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/playback-errors")
            .join(name);
        for index in 0..2 {
            let output = directory.0.join(format!("{name}-{index}.wav"));
            let stats = fvid_media::decode_audio(
                &source,
                &output,
                &fvid_control::CopyOptions {
                    streams: vec![index],
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(stats.sample_frames, if index == opus { 5311 } else { 411 });
        }
        assert!(
            fvid_media::decode_audio(
                &source,
                &directory.0.join(format!("{name}-ambiguous.wav")),
                &Default::default()
            )
            .unwrap_err()
            .contains("select exactly one")
        );
    }
}
