use fvid::codec::av1::Obus;
use fvid::codec::av1_frame::Header;
use fvid::codec::av1_sequence::Sequence;
use fvid_media::owned_ffv1_decoder::Decoder as Ffv1Decoder;
use std::path::PathBuf;

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("video-decoders")
}

#[test]
fn ffv1_v3_nonkeyframe_quantizer_set_growth_initializes_range_contexts() {
    let dir = fixture_dir();
    let cfg = std::fs::read(dir.join("ffv1_v3_two_quant_sets.cfg")).expect("ffv1_v3_two_quant_sets");
    let kf = std::fs::read(dir.join("ffv1_v3_keyframe_set0.pkt")).expect("ffv1_v3_keyframe_set0");
    let inter =
        std::fs::read(dir.join("ffv1_v3_inter_grow_set1.pkt")).expect("ffv1_v3_inter_grow_set1");

    let mut decoder =
        Ffv1Decoder::from_configuration(2, 2, &cfg, 1 << 20).expect("valid FFV1 v3 configuration");
    let first = decoder.decode(&kf).expect("decodes keyframe with set 0");
    assert!(first.keyframe);
    assert_eq!(&first.frame.data[..4], &[0, 0, 0, 10]);

    let second = decoder
        .decode(&inter)
        .expect("decodes non-keyframe switching to larger quantizer set 1");
    assert!(!second.keyframe);
    assert_eq!(
        &second.frame.data[..4],
        &[0, 0, 0, 42],
        "grown FFV1 v3 range context table must initialize from set.initial rather than all-zero probabilities"
    );
}

#[test]
fn av1_film_grain_420_rejects_symmetric_chroma_point_count_disagreement() {
    let dir = fixture_dir();
    let grain_bytes =
        std::fs::read(dir.join("av1_grain_420_cb1_cr0.bin")).expect("av1_grain_420_cb1_cr0");

    let seq_obus = Obus::new(include_bytes!("fixtures/av1/sequence.obu"));
    let mut seq = Sequence::parse(
        seq_obus
            .map(Result::unwrap)
            .find(|o| o.kind == 1)
            .expect("sequence header OBU")
            .payload,
    )
    .expect("valid sequence header");
    seq.film_grain = true;
    assert_eq!(seq.color.subsampling, [true, true]);

    let err = match Header::parse_intra(&seq, &grain_bytes, 0, 0) {
        Ok(_) => panic!(
            "AV1 4:2:0 film grain with num_cb_points > 0 and num_cr_points == 0 must be rejected"
        ),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("chroma point counts disagree"),
        "unexpected error: {err}"
    );
}
