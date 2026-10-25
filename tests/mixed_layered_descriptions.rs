use fvid::{
    container::mp4::Mp4Reader,
    playback::video::mp4::{Mp4VideoReader, VideoFrame},
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mixed-layered-descriptions")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 12);
    m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["error"].is_null())
        .cloned()
        .collect()
}
fn pixels(f: &VideoFrame) -> Vec<u8> {
    if let Some(p) = &f.packed {
        return p.frame.data.clone();
    }
    f.picture
        .y
        .iter()
        .chain(&f.picture.cb)
        .chain(&f.picture.cr)
        .map(|&n| u8::try_from(n).unwrap())
        .collect()
}
fn check(
    c: &serde_json::Value,
    f: &VideoFrame,
    reader: &Mp4VideoReader<Cursor<&Vec<u8>>>,
    demux: &Mp4Reader<Cursor<&Vec<u8>>>,
) {
    let i = f.sample_index;
    assert_eq!(f.presentation_time.ticks, c["pts"][i].as_i64().unwrap());
    let current = c["pts"][i].as_i64().unwrap();
    let next = c["visible"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| c["pts"][n.as_u64().unwrap() as usize].as_i64().unwrap())
        .filter(|&pts| pts > current)
        .min();
    assert_eq!(f.duration.ticks, next.map_or(1, |pts| pts - current));
    if !c["hdr"].is_null() {
        assert_eq!(
            reader.bitstream_hdr().light.max_cll,
            c["hdr"][i].as_u64().unwrap() as f32
        );
    }

    let layer = c["layers"][i].as_u64().map(|n| n as u8);
    assert_eq!(reader.selected_layer_id(), layer);
    assert_eq!(
        reader.container_description(),
        demux.tracks()[0].description_for_sample(i).unwrap()
    );
    let p = pixels(f);
    let o = read(c["oracles"][i].as_str().unwrap());
    let offset = c["oracle_frames"][i].as_u64().unwrap() as usize * p.len();
    assert_eq!(p, &o[offset..offset + p.len()], "{} sample{i}", c["name"]);
}
#[test]
fn mixed_decoders_preserve_pixels_pts_description_and_displayed_layer() {
    for c in cases() {
        for suffix in ["", "-fragmented"] {
            let name = format!("{}{suffix}.mp4", c["name"].as_str().unwrap());
            let data = read(&name);
            let demux = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
            let mut reader = Mp4VideoReader::open(Cursor::new(&data), Default::default(), BUDGET)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!reader.hardware_accelerated());
            let mut order: Vec<_> = c["visible"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as usize)
                .collect();
            order.sort_by_key(|&i| c["pts"][i].as_i64().unwrap());
            for _ in 0..2 {
                for &i in &order {
                    let f = reader
                        .read_frame()
                        .unwrap_or_else(|e| panic!("{name} sample{i}: {e}"))
                        .unwrap();
                    assert_eq!(f.sample_index, i, "{name} presentation order");
                    check(&c, &f, &reader, &demux);
                }
                assert!(reader.read_frame().unwrap().is_none());
                reader.rewind();
            }
            for target in [*order.last().unwrap(), order[0], order[order.len() / 2]] {
                let at = c["pts"][target].as_i64().unwrap();
                let landed = reader.seek_to_sync(at);
                let mut got = Vec::new();
                while let Some(f) = reader
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name} seek{at}: {e}"))
                {
                    check(&c, &f, &reader, &demux);
                    got.push(f.sample_index);
                }
                let start = order.iter().position(|i| Some(i) == got.first()).unwrap();
                let wanted = order
                    .iter()
                    .position(|&i| c["pts"][i].as_i64().unwrap() == at)
                    .unwrap();
                assert!(
                    start <= wanted,
                    "{name}: seek {at} omitted its target, landed {landed}"
                );
                assert_eq!(got, &order[start..], "{name}: seek suffix, landed {landed}");
            }
        }
    }
}

#[test]
fn late_hevc_entropy_failure_keeps_rewind_available() {
    for suffix in ["", "-fragmented"] {
        let data = read(&format!("corrupt-hevc-between-vp9{suffix}.mp4"));
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(&data), Default::default(), BUDGET).unwrap();
        for _ in 0..2 {
            let error = loop {
                match reader.read_frame() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("missing entropy refusal"),
                    Err(e) => break e,
                }
            };
            assert_eq!(error.to_string(), "truncated or oversized bit field");
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
fn opening_and_presented_hdr_do_not_follow_the_other_codec() {
    for suffix in ["", "-fragmented"] {
        let data = read(&format!("opening-hdr{suffix}.mp4"));
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(&data), Default::default(), BUDGET).unwrap();
        for _ in 0..2 {
            assert_eq!(reader.bitstream_hdr().light.max_cll, 100.0);
            for (i, cll) in [100.0, 0.0, 100.0].into_iter().enumerate() {
                assert_eq!(reader.read_frame().unwrap().unwrap().sample_index, i);
                assert_eq!(reader.bitstream_hdr().light.max_cll, cll);
            }
            assert!(reader.read_frame().unwrap().is_none());
            reader.rewind();
        }
    }
}
