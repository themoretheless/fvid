use fvid::{
    container::mp4::Mp4Reader,
    playback::video::mp4::{Mp4VideoReader, VideoFrame},
};
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-unused-video-descriptions")
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
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
        .map(|&v| v as u8)
        .collect()
}
#[test]
fn unused_unknown_entries_do_not_hide_owned_video_decoders() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let original = bytes(c["source"].as_str().unwrap());
        let mut demux = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
        let mut control =
            Mp4Reader::open_indexed(Cursor::new(&original), Default::default()).unwrap();
        assert_eq!(
            demux.tracks().len(),
            1,
            "{name}: unused entry must not discard a supported track"
        );
        let insert = c["insert"].as_u64().unwrap() as usize;
        assert_eq!(
            demux.tracks()[0].sample_descriptions[insert].codec,
            *b"zzzz"
        );
        let count = control.tracks()[0].samples.len();
        assert_eq!(demux.tracks()[0].samples.len(), count);
        for i in 0..count {
            let mut a = Vec::new();
            let mut b = Vec::new();
            demux.read_packet(0, i, &mut a).unwrap();
            control.read_packet(0, i, &mut b).unwrap();
            assert_eq!(a, b, "{name} sample{i}");
            assert_eq!(
                demux.tracks()[0].description_for_sample(i),
                control.tracks()[0].description_for_sample(i)
            );
        }
        let mut actual =
            Mp4VideoReader::open_software(Cursor::new(&data), Default::default(), 64 << 20)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(&original), Default::default(), 64 << 20)
                .unwrap();
        for _ in 0..2 {
            while let Some(expected) = baseline.read_frame().unwrap() {
                let got = actual
                    .read_frame()
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                    .expect("missing supported frame");
                assert_eq!(got.sample_index, expected.sample_index);
                assert_eq!(got.presentation_time, expected.presentation_time);
                assert_eq!(got.duration, expected.duration);
                assert_eq!(pixels(&got), pixels(&expected), "{name}");
                assert_eq!(actual.selected_layer_id(), baseline.selected_layer_id());
                assert_eq!(
                    actual.container_description(),
                    baseline.container_description()
                );
                assert_eq!(actual.bitstream_hdr(), baseline.bitstream_hdr());
            }
            assert!(actual.read_frame().unwrap().is_none());
            baseline.rewind();
            actual.rewind();
        }
    }
}

#[test]
fn active_unknown_coding_and_external_references_still_refuse() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in m["refusals"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        if c["kind"] == "external" {
            let e = Mp4Reader::open_indexed(Cursor::new(&data), Default::default())
                .err()
                .expect("external sample reference admitted");
            assert_eq!(e.to_string(), "invalid sample data reference", "{name}");
            continue;
        }
        let demux = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
        assert_eq!(demux.tracks().len(), 1);
        let i = c["sample"].as_u64().unwrap() as usize;
        assert_eq!(
            demux.tracks()[0].description_for_sample(i).unwrap().codec,
            *b"zzzz"
        );
        let result =
            Mp4VideoReader::open_software(Cursor::new(&data), Default::default(), 64 << 20);
        if i == 0 {
            assert_eq!(
                result
                    .err()
                    .expect("unknown first coding admitted")
                    .to_string(),
                "MP4 has no supported AVC, HEVC, VP9 or AV1 video track"
            );
        } else {
            let mut r = result.unwrap();
            for _ in 0..2 {
                let e = loop {
                    match r.read_frame() {
                        Ok(Some(_)) => {}
                        Ok(None) => panic!("unknown late codec omitted"),
                        Err(e) => break e,
                    }
                };
                assert_eq!(
                    e.to_string(),
                    "unsupported MP4 video sample description",
                    "{name}"
                );
                assert!(
                    r.read_frame()
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("requires rewind")
                );
                r.rewind();
            }
        }
    }
}

#[test]
fn public_reader_reports_the_codec_of_the_presented_frame() {
    use fvid::playback_native::NativeReader;
    for suffix in ["", "-fragmented"] {
        let data = bytes(&format!("all-owned-codecs{suffix}-insert0.mp4"));
        let mut reader = NativeReader::software(Cursor::new(&data), 64 << 20).unwrap();
        for _ in 0..2 {
            assert_eq!(reader.video_codec(), "H.264");
            for expected in ["H.264", "H.265", "VP9", "H.265", "AV1"] {
                assert!(reader.read_frame_raw().unwrap().is_some());
                assert_eq!(reader.video_codec(), expected);
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind().unwrap();
        }
    }
}

#[test]
fn unused_unknown_descriptions_preserve_seek_resume() {
    let m: serde_json::Value = serde_json::from_slice(&bytes("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let data = bytes(name);
        let original = bytes(c["source"].as_str().unwrap());
        let demux = Mp4Reader::open_indexed(Cursor::new(&original), Default::default()).unwrap();
        let count = demux.tracks()[0].samples.len();
        let mut actual =
            Mp4VideoReader::open_software(Cursor::new(&data), Default::default(), 64 << 20)
                .unwrap();
        let mut baseline =
            Mp4VideoReader::open_software(Cursor::new(&original), Default::default(), 64 << 20)
                .unwrap();
        for index in [count - 1, 0, count / 2] {
            let pts = demux.tracks()[0].samples.get(index).unwrap().pts;
            assert_eq!(
                actual.seek_to_sync(pts),
                baseline.seek_to_sync(pts),
                "{name}"
            );
            assert_eq!(
                actual.container_description(),
                baseline.container_description()
            );
            while let Some(expected) = baseline.read_frame().unwrap() {
                let got = actual.read_frame().unwrap().expect("seek omitted frame");
                assert_eq!(got.sample_index, expected.sample_index);
                assert_eq!(got.presentation_time, expected.presentation_time);
                assert_eq!(got.duration, expected.duration);
                assert_eq!(pixels(&got), pixels(&expected), "{name}");
                assert_eq!(
                    actual.container_description(),
                    baseline.container_description()
                );
                assert_eq!(actual.selected_layer_id(), baseline.selected_layer_id());
                assert_eq!(actual.bitstream_hdr(), baseline.bitstream_hdr());
            }
            assert!(actual.read_frame().unwrap().is_none());
        }
    }
}
