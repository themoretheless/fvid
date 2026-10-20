use fvid_media::owned_webm::{Limits, WebmReader};
use std::{fs, io::Cursor, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/matroska-clock")
}
fn reader(name: &str) -> WebmReader<Cursor<Vec<u8>>> {
    WebmReader::open(
        Cursor::new(fs::read(root().join(name)).unwrap()),
        Limits::default(),
    )
    .unwrap()
}

#[test]
fn recurring_identical_segment_clock_preserves_the_control_timeline_and_pixels() {
    let mut control = reader("control.mkv");
    let mut recurring = reader("same-segment-scale.mkv");
    control.scan_all().unwrap();
    recurring.scan_all().unwrap();
    assert_eq!(control.packets.len(), 3);
    assert_eq!(
        control.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
        [0, 10_000_000, 50_000_000]
    );
    for i in 0..3 {
        assert_eq!(control.packets[i].pts_ns, recurring.packets[i].pts_ns);
        assert_eq!(
            control.read_packet(i).unwrap(),
            recurring.read_packet(i).unwrap()
        );
        let out = fvid_codecs::codec::vp9_decoder::Decoder::new(32 << 20)
            .decode(&control.read_packet(i).unwrap())
            .unwrap();
        assert_eq!(out.picture.size, [[16, 16], [32, 16], [16, 32]][i]);
    }
}

#[test]
fn changing_segment_scale_cannot_retime_an_already_indexed_prefix() {
    let mut value = reader("changing-segment-scale.mkv");
    assert_eq!(value.packets.len(), 2);
    let prefix = value.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>();
    let error = value
        .scan_all()
        .expect_err("conflicting clock must not silently change timestamps");
    assert_eq!(error.to_string(), "changing WebM timestamp scale");
    assert_eq!(value.timestamp_scale_ns(), 1_000_000);
    assert_eq!(
        value.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
        prefix
    );
}

#[test]
fn track_scale_applies_to_relative_block_times_and_block_durations() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let mut value = reader(case["file"].as_str().unwrap());
        value.scan_all().unwrap();
        let times: Vec<i64> = serde_json::from_value(case["pts_ns"].clone()).unwrap();
        assert_eq!(
            value.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
            times,
            "{}",
            case["file"]
        );
        assert!(value
            .packets
            .iter()
            .all(|p| p.duration_ns == case["duration_ns"].as_u64()));
    }
}

#[test]
fn invalid_track_scales_fail_for_their_own_metadata_cause() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for file in manifest["invalid"].as_array().unwrap() {
        let data = fs::read(root().join(file.as_str().unwrap())).unwrap();
        let error = match WebmReader::open(Cursor::new(data), Limits::default()) {
            Ok(_) => panic!("invalid TrackTimestampScale was ignored: {file}"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), "invalid Matroska TrackTimestampScale");
    }
}

#[test]
fn exact_track_clock_matches_independent_rational_oracles() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for case in manifest["timestamp_oracles"].as_array().unwrap() {
        let expected = case["expected"].as_i64();
        let actual = fvid_media::owned_matroska_time::timestamp_ns(
            case["cluster"].as_u64().unwrap(),
            case["relative"].as_i64().unwrap() as i16,
            case["scale"].as_u64().unwrap(),
            f64::from_bits(case["factor_bits"].as_u64().unwrap()),
        );
        assert_eq!(actual, expected, "{case}");
    }
    for case in manifest["duration_oracles"].as_array().unwrap() {
        let actual = fvid_media::owned_matroska_time::duration_ns(
            case["ticks"].as_u64().unwrap(),
            case["scale"].as_u64().unwrap(),
            f64::from_bits(case["factor_bits"].as_u64().unwrap()),
        );
        assert_eq!(actual, case["expected"].as_u64(), "{case}");
    }
}

#[test]
fn track_scale_does_not_rescale_lace_default_duration_nanoseconds() {
    let mut value = reader("track-scale-laced.mkv");
    value.scan_all().unwrap();
    assert_eq!(
        value.packets.iter().map(|p| p.pts_ns).collect::<Vec<_>>(),
        [0, 10_000_000, 40_000_000]
    );
    assert!(value
        .packets
        .iter()
        .all(|p| p.duration_ns == Some(10_000_000)));
    for index in 0..3 {
        let decoded = fvid_codecs::codec::vp9_decoder::Decoder::new(32 << 20)
            .decode(&value.read_packet(index).unwrap())
            .unwrap();
        assert_eq!(decoded.picture.size, [[16, 16], [32, 16], [16, 32]][index]);
    }
}

#[test]
fn track_clock_storage_is_accounted_and_packet_limits_still_apply() {
    let mut control = reader("control.mkv");
    let mut scaled = reader("track-scale-double.mkv");
    control.scan_all().unwrap();
    scaled.scan_all().unwrap();
    assert!(
        scaled.estimated_index_payload_bytes().unwrap()
            >= control.estimated_index_payload_bytes().unwrap() + std::mem::size_of::<(u64, f64)>()
    );
    let data = fs::read(root().join("track-scale-double.mkv")).unwrap();
    let mut bounded = WebmReader::open(
        Cursor::new(data),
        Limits {
            packets: 2,
            ..Limits::default()
        },
    )
    .unwrap();
    assert_eq!(
        bounded.scan_all().unwrap_err().to_string(),
        "WebM packet count exceeds limit"
    );
}

#[test]
fn finite_track_scales_refuse_unrepresentable_packet_clocks() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for (file, cause) in manifest["overflow"].as_object().unwrap() {
        let data = fs::read(root().join(file)).unwrap();
        let error = match WebmReader::open(Cursor::new(data), Limits::default()) {
            Ok(_) => panic!("overflowing clock accepted: {file}"),
            Err(error) => error,
        };
        assert_eq!(error.to_string(), cause.as_str().unwrap());
    }
}
