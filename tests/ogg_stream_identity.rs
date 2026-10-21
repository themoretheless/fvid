use std::path::PathBuf;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/ogg-stream-identity")
            .join(name),
    )
    .unwrap()
}
#[test]
fn canonical_reader_keeps_healthy_identities_and_its_existing_bos_refusals() {
    for (name, serial) in [("normal.ogg", 0x46564920), ("zero-serial.ogg", 0)] {
        let parsed = fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()).unwrap();
        assert_eq!(parsed.chains().len(), 1);
        assert_eq!(parsed.chains()[0].serial, serial);
        assert_eq!(parsed.chains()[0].packets.len(), 4);
        let source =
            fvid_media::owned_ogg_opus::Program::open(bytes(name).as_slice(), Default::default())
                .unwrap();
        assert_eq!(
            source.duration(),
            Some(std::time::Duration::from_millis(40))
        );
    }
    for name in [
        "missing-bos-zero.ogg",
        "repeated-tags-bos.ogg",
        "repeated-audio-bos.ogg",
    ] {
        let error = match fvid_media::owned_ogg::Ogg::parse(&bytes(name), &Default::default()) {
            Ok(_) => panic!("{name}: existing canonical refusal changed"),
            Err(e) => e.to_string(),
        };
        assert!(
            error.contains("begins") || error.contains("began"),
            "{name}:{error}"
        );
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[cfg(feature = "player")]
#[test]
fn healthy_controls_keep_native_pcm_worker_navigation_and_owned_video() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes("video.mkv")), 32 << 20)
            .unwrap();
    assert!(video.read_frame_raw().unwrap().is_some());
    assert!(video.read_frame_raw().unwrap().is_none());
    for name in ["normal.ogg", "zero-serial.ogg"] {
        let make = || {
            fvid::playback_ogg_opus::OggOpusAudioReader::open(
                Cursor::new(bytes(name)),
                Default::default(),
            )
            .unwrap()
        };
        let mut decoder = fvid_opus::OpusHead::parse(&bytes("mono.head"))
            .unwrap()
            .decoder(48000)
            .unwrap();
        let mut expected = Vec::new();
        for n in 0..2 {
            let mut pcm = vec![0.; 5760];
            let frames = decoder
                .decode(&bytes("20ms.packet"), 5760, &mut pcm)
                .unwrap();
            assert_eq!(frames, 960);
            let data = pcm[..frames]
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>();
            expected.push((Duration::from_millis(n * 20), data));
        }
        assert_eq!(make().duration(), Some(Duration::from_millis(40)));
        let clipped = vec![
            (Duration::from_millis(10), expected[0].1[480 * 4..].to_vec()),
            expected[1].clone(),
        ];
        capture::worker_check_with_seek_expected(
            Box::new(make()),
            &expected,
            Duration::from_millis(10),
            &clipped,
        );
        capture::worker_check_with_seek(
            Box::new(make()),
            &expected,
            Some(Duration::from_millis(40)),
        );
    }
}
