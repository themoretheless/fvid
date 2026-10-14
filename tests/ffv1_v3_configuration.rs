//! Independent configuration/framing records and a v3 pixel acceptance reproducer.
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("ffv1-v3-configuration.json")).unwrap())
        .unwrap()
}
#[test]
fn v3_configuration_accepts_all_coders_depths_and_predicted_initial_states() {
    for case in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        let config =
            fvid_media::owned_ffv1_decoder::Configuration::parse(&bytes, 32 << 20).unwrap();
        assert_eq!(config.coder_type(), case["coder"].as_u64().unwrap() as u8);
        assert_eq!(config.depth(), case["depth"].as_u64().unwrap() as u8);
        assert_eq!(
            config.micro_version(),
            case["micro"].as_u64().unwrap() as u32
        );
        assert_eq!(config.rgb(), case["rgb"].as_bool().unwrap());
        assert_eq!(config.alpha(), case["alpha"].as_bool().unwrap());
        assert!(config.chroma() && config.slice_crc() && !config.intra());
        assert_eq!(config.chroma_shifts(), [0, 0]);
        assert_eq!(config.slice_grid(), [3, 2]);
        assert_eq!(config.quantization_sets().len(), 2);
        for (i, set) in config.quantization_sets().iter().enumerate() {
            let expected: Vec<[u8; 32]> =
                serde_json::from_value(case["initial"][i].clone()).unwrap();
            assert_eq!(set.initial_states(), expected);
            assert_eq!(set.context_count(), expected.len());
            let table = &set.quantizers()[0];
            assert_eq!(table[0], 0);
            assert_eq!(
                (table[1], table[128], table[255]),
                if i == 0 { (1, -1, -1) } else { (0, 0, 0) }
            );
        }
        let expected: Vec<u8> = serde_json::from_value(case["transitions"].clone()).unwrap();
        assert_eq!(config.transitions().as_slice(), expected);
        let native = fvid::codec::ffv1_decoder::Configuration::parse(&bytes, 32 << 20).unwrap();
        assert_eq!(native.memory_bytes(), config.memory_bytes());
        let exact =
            fvid_media::owned_ffv1_decoder::Configuration::parse(&bytes, config.memory_bytes())
                .unwrap();
        assert_eq!(exact.depth(), config.depth());
        assert_eq!(
            fvid_media::owned_ffv1_decoder::Configuration::parse(&bytes, config.memory_bytes() - 1)
                .err()
                .unwrap(),
            "FFV1 configuration exceeds memory budget"
        );
    }
}
#[test]
fn invalid_configuration_crc_parameters_and_states_fail_before_admission() {
    for case in manifest()["invalid"].as_array().unwrap() {
        let data = std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap();
        assert_eq!(
            fvid_media::owned_ffv1_decoder::Configuration::parse(&data, 32 << 20)
                .err()
                .unwrap(),
            case["error"].as_str().unwrap()
        );
    }
}
#[test]
fn a_coded_zero_probability_initial_state_is_admitted() {
    let data =
        std::fs::read(root().join(manifest()["zero_probability"].as_str().unwrap())).unwrap();
    let config = fvid_media::owned_ffv1_decoder::Configuration::parse(&data, 32 << 20).unwrap();
    assert_eq!(config.quantization_sets()[0].initial_states()[0][0], 0);
}
#[test]
fn v3_slice_footer_index_preserves_boundaries_and_checks_every_crc() {
    for case in manifest()["slices"].as_array().unwrap() {
        let config = fvid_media::owned_ffv1_decoder::Configuration::parse(
            &std::fs::read(root().join(case["config"].as_str().unwrap())).unwrap(),
            32 << 20,
        )
        .unwrap();
        let packet = std::fs::read(root().join(case["packet"].as_str().unwrap())).unwrap();
        let slices = config.slices(&packet, 32 << 20).unwrap();
        assert_eq!(slices.len(), 6);
        for (i, slice) in slices.iter().enumerate() {
            assert_eq!(slice.payload, [i as u8, 19 + i as u8, 200 - i as u8]);
            assert_eq!(
                slice.offset,
                i * (3 + if config.slice_crc() { 8 } else { 3 })
            );
        }
        let exact = config.memory_bytes()
            + 6 * std::mem::size_of::<fvid_media::owned_ffv1_decoder::Slice<'_>>();
        assert_eq!(config.slices(&packet, exact).unwrap().len(), 6);
        assert_eq!(
            config.slices(&packet, exact - 1).err().unwrap(),
            "FFV1 slice index exceeds memory budget"
        );
        for invalid in case["invalid"].as_array().unwrap() {
            let data = std::fs::read(root().join(invalid["file"].as_str().unwrap())).unwrap();
            assert_eq!(
                config.slices(&data, 32 << 20).err().unwrap(),
                invalid["error"].as_str().unwrap()
            );
        }
    }
}
#[test]
fn v3_zero_frame_reproducer_now_plays_with_exact_samples() {
    let data = std::fs::read(root().join(manifest()["reproducer"].as_str().unwrap())).unwrap();
    let mut container =
        fvid_media::owned_webm::WebmReader::open(Cursor::new(data.clone()), Default::default())
            .unwrap();
    container.scan_all().unwrap();
    assert_eq!(container.tracks[0].codec, "V_FFV1");
    let config = fvid_media::owned_ffv1_decoder::Configuration::parse(
        &container.tracks[0].codec_private,
        32 << 20,
    )
    .unwrap();
    assert_eq!(
        (
            config.coder_type(),
            config.depth(),
            config.rgb(),
            config.alpha(),
            config.slice_grid()
        ),
        (1, 8, false, false, [1, 1])
    );
    let packet = container.read_packet(0).unwrap();
    assert_eq!(config.slices(&packet, 32 << 20).unwrap().len(), 1);
    let mut reader =
        fvid::playback_native::NativeReader::software(Cursor::new(data), 32 << 20).unwrap();
    for _ in 0..2 {
        let fvid::playback_native::RawFrame::Planar(p) = reader.read_frame_raw().unwrap().unwrap()
        else {
            panic!("v3 raw samples lost")
        };
        assert_eq!(
            (p.depth, p.frame.width, p.frame.height, p.frame.subsampling),
            (8, 4, 2, Some([1, 1]))
        );
        assert_eq!(
            p.frame.data,
            std::fs::read(root().join(manifest()["reference"].as_str().unwrap())).unwrap()
        );
        assert!(reader.read_frame_raw().unwrap().is_none());
        reader.rewind().unwrap();
    }
}
