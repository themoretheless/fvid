use fvid::playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader};
use fvid::playback_native::RawFrame;
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-topology-transitions")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
#[test]
fn native_frontends_preserve_base_and_enhancement_epochs() {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();

    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
    {
        let name = c["name"].as_str().unwrap();
        let oracle =
            std::fs::read(root().parent().unwrap().join(c["oracle"].as_str().unwrap())).unwrap();
        let samples = c["samples"].as_array().unwrap();
        let layers = c["layers"].as_array().unwrap();
        let mut mp4 = Mp4VideoReader::open_software(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        let mut mkv =
            WebmVideoReader::open_software(Cursor::new(read(&format!("{name}.mkv"))), BUDGET)
                .unwrap();
        for _ in 0..2 {
            for (i, sample) in samples.iter().enumerate() {
                let f = mp4
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} MP4 frame{i}: {e}"))
                    .unwrap();
                assert_eq!(
                    mp4.selected_layer_id(),
                    Some(layers[i].as_u64().unwrap() as u8)
                );
                assert_eq!(f.sample_index, sample.as_u64().unwrap() as usize);
                let Some(RawFrame::Planar(p)) = mkv
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{name} Matroska frame{i}: {e}"))
                else {
                    panic!("missing frame")
                };
                assert_eq!(
                    mkv.selected_layer_id(),
                    Some(layers[i].as_u64().unwrap() as u8)
                );
                assert_eq!(
                    &p.frame.data,
                    &oracle[..p.frame.data.len()],
                    "{name} scalar oracle"
                );
                assert_eq!(p.frame.data, f.packed.unwrap().frame.data);
                assert_eq!(
                    mkv.frame_interval().unwrap().0,
                    sample.as_u64().unwrap() as u128 * 40000000
                );
            }
            assert!(mp4.read_frame().unwrap().is_none());
            assert!(mkv.read_frame_raw().unwrap().is_none());
            mp4.rewind();
            mkv.rewind();
        }
        for at in [samples.len() - 1, 0, 2] {
            let sample = samples[at].as_u64().unwrap();
            let layer = layers[at].as_u64().unwrap() as u8;
            let Some(RawFrame::Planar(p)) = mkv
                .seek_to_frame_raw((sample * 40000000 + 1) as i64)
                .unwrap()
            else {
                panic!("{name} seek picture")
            };
            assert_eq!(&p.frame.data, &oracle[..p.frame.data.len()]);
            assert_eq!(mkv.selected_layer_id(), Some(layer));
            mp4.seek_to_sync(sample as i64);
            let f = mp4.read_frame().unwrap().unwrap();
            let p = f.packed.unwrap();
            assert_eq!(&p.frame.data, &oracle[..p.frame.data.len()]);
            assert_eq!(mp4.selected_layer_id(), Some(layer));
        }
    }
}

#[test]
fn base_prefix_reaches_later_multilayer_pictures() {
    let mut reader = Mp4VideoReader::open_software(
        Cursor::new(read("base-to-multi.mp4")),
        Default::default(),
        BUDGET,
    )
    .unwrap();
    let mut count = 0;
    while reader.read_frame().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, 4);
}

#[test]
fn topology_change_without_sequence_boundary_still_refuses_and_requires_rewind() {
    for name in ["illegal-base-to-multi", "illegal-multi-to-base"] {
        let mut mp4 = Mp4VideoReader::open_software(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        let mut mkv =
            WebmVideoReader::open_software(Cursor::new(read(&format!("{name}.mkv"))), BUDGET)
                .unwrap();
        for _ in 0..2 {
            let error = loop {
                match mp4.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{name} missing refusal"),
                    Err(e) => break e,
                }
            };
            assert!(error.to_string().contains("active VPS changed"), "{error}");
            assert!(
                mp4.read_frame()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            mp4.rewind();
            let error = loop {
                match mkv.read_frame_raw() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{name} missing refusal"),
                    Err(e) => break e,
                }
            };
            assert!(error.to_string().contains("active VPS changed"), "{error}");
            assert!(
                mkv.read_frame_raw()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            mkv.rewind();
        }
    }
}

#[test]
fn automatic_entries_keep_the_owned_layered_owner_for_the_entire_source() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for case in manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
    {
        let name = case["name"].as_str().unwrap();
        let oracle = std::fs::read(
            root()
                .parent()
                .unwrap()
                .join(case["oracle"].as_str().unwrap()),
        )
        .unwrap();
        let mut mp4 = Mp4VideoReader::open(
            Cursor::new(read(&format!("{name}.mp4"))),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        let mut mkv =
            WebmVideoReader::open(Cursor::new(read(&format!("{name}.mkv"))), BUDGET).unwrap();
        assert!(
            !mp4.hardware_accelerated(),
            "{name} MP4 topology needs the owned decoder"
        );
        assert!(
            !mkv.hardware_accelerated(),
            "{name} Matroska topology needs the owned decoder"
        );
        assert!(!mp4.enable_shared_surfaces().unwrap());
        for layer in case["layers"].as_array().unwrap() {
            let layer = layer.as_u64().unwrap() as u8;
            let frame = mp4.read_frame().unwrap().unwrap().packed.unwrap();
            assert_eq!(mp4.selected_layer_id(), Some(layer));
            assert_eq!(&frame.frame.data, &oracle[..frame.frame.data.len()]);
            let Some(RawFrame::Planar(frame)) = mkv.read_frame_raw().unwrap() else {
                panic!("{name}: owned planar frame required")
            };
            assert_eq!(mkv.selected_layer_id(), Some(layer));
            assert_eq!(&frame.frame.data, &oracle[..frame.frame.data.len()]);
        }
        assert!(mp4.read_frame().unwrap().is_none());
        assert!(mkv.read_frame_raw().unwrap().is_none());
    }
}
