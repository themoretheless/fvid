use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-b")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn pixels(picture: &IntraPicture) -> Vec<u8> {
    let mut bytes = Vec::new();
    picture.write_planar(&mut bytes).unwrap();
    bytes
}
#[test]
fn partitioned_b_prediction_and_residuals_match_scalar_pixels_and_combined_streams() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 22);
    for case in m["cases"].as_array().unwrap() {
        let mut actual = AvcDecoder::new(&read("configuration.avcc"), 2 << 20).unwrap();
        let mut control = AvcDecoder::new(&read("configuration.avcc"), 2 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for frame in case["frames"].as_array().unwrap() {
                let picture = actual
                    .decode_order(&read(frame["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} {}: {e}", case["name"], frame["packet"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(frame["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                let index = frame["presentation"].as_u64().unwrap() as usize;
                assert_eq!(
                    pixels(&picture),
                    pixels(&expected),
                    "{} combined frame{index}",
                    case["name"]
                );
                assert_eq!(
                    pixels(&picture),
                    oracle[index * 768..(index + 1) * 768],
                    "{} scalar frame{index}",
                    case["name"]
                );
            }
            actual.reset();
            control.reset();
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn partitioned_b_mp4_reorders_pictures_and_timestamps_through_rewind_and_seek() {
    use fvid::playback::video::mp4::Mp4VideoReader;
    for case in manifest()["cases"].as_array().unwrap() {
        let mut reader = Mp4VideoReader::open_software(
            std::io::Cursor::new(read(case["file"].as_str().unwrap())),
            Default::default(),
            8 << 20,
        )
        .unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for pass in 0..3 {
            for index in 0..5 {
                let frame = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"]))
                    .unwrap();
                assert_eq!(
                    pixels(&frame.picture),
                    oracle[index * 768..(index + 1) * 768],
                    "{} presentation{index}",
                    case["name"]
                );
                assert_eq!(
                    frame.presentation_time.nanoseconds().unwrap(),
                    index as i128 * 40_000_000
                );
                assert_eq!(frame.duration.nanoseconds().unwrap(), 40_000_000);
            }
            assert!(reader.read_frame().unwrap().is_none());
            if pass == 0 {
                reader.rewind();
            } else {
                assert_eq!(reader.seek_to_sync(4), 0);
            }
        }
    }
}

#[test]
fn fixtures_exercise_b_slice_headers_nonreference_partitions_and_decode_reordering() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    };
    let configuration = read("configuration.avcc");
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    assert_eq!(sps.profile, 88);
    assert_eq!(sps.max_num_ref_frames, 2);
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let records = case["frames"].as_array().unwrap();
        assert_eq!(
            records
                .iter()
                .map(|r| r["presentation"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            [0, 3, 1, 2, 4]
        );
        for record in &records[2..4] {
            let data = read(record["packet"].as_str().unwrap());
            let mut slices = 0;
            for nal in NalUnits::new(&data, 4).unwrap() {
                let nal = nal.unwrap();
                assert!((2..=4).contains(&(nal[0] & 31)));
                assert_eq!(nal[0] & 0x60, 0);
                if nal[0] & 31 == 2 {
                    let h = SliceHeader::parse(nal, &sps, &pps).unwrap();
                    assert_eq!(h.slice_type, SliceType::B);
                    assert_eq!(h.frame_num, 2);
                    assert_eq!(
                        h.poc_lsb,
                        Some(record["presentation"].as_u64().unwrap() as u32 * 2)
                    );
                    assert_eq!((h.refs_l0, h.refs_l1), (1, 1));
                    assert_eq!(
                        h.direct_spatial_mv_pred,
                        case["name"].as_str().unwrap().contains("spatial1")
                    );
                    slices += 1;
                    checked += 1;
                }
            }
            assert_eq!(
                slices,
                if case["name"].as_str().unwrap().contains("split1") {
                    2
                } else {
                    1
                }
            );
        }
    }
    assert_eq!(checked, 66);
    let l0 = read("l0-l1-spatial1-split0.yuv");
    let bi = read("bi-bi-spatial1-split0.yuv");
    // Chroma has no residual: independently distinguish the two predictors and
    // their rounded average at the same local sample in each macroblock.
    assert_eq!(l0[768 + 512], 42);
    assert_eq!(l0[768 + 512 + 8], 134);
    assert_eq!(bi[768 + 512], 44);
}
