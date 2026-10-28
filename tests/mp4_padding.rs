use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-padding")
}
#[test]
fn malformed_padding_movies_are_rejected_at_the_specific_padding_table() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let expected = case["error"].as_str().unwrap();
        match fvid_media::owned_mp4::Mp4Reader::open_indexed(
            Cursor::new(&bytes),
            Default::default(),
        ) {
            Ok(_) => panic!("{} ignored malformed padb", case["video"]),
            Err(e) => assert!(e.to_string().contains(expected), "{}: {e}", case["video"]),
        }
        let mut output = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error.to_string().contains(expected),
            "export {}: {error}",
            case["video"]
        );
        assert!(output.is_empty());
        #[cfg(feature = "player")]
        match fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(&bytes),
            Default::default(),
        ) {
            Ok(_) => panic!("{} player ignored malformed padb", case["video"]),
            Err(e) => assert!(
                e.to_string().contains(expected),
                "player {}: {e}",
                case["video"]
            ),
        }
        match fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
        {
            Ok(_) => panic!("{} public parser ignored malformed padb", case["video"]),
            Err(e) => assert!(e.to_string().contains(expected), "{}: {e}", case["video"]),
        }
    }
}
#[test]
fn indexed_and_fragment_padding_preserve_raw_bytes_and_exact_bit_lengths() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let seed = std::fs::read(root().join("reference.mp4")).unwrap();
    let bytes = std::fs::read(root().join("padding.mp4")).unwrap();
    let mut original =
        fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(seed), Default::default())
            .unwrap();
    let mut reader =
        fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
            .unwrap();
    let public =
        fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
            .unwrap();
    let audio = reader
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    for (i, pad) in m["padding"].as_array().unwrap().iter().enumerate() {
        let expected =
            m["sizes"][i].as_u64().unwrap() as usize * 8 - pad.as_u64().unwrap() as usize;
        assert_eq!(reader.packet_bit_len(audio, i).unwrap(), expected);
        assert_eq!(public.packet_bit_len(audio, i).unwrap(), expected);
        let mut raw = Vec::new();
        let mut reference = Vec::new();
        reader.read_packet(audio, i, &mut raw).unwrap();
        original.read_packet(audio, i, &mut reference).unwrap();
        assert_eq!(raw, reference);
        assert_eq!(
            original.packet_bit_len(audio, i).unwrap(),
            reference.len() * 8
        );
    }
    assert!(
        reader
            .packet_bit_len(audio, reader.tracks()[audio].samples.len())
            .is_err()
    );
    for row in m["fragments"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(row["video"].as_str().unwrap())).unwrap();
        let reader =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
                .unwrap();
        let public =
            fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!(
            reader.tracks()[0].samples.len(),
            row["padding"].as_array().unwrap().len()
        );
        for (i, pad) in row["padding"].as_array().unwrap().iter().enumerate() {
            let expected =
                row["bytes"].as_u64().unwrap() as usize * 8 - pad.as_u64().unwrap() as usize;
            assert_eq!(
                reader.packet_bit_len(0, i).unwrap(),
                expected,
                "{} sample {i}",
                row["video"]
            );
            assert_eq!(public.packet_bit_len(0, i).unwrap(), expected);
        }
    }
}
