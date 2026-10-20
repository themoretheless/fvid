#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn read(bytes: Vec<u8>) -> Vec<(Duration, Vec<u8>)> {
    let mut source =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    let mut decoder = source.make_decoder().unwrap();
    let mut packets = Vec::new();
    while let Some(p) = source.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
        {
            if let Some(p) = source
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                assert!(
                    p.data
                        .chunks_exact(4)
                        .all(|s| f32::from_le_bytes(s.try_into().unwrap()).is_finite())
                );
                packets.push((p.presentation_time(), p.data));
            }
        }
    }
    source.validate_eof().unwrap();
    packets
}
#[test]
fn vorbis_laces_keep_variable_overlap_clocks_instead_of_equal_block_subdivision() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/vorbis-lace-clock");
    let expected = read(std::fs::read(base.join("control.mkv")).unwrap());
    assert_eq!(
        expected.iter().map(|p| p.1.len() / 8).collect::<Vec<_>>(),
        vec![80, 80, 80, 128, 80]
    );
    assert_eq!(
        expected.iter().map(|p| p.0).collect::<Vec<_>>(),
        [0, 1_666_666, 3_333_333, 5_000_000, 7_666_666].map(Duration::from_nanos)
    );
    let mut wrong = Vec::new();
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    for mode in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
    {
        let bytes = std::fs::read(base.join(format!("{mode}.mkv"))).unwrap();
        let mut owned = fvid_media::owned_webm::WebmReader::open(
            Cursor::new(bytes.clone()),
            Default::default(),
        )
        .unwrap();
        owned.scan_all().unwrap();
        let indexed: Vec<_> = owned
            .packets
            .iter()
            .filter(|packet| packet.track == 2)
            .collect();
        assert_eq!(indexed.len(), 6);
        for (packet, stamp) in indexed
            .iter()
            .zip([0i64, 0, 1_666_666, 3_333_333, 5_000_000, 7_666_666])
        {
            assert!(
                packet.pts_ns.abs_diff(stamp) <= 1,
                "{mode} owned timestamp {} versus {stamp}",
                packet.pts_ns
            );
        }
        assert_eq!(
            indexed
                .iter()
                .map(|p| (u128::from(p.duration_ns.unwrap()) * 48000 + 500_000_000) / 1_000_000_000)
                .collect::<Vec<_>>(),
            vec![0, 80, 80, 80, 128, 80]
        );

        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        assert!(video.read_frame_raw().unwrap().is_some());
        assert!(video.read_frame_raw().unwrap().is_none());
        let mut wanted = expected.clone();
        if mode.ends_with("-tail-two") {
            let last = wanted.last_mut().unwrap();
            last.1.truncate(last.1.len() - 16);
        }
        let expected = &wanted;
        let actual = read(bytes.clone());
        assert_eq!(
            actual.iter().flat_map(|p| p.1.iter()).collect::<Vec<_>>(),
            expected.iter().flat_map(|p| p.1.iter()).collect::<Vec<_>>(),
            "native data control {mode}"
        );
        if actual != *expected {
            wrong.push(format!(
                "{mode}: {:?} versus {:?}",
                actual.iter().map(|p| p.0).collect::<Vec<_>>(),
                expected.iter().map(|p| p.0).collect::<Vec<_>>()
            ));
            continue;
        }
        let source = fvid::playback_webm_audio::WebmAudioReader::open(
            Cursor::new(bytes),
            Default::default(),
        )
        .unwrap();
        capture::worker_check_with_seek(Box::new(source), &expected, Some(Duration::ZERO));
    }
    assert!(
        wrong.is_empty(),
        "variable Vorbis laces were equally subdivided: {wrong:?}"
    );
}

#[test]
fn invalid_vorbis_lace_mode_is_a_precise_parser_refusal() {
    let bytes = include_bytes!("fixtures/playback-errors/vorbis-lace-clock/invalid-mode.mkv");
    let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
        Cursor::new(bytes.as_slice()),
        Default::default(),
    )
    .unwrap();
    let error = match source.next_packet() {
        Err(error) => error,
        Ok(_) => panic!("invalid mode accepted"),
    };
    assert!(
        error
            .to_string()
            .contains("Vorbis packet mode exceeds configured count"),
        "{error}"
    );
    let mut owned =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(bytes.as_slice()), Default::default())
            .unwrap();
    let error = owned.scan_all().unwrap_err();
    assert!(
        error
            .to_string()
            .contains("Vorbis packet mode exceeds configured count"),
        "{error}"
    );
}
