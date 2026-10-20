use fvid::{
    container::webm::{Limits, WebmReader},
    playback_webm::WebmVideoReader,
};
use std::{fs, io::Cursor, path::PathBuf, time::Duration};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/matroska-tail")
}
fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}
fn open(case: &serde_json::Value, key: &str) -> WebmVideoReader<Cursor<Vec<u8>>> {
    WebmVideoReader::open_software(
        Cursor::new(fs::read(root().join(case[key].as_str().unwrap())).unwrap()),
        32 << 20,
    )
    .unwrap()
}

fn named(name: &str) -> serde_json::Value {
    cases()
        .into_iter()
        .find(|case| case["inferred"] == format!("{name}-inferred.mkv"))
        .unwrap()
}
fn check_duration(name: &str) {
    let case = named(name);
    let video = open(&case, "inferred");
    assert_eq!(
        video.duration(),
        Some(Duration::from_nanos(case["expected_ns"].as_u64().unwrap())),
        "{}",
        case["inferred"]
    );
}
fn check_cache(name: &str, key: &str) {
    let case = named(name);
    let mut video = open(&case, key);
    let before = video.cache_packets();
    assert_eq!(
        before.len(),
        case["intervals"].as_array().unwrap().len(),
        "{}",
        case[key]
    );
    while video.read_frame().unwrap() {}
    assert_eq!(before, video.cache_packets(), "{}", case[key]);
    for (packet, interval) in before.iter().zip(case["intervals"].as_array().unwrap()) {
        assert_eq!(
            packet.2.as_nanos(),
            u128::from(interval[0].as_u64().unwrap())
        );
        assert_eq!(
            packet.3.as_nanos(),
            u128::from(interval[1].as_u64().unwrap())
        );
    }
}
#[test]
fn overlapping_explicit_cache_intervals_match_presentation() {
    check_duration("overlapping-explicit");
    for key in ["inferred", "declared"] {
        check_cache("overlapping-explicit", key);
        let case = named("overlapping-explicit");
        let mut video = open(&case, key);
        let cache = video.cache_packets();
        // Alternate directions and land inside each displayed interval.
        for (target, expected) in [(41_000_000, (40_000_000, 50_000_000)),
                                   (1_000_000, (0, 20_000_000)),
                                   (22_000_000, (20_000_000, 25_000_000))] {
            video.seek_to_frame(target).unwrap();
            let interval = video.frame_interval().unwrap();
            assert_eq!((interval.0, interval.1), expected);
            assert_eq!(video.cache_packets(), cache);
        }
    }
}

#[test]
fn missing_duration_uses_explicit_tail() {
    check_duration("explicit-tail");
}
#[test]
fn missing_duration_uses_default_tail() {
    check_duration("default-tail");
}
#[test]
fn single_frame_uses_its_default_duration() {
    check_duration("single-default");
}
#[test]
fn inferred_cache_keeps_the_default_tail_before_playback() {
    check_cache("default-tail", "inferred");
}
#[test]
fn declared_cache_keeps_the_default_tail_before_playback() {
    check_cache("default-tail", "declared");
}
#[test]
fn inferred_cache_keeps_a_single_frame_before_playback() {
    check_cache("single-default", "inferred");
}
#[test]
fn declared_cache_keeps_a_single_frame_before_playback() {
    check_cache("single-default", "declared");
}
#[test]
fn explicit_cache_tail_stays_stable_during_playback() {
    for key in ["inferred", "declared"] {
        check_cache("explicit-tail", key);
    }
}

#[test]
fn previous_explicit_presentation_duration_defines_an_unknown_tail() {
    check_duration("previous-explicit");
}

#[test]
fn timestamp_gap_and_single_unknown_frame_fallbacks_stay_consistent() {
    for name in ["inferred-gap", "single-unknown"] {
        check_duration(name);
        for key in ["inferred", "declared"] {
            check_cache(name, key);
        }
    }
}

#[test]
fn previous_explicit_cache_tail_does_not_use_the_current_frame_period() {
    for key in ["inferred", "declared"] {
        check_cache("previous-explicit", key);
    }
}

#[test]
fn inferred_tail_and_cache_use_the_same_nonzero_track_origin() {
    check_duration("offset-gap");
    for key in ["inferred", "declared"] {
        check_cache("offset-gap", key);
    }
}

#[test]
fn paired_controls_decode_the_same_frames_with_exact_intervals_seek_and_eof() {
    for case in cases() {
        let read = |key: &str| {
            WebmReader::open(
                Cursor::new(fs::read(root().join(case[key].as_str().unwrap())).unwrap()),
                Limits::default(),
            )
            .unwrap()
        };
        let mut a = read("inferred");
        let mut b = read("declared");
        a.scan_all().unwrap();
        b.scan_all().unwrap();
        assert_eq!(a.packets.len(), b.packets.len());
        for i in 0..a.packets.len() {
            assert_eq!(a.read_packet(i).unwrap(), b.read_packet(i).unwrap());
        }
        for key in ["inferred", "declared"] {
            let mut video = open(&case, key);
            for _ in 0..2 {
                for (i, interval) in case["intervals"].as_array().unwrap().iter().enumerate() {
                    assert!(video.read_frame().unwrap());
                    let size: [usize; 2] =
                        serde_json::from_value(case["sizes"][i].clone()).unwrap();
                    assert_eq!(video.dimensions(), size);
                    let actual = video.frame_interval().unwrap();
                    assert_eq!(
                        (actual.0, actual.1),
                        (
                            u128::from(interval[0].as_u64().unwrap()),
                            u128::from(interval[1].as_u64().unwrap())
                        )
                    );
                    assert!(video.rgb().iter().all(|&sample| sample == 130));
                }
                assert!(!video.read_frame().unwrap());
                video.rewind();
            }
            let last = case["intervals"].as_array().unwrap().last().unwrap();
            video.seek_to_frame(last[0].as_i64().unwrap()).unwrap();
            assert_eq!(
                video.frame_interval().unwrap().1,
                u128::from(last[1].as_u64().unwrap()),
                "{} seek",
                case[key]
            );
            assert!(!video.read_frame().unwrap());
        }
    }
}
