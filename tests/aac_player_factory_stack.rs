//! A real own short video must decode through the player factory on a normal
//! worker stack. Run in a child so a stack-overflow regression is reported.
#[cfg(feature = "player")]
#[test]
fn ps_factory_fits_two_mib_stack() {
    const CHILD: &str = "FVID_PS_FACTORY_STACK_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "ps_factory_fits_two_mib_stack", "--nocapture"])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "player factory stack regression: {status}"
        );
        return;
    }
    std::thread::Builder::new()
        .name("ps-factory".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            use fvid::audio::AudioStream;
            use std::io::Cursor;
            let headroom = [0u8; 128 * 1024];
            std::hint::black_box(&headroom);
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/playback-errors/aac-player-factory-stack");
            let mp4 = std::fs::read(root.join("ps-er-960.mp4")).unwrap();
            let mkv = std::fs::read(root.join("ps-er-960.mka")).unwrap();
            let mut expected = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut expected).unwrap();
            assert_eq!(expected.len(), 4 * 1920 * 2 * 4);
            assert!(
                expected
                    .chunks_exact(4)
                    .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 1e-8)
            );
            let mut readers: Vec<Box<dyn AudioStream>> = vec![
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        Cursor::new(mp4),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        Cursor::new(mkv),
                        Default::default(),
                    )
                    .unwrap(),
                ),
            ];
            for reader in &mut readers {
                let mut decoder = reader.make_decoder().unwrap();
                let mut actual = Vec::new();
                while let Some(packet) = reader.next_packet().unwrap() {
                    if let Some(frame) = decoder
                        .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                        .unwrap()
                    {
                        if let Some(pcm) = reader
                            .present_decoded(frame.packet, frame.source_pts)
                            .unwrap()
                        {
                            actual.extend(pcm.data);
                        }
                    }
                }
                while let Some(frame) = decoder.finish_packet().unwrap() {
                    if let Some(pcm) = reader
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        actual.extend(pcm.data);
                    }
                }
                assert_eq!(actual, expected);
            }
            std::hint::black_box(&headroom);
        })
        .unwrap()
        .join()
        .unwrap();
}
