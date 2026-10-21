use fvid::codec::{avc_decoder::AvcDecoder, avc_picture::IntraPicture};
use std::path::PathBuf;
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avc-data-partitions-switching")
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
fn partitioned_switching_matches_normative_matrix_pixels_and_combined_streams() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 14);
    for case in m["cases"].as_array().unwrap() {
        let mut actual = AvcDecoder::new(&read("configuration.avcc"), 2 << 20).unwrap();
        let mut control = AvcDecoder::new(&read("configuration.avcc"), 2 << 20).unwrap();
        let oracle = read(case["oracle"].as_str().unwrap());
        for _ in 0..2 {
            for d in [&mut actual, &mut control] {
                d.decode_order(&read("seed.packet")).unwrap().unwrap();
            }
            for (index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
                let picture = actual
                    .decode_order(&read(frame["packet"].as_str().unwrap()))
                    .unwrap_or_else(|e| panic!("{} {}: {e}", case["name"], frame["packet"]))
                    .unwrap();
                let expected = control
                    .decode_order(&read(frame["control"].as_str().unwrap()))
                    .unwrap()
                    .unwrap();
                let index = index + 1;
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
fn partitioned_switching_mp4_preserves_pixels_and_clocks_through_rewind_and_seek() {
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
            for index in 0..4 {
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
                assert_eq!(reader.seek_to_sync(3), 0);
            }
        }
    }
}

#[test]
fn switching_partition_headers_and_payload_categories_match_intended_tools() {
    use fvid::codec::{
        avc::{Pps, Sps},
        avc_access_unit::prepare,
        avc_slice::SliceType,
        config::{AvcConfig, NalUnits},
    };
    let configuration = read("configuration.avcc");
    let config = AvcConfig::parse(&configuration).unwrap();
    let sps = Sps::parse(config.sps[0]).unwrap();
    let pps = Pps::parse(config.pps[0], &sps).unwrap();
    assert_eq!(sps.profile, 88);
    let mut checked = 0;
    for case in manifest()["cases"].as_array().unwrap() {
        let sp = case["kind"].as_str().unwrap().starts_with("sp-");
        let secondary = case["kind"] == "sp-secondary";
        let split = case["name"].as_str().unwrap().ends_with("split1");
        for record in case["frames"].as_array().unwrap() {
            let packet = read(record["packet"].as_str().unwrap());
            let selected = prepare(&packet, 4, &sps, &pps, packet.len()).unwrap();
            assert_eq!(selected.len(), if split { 2 } else { 1 });
            for slice in selected {
                assert_eq!(
                    slice.header.slice_type,
                    if sp { SliceType::Sp } else { SliceType::Si }
                );
                assert_eq!(slice.header.slice_qp, 26);
                assert_eq!(
                    slice.header.slice_qs,
                    Some(record["qs"].as_i64().unwrap() as i32)
                );
                assert_eq!(slice.header.sp_for_switch, secondary);
                assert!(slice.header.data_partitions.is_some());
                checked += 1;
            }
            let mut counts = [0; 5];
            for nal in NalUnits::new(&packet, 4).unwrap() {
                counts[usize::from(nal.unwrap()[0] & 31)] += 1;
            }
            assert_eq!(
                counts[4],
                usize::from(sp && case["signed"].as_bool().unwrap())
            );
            assert_eq!(
                counts[3],
                if !sp && split && case["signed"].as_bool().unwrap() {
                    2
                } else {
                    1
                }
            );
            assert!(
                prepare(&packet, 4, &sps, &pps, packet.len() - 1)
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("byte limit")
            );
        }
    }
    assert_eq!(checked, 63);
    let ordinary = read("si-ordinary-signed0-split0.yuv");
    assert_eq!(ordinary[768..1536], ordinary[2304..3072]);
    let switching = read("si-switching-signed0-split0.yuv");
    assert_ne!(switching[768..1536], ordinary[768..1536]);
    let signed = read("sp-primary-signed1-split0.yuv");
    let skip = read("sp-primary-signed0-split0.yuv");
    assert_ne!(signed[768..1536], skip[768..1536]);
}
