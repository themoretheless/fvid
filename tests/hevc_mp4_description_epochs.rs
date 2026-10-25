use fvid::{
    container::mp4::Mp4Reader,
    playback::video::mp4::{LayeredMp4VideoReader, Mp4VideoReader},
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-mp4-description-epochs")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 14);
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
        .cloned()
        .collect()
}
fn oracle(c: &serde_json::Value, i: usize, bytes: &[u8]) {
    let o = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join(c["oracles"][i].as_str().unwrap()),
    )
    .unwrap();
    let at = c["oracle_frames"][i].as_u64().unwrap() as usize * bytes.len();
    assert_eq!(bytes, &o[at..at + bytes.len()], "{} frame{i}", c["name"]);
}
#[test]
fn descriptions_preserve_pixels_layer_selection_clock_and_presented_container_metadata() {
    for c in cases() {
        for suffix in ["", "-fragmented"] {
            let name = format!("{}{suffix}.mp4", c["name"].as_str().unwrap());
            let data = read(&name);
            let demux = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
            let mut reader = Mp4VideoReader::open(Cursor::new(&data), Default::default(), BUDGET)
                .unwrap_or_else(|e| panic!("{name} open: {e}"));
            assert!(!reader.hardware_accelerated());
            let visible: Vec<_> = c["layers"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .filter(|(_, l)| !l.is_null())
                .map(|(i, _)| i)
                .collect();
            for _ in 0..2 {
                for (at, &i) in visible.iter().enumerate() {
                    let f = reader
                        .read_frame()
                        .unwrap_or_else(|e| panic!("{name} frame{i}: {e}"))
                        .unwrap();
                    assert_eq!(f.sample_index, i);
                    assert_eq!(
                        f.presentation_time.nanoseconds().unwrap(),
                        i as i128 * 40_000_000
                    );
                    let ticks = visible.get(at + 1).map_or(1, |next| next - i);
                    assert_eq!(
                        f.duration.nanoseconds().unwrap(),
                        ticks as i128 * 40_000_000
                    );
                    assert_eq!(
                        reader.selected_layer_id(),
                        Some(c["layers"][i].as_u64().unwrap() as u8)
                    );
                    if !c["hdr"].is_null() {
                        assert_eq!(
                            reader.bitstream_hdr().light.max_cll,
                            c["hdr"][i].as_u64().unwrap() as f32,
                            "{name} HDR snapshot"
                        );
                    }
                    let description = demux.tracks()[0].description_for_sample(i).unwrap();
                    assert_eq!(
                        reader.container_description(),
                        description,
                        "{name} display metadata must not follow lookahead"
                    );
                    oracle(&c, i, &f.packed.unwrap().frame.data);
                }
                assert!(reader.read_frame().unwrap().is_none());
                reader.rewind();
            }
            for i in [*visible.last().unwrap(), 0, 1] {
                let landed = reader.seek_to_sync(i as i64);
                let f = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} seek{i}: {e}"))
                    .unwrap();
                assert_eq!(f.presentation_time.ticks, landed);
                let n = f.sample_index;
                assert!(n <= i);
                if !c["hdr"].is_null() {
                    assert_eq!(
                        reader.bitstream_hdr().light.max_cll,
                        c["hdr"][n].as_u64().unwrap() as f32,
                        "{name} seek HDR"
                    );
                }
                oracle(&c, n, &f.packed.unwrap().frame.data);
                assert_eq!(
                    reader.container_description(),
                    demux.tracks()[0].description_for_sample(n).unwrap()
                );
            }
        }
    }
}
#[test]
fn explicit_layered_reader_keeps_local_au_identity_and_all_output_layers() {
    for c in cases() {
        for suffix in ["", "-fragmented"] {
            let name = format!("{}{suffix}.mp4", c["name"].as_str().unwrap());
            // Explicit OLS1 also exercises base-only OLS0 only through the automatic
            // source; wholly multilayer cases keep this caller's fixed selection.
            if c["layers"].as_array().unwrap().iter().any(|l| l == 0) {
                continue;
            }
            let mut reader = LayeredMp4VideoReader::open(
                Cursor::new(read(&name)),
                Default::default(),
                1,
                0,
                BUDGET,
            )
            .unwrap();
            let mut ids = Vec::new();
            while let Some(f) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert_eq!(f.decoded.access_unit as usize, f.sample_index);
                let index = f.sample_index;
                ids.push((index, f.decoded.layer_id));
                oracle(
                    &c,
                    index,
                    &f.into_video_frame().unwrap().packed.unwrap().frame.data,
                );
            }
            assert_eq!(
                ids,
                c["layers"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| !l.is_null())
                    .flat_map(|(i, _)| [(i, 0), (i, 3)])
                    .collect::<Vec<_>>(),
                "{name}"
            );
        }
    }
}

#[test]
fn a_description_change_does_not_bypass_active_vps_guards() {
    for suffix in ["", "-fragmented"] {
        let name = format!("illegal-vps-inter{suffix}.mp4");
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(read(&name)), Default::default(), BUDGET)
                .unwrap();
        for _ in 0..2 {
            let error = loop {
                match reader.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{name}: missing refusal"),
                    Err(e) => break e,
                }
            };
            assert!(
                error.to_string().contains("active VPS changed"),
                "{name}: {error}"
            );
            assert!(
                reader
                    .read_frame()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            reader.rewind();
        }
    }
}

#[test]
fn layered_matroska_preserves_split_guidance_and_explicit_zero_across_seek() {
    use fvid::{playback::video::webm::WebmVideoReader, playback_native::RawFrame};
    for c in cases().into_iter().filter(|c| !c["hdr"].is_null()) {
        let name = format!("{}.mkv", c["name"].as_str().unwrap());
        let mut reader = WebmVideoReader::open_software(Cursor::new(read(&name)), BUDGET).unwrap();
        let visible: Vec<_> = c["layers"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .filter(|(_, l)| !l.is_null())
            .map(|(i, _)| i)
            .collect();
        for _ in 0..2 {
            for &i in &visible {
                let Some(RawFrame::Planar(f)) = reader
                    .read_frame_raw()
                    .unwrap_or_else(|e| panic!("{name} frame{i}: {e}"))
                else {
                    panic!("{name} missing frame{i}")
                };
                oracle(&c, i, &f.frame.data);
                assert_eq!(reader.selected_layer_id(), Some(3));
                assert_eq!(reader.frame_interval().unwrap().0, i as u128 * 40_000_000);
                assert_eq!(
                    reader.bitstream_hdr().light.max_cll,
                    c["hdr"][i].as_u64().unwrap() as f32,
                    "{name} HDR{i}"
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        for &i in visible.iter().rev() {
            let Some(RawFrame::Planar(f)) =
                reader.seek_to_frame_raw(i as i64 * 40_000_000 + 1).unwrap()
            else {
                panic!("{name} seek{i}")
            };
            oracle(&c, i, &f.frame.data);
            assert_eq!(reader.frame_interval().unwrap().0, i as u128 * 40_000_000);
            assert_eq!(
                reader.bitstream_hdr().light.max_cll,
                c["hdr"][i].as_u64().unwrap() as f32,
                "{name} seek HDR{i}"
            );
        }
    }
}

#[test]
fn opening_configuration_receipt_keeps_the_last_explicit_zero() {
    let c = cases()
        .into_iter()
        .find(|c| c["name"] == "hvcC-zero-hdr-inter")
        .unwrap();
    let mut reader = Mp4VideoReader::open_software(
        Cursor::new(read("initial-zero.mp4")),
        Default::default(),
        BUDGET,
    )
    .unwrap();
    for _ in 0..2 {
        for i in 0..3 {
            let f = reader.read_frame().unwrap().unwrap();
            assert_eq!(f.sample_index, i);
            assert_eq!(reader.bitstream_hdr().light.max_cll, 0.0);
            assert_eq!(reader.bitstream_hdr().light.max_fall, 0.0);
            oracle(&c, i, &f.packed.unwrap().frame.data);
        }
        assert!(reader.read_frame().unwrap().is_none());
        reader.rewind();
    }
}
