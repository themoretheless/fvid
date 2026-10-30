//! Native AAC-ELD spatial playback, speaker layout and checkpoint acceptance.
fn movie(tree: usize) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/aac-eld-mps-core/tree{tree}.mp4"
        )),
    )
    .unwrap()
}
#[test]
fn actual_eld_core_spatial_movies_match_public_and_owned_pcm() {
    for tree in 0..8 {
        let movie = movie(tree);
        let mut pcm = Vec::new();
        let mut owned = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie, &mut pcm).unwrap();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(movie),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(pcm, owned);
        let channels = if tree == 7 {
            2
        } else if tree <= 2 {
            6
        } else {
            8
        };
        assert_eq!(pcm.len(), 3 * 512 * channels * 4);
    }
}
#[test]
fn actual_eld_core_spatial_movies_play_all_eight_trees() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-mps-core");
    for tree in 0..8 {
        let asc = std::fs::read(root.join(format!("tree{tree}.spatial.asc"))).unwrap();
        let native = fvid_media::owned_aac::NativeAacDecoder::new(&asc).unwrap();
        let masks = [0x3f, 0x3f, 0x3f, 0xff, 0x63f, 0xff, 0x63f, 3];
        let outputs = [6, 6, 6, 8, 8, 8, 8, 2];
        let inputs = [1, 1, 2, 2, 2, 6, 6, 1];
        assert_eq!(
            (native.channels(), native.channel_mask()),
            (outputs[tree], masks[tree])
        );
        let parsed = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(parsed.output_channels(), outputs[tree]);
        assert_eq!(parsed.core.channels, inputs[tree]);
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(tree), &mut pcm).unwrap();
        assert!(pcm.iter().any(|&x| x != 0));
    }
}

#[test]
fn actual_eld_core_spatial_seek_replays_pcm_and_timestamps() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-mps-core");
    for tree in 0..8 {
        let asc = std::fs::read(root.join(format!("tree{tree}.spatial.asc"))).unwrap();
        let mut decoder = fvid_media::owned_aac::NativeAacDecoder::new(&asc).unwrap();
        let packets: Vec<_> = (0..3)
            .map(|i| std::fs::read(root.join(format!("tree{tree}-{i}.au"))).unwrap())
            .collect();
        decoder.decode_timed(&packets[0], 0, 512).unwrap().unwrap();
        let saved = decoder.checkpoint();
        let mut expected = Vec::new();
        for (i, packet) in packets.iter().enumerate().skip(1) {
            expected.push(
                decoder
                    .decode_timed(packet, i as i64 * 512, 512)
                    .unwrap()
                    .unwrap(),
            );
        }
        decoder.restore(&saved).unwrap();
        for (i, (packet, want)) in packets.iter().skip(1).zip(expected).enumerate() {
            let got = decoder
                .decode_timed(packet, (i as i64 + 1) * 512, 512)
                .unwrap()
                .unwrap();
            assert_eq!(got.samples, want.samples);
            assert_eq!((got.pts, got.duration), (want.pts, want.duration));
        }
    }
}
