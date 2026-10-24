use std::{
    fs,
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avi-headers-seek")
}

#[test]
fn duplicate_avi_headers_and_truncated_or_unaligned_idx1_are_rejected() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let expected_err = case["expected_error"].as_str().unwrap();
        let data = fs::read(root().join(file)).unwrap();

        let err = fvid::container::avi::Avi::parse(&data, &Default::default())
            .err()
            .unwrap_or_else(|| panic!("Avi::parse accepted {file}"));
        assert!(
            matches!(err, fvid::Error::Invalid(_)) && err.to_string().contains(expected_err),
            "{file}: {err:?}"
        );

        #[cfg(feature = "player")]
        {
            let audio_err = fvid::playback_avi_audio::AviAudioReader::open(
                data.as_slice(),
                Default::default(),
            )
            .err()
            .unwrap_or_else(|| panic!("AviAudioReader accepted {file}"));
            assert!(
                matches!(audio_err, fvid::Error::Invalid(_))
                    && audio_err.to_string().contains(expected_err),
                "{file}: {audio_err:?}"
            );
        }
    }
}

#[test]
fn delayed_start_avi_pcm_stream_seeks_and_decodes_deterministically() {
    let data = fs::read(root().join("valid-delayed-pcm.avi")).unwrap();
    let avi = fvid::container::avi::Avi::parse(&data, &Default::default()).unwrap();
    assert_eq!(avi.streams, 1);
    assert_eq!(avi.audio().len(), 1);
    assert_eq!(avi.audio()[0].start, 100);
    assert_eq!(avi.audio()[0].declared, 8);
    assert_eq!(avi.audio()[0].records.len(), 2);

    #[cfg(feature = "player")]
    {
        use fvid::audio::AudioStream;
        let mut reader = fvid::playback_avi_audio::AviAudioReader::open(
            data.as_slice(),
            Default::default(),
        )
        .unwrap();

        // Seeking before dwStart (100) must clamp to the stream's first sample (100)
        // and match the timestamp of the next emitted packet.
        assert_eq!(reader.seek_to(0), 100);
        let first = reader.next_packet().unwrap().unwrap();
        assert_eq!(first.pts, 100);
        assert_eq!(first.duration, 4);
        assert_eq!(
            first.data,
            [100i16, -100, 200, -200]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<u8>>()
        );

        // Seeking inside the second record (sample 106) lands at 106 and slices 2 frames.
        assert_eq!(reader.seek_to(106), 106);
        let tail = reader.next_packet().unwrap().unwrap();
        assert_eq!(tail.pts, 106);
        assert_eq!(tail.duration, 2);
        assert_eq!(
            tail.data,
            [400i16, -400]
                .into_iter()
                .flat_map(i16::to_le_bytes)
                .collect::<Vec<u8>>()
        );
        assert!(reader.next_packet().unwrap().is_none());

        // Seeking at or past the end (108) lands at 108 and reaches EOF immediately.
        assert_eq!(reader.seek_to(200), 108);
        assert!(reader.next_packet().unwrap().is_none());
    }
}
