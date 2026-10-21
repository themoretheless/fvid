use fvid::{
    container::webm::{Limits, WebmReader},
    playback::video::webm::WebmVideoReader,
    playback_native::RawFrame,
};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/matroska-parameter-seek")
}
fn cases() -> Vec<serde_json::Value> {
    let cases: Vec<_> =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    assert_eq!(cases.len(), 36);
    cases
}
fn controls(mode: &str) -> Vec<serde_json::Value> {
    let all: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root().join("controls.json")).unwrap()).unwrap();
    assert_eq!(all.len(), 4);
    let selected: Vec<_> = all.into_iter().filter(|c| c["mode"] == mode).collect();
    assert_eq!(selected.len(), 2);
    assert!(selected.iter().any(|c| c["codec"] == "avc"));
    assert!(selected.iter().any(|c| c["codec"] == "hevc"));
    selected
}

#[test]
fn fixture_index_keeps_real_keyframes_parameter_only_blocks_and_companions() {
    for case in cases() {
        let mut source = WebmReader::open(
            Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
            Limits::default(),
        )
        .unwrap();
        source.scan_until(i64::MAX).unwrap();
        let track = source.tracks.iter().find(|t| t.number == 1).unwrap();
        assert_eq!(
            track.codec,
            if case["codec"] == "avc" {
                "V_MPEG4/ISO/AVC"
            } else {
                "V_MPEGH/ISO/HEVC"
            }
        );
        let video: Vec<_> = source.packets.iter().filter(|p| p.track == 1).collect();
        let metadata =
            case["target_ns"].as_u64().unwrap() / 40_000_000 - case["new_start"].as_u64().unwrap();
        assert_eq!(
            video.len(),
            case["frames"].as_array().unwrap().len() + metadata as usize
        );
        let keys: Vec<_> = video
            .iter()
            .filter(|p| p.keyframe)
            .map(|p| p.pts_ns)
            .collect();
        assert_eq!(keys, [0, case["target_ns"].as_i64().unwrap()]);
        let others = source.packets.iter().filter(|p| p.track == 2).count();
        assert_eq!(
            others,
            if case["companion"].as_bool().unwrap() {
                video.len()
            } else {
                0
            }
        );
        assert_eq!(source.tracks.len(), if others == 0 { 1 } else { 2 });
    }
}

fn check_frame(
    reader: &mut WebmVideoReader<Cursor<Vec<u8>>>,
    case: &serde_json::Value,
    index: usize,
) {
    let file = case["file"].as_str().unwrap();
    let RawFrame::Planar(p) = reader
        .read_frame_raw()
        .unwrap_or_else(|e| panic!("{file}/{index}: {e}"))
        .unwrap()
    else {
        panic!("packed source precision expected")
    };
    let expected = &case["frames"][index];
    assert_eq!(
        p.frame.data,
        std::fs::read(root().join(expected["oracle"].as_str().unwrap())).unwrap(),
        "{file}/{index}"
    );
    assert_eq!(p.depth, expected["depth"].as_u64().unwrap() as u8);
    assert_eq!(
        [p.frame.width, p.frame.height],
        [
            case["size"][0].as_u64().unwrap() as usize,
            case["size"][1].as_u64().unwrap() as usize
        ]
    );
    assert_eq!(
        reader.frame_interval(),
        Some((
            expected["start_ns"].as_u64().unwrap() as u128,
            expected["end_ns"].as_u64().unwrap() as u128,
            1_000_000_000
        )),
        "{file}/{index}"
    );
}
fn open(case: &serde_json::Value) -> WebmVideoReader<Cursor<Vec<u8>>> {
    WebmVideoReader::open_software(
        Cursor::new(std::fs::read(root().join(case["file"].as_str().unwrap())).unwrap()),
        64 << 20,
    )
    .unwrap()
}
#[test]
fn sequential_and_rewind_controls_match_every_owned_component_and_interval() {
    for case in cases() {
        let mut reader = open(&case);
        for _ in 0..2 {
            for i in 0..case["frames"].as_array().unwrap().len() {
                check_frame(&mut reader, &case, i);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
    }
}
fn seek_family(codec: &str) {
    for case in cases().into_iter().filter(|case| case["codec"] == codec) {
        let mut reader = open(&case);
        for pass in 0..3 {
            if pass == 1 {
                reader.rewind();
                check_frame(&mut reader, &case, 0);
            }
            if pass == 2 {
                reader.rewind();
            }
            let target = case["target_ns"].as_i64().unwrap();
            assert_eq!(reader.seek_to_sync(target).unwrap(), target);
            for i in case["new_start"].as_u64().unwrap() as usize
                ..case["frames"].as_array().unwrap().len()
            {
                check_frame(&mut reader, &case, i);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
        }
    }
}

#[test]
fn fresh_and_used_avc_seek_restores_received_parameters() {
    seek_family("avc");
}
#[test]
fn fresh_and_used_hevc_seek_restores_received_parameters() {
    seek_family("hevc");
}

#[test]
fn seek_replays_parameters_but_does_not_decode_old_picture_entropy() {
    for case in &controls("skip-pictures") {
        let mut reader = open(case);
        // These damaged old pictures are a refusal control, not full playback
        // acceptance. Only random access to the independently coded new CVS
        // can succeed, while still requiring its received parameter prefix.
        assert!(reader.read_frame_raw().is_err());
        let target = case["target_ns"].as_i64().unwrap();
        assert_eq!(reader.seek_to_sync(target).unwrap(), target);
        for i in
            case["new_start"].as_u64().unwrap() as usize..case["frames"].as_array().unwrap().len()
        {
            check_frame(&mut reader, case, i);
        }
        assert!(reader.read_frame_raw().unwrap().is_none());
    }
}

#[test]
fn malformed_received_parameters_fail_seek_and_require_rewind() {
    for case in &controls("refuse-parameters") {
        let mut reader = open(case);
        assert!(
            reader
                .seek_to_sync(case["target_ns"].as_i64().unwrap())
                .is_err()
        );
        assert!(
            reader
                .read_frame_raw()
                .err()
                .unwrap()
                .to_string()
                .contains("requires rewind")
        );
        reader.rewind();
        check_frame(&mut reader, case, 0);
        // This stream really is malformed in sequential reception too.
        let mut failed = false;
        loop {
            match reader.read_frame_raw() {
                Err(_) => {
                    failed = true;
                    break;
                }
                Ok(None) => break,
                Ok(Some(_)) => {}
            }
        }
        assert!(failed);
    }
}
