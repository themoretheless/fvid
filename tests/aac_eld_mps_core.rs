//! Real core + spatial suffix movies. Refusal is not playback qualification.
fn movie(tree: usize) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
            "tests/fixtures/playback-errors/aac-eld-mps-core/tree{tree}.mp4"
        )),
    )
    .unwrap()
}
#[test]
fn actual_eld_core_spatial_movies_reproduce_native_factory_refusal() {
    for tree in 0..8 {
        let mut pcm = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(&movie(tree), &mut pcm)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("LD-MPS spatial reconstruction"),
            "tree {tree}: {error}"
        );
        assert!(pcm.is_empty());
    }
}
#[test]
#[ignore = "pending native LD-MPS session, timestamp queue and retained-memory integration"]
fn actual_eld_core_spatial_movies_play_all_eight_trees() {
    for tree in 0..8 {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&movie(tree), &mut pcm).unwrap();
        assert!(!pcm.is_empty());
        assert!(pcm.iter().any(|&x| x != 0));
    }
}

#[test]
#[ignore = "pending native LD-MPS checkpoint and timed-output integration"]
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
