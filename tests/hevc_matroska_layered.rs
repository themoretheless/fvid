use fvid::playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader};
use fvid::playback_native::RawFrame;
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
#[test]
fn layered_matroska_matches_native_mp4_pixels_timing_seek_and_rewind() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-matroska-layered/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["cases"].as_array().unwrap().len(), 80);
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let mut control = Mp4VideoReader::open_software(
            Cursor::new(std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap()),
            Default::default(),
            BUDGET,
        )
        .unwrap();
        let mut expected = Vec::new();
        while let Some(f) = control.read_frame().unwrap() {
            let p = f.packed.unwrap();
            let rgb = RawFrame::Planar(p.clone()).into_rgb(BUDGET).unwrap();
            expected.push((
                p.frame.data.clone(),
                f.presentation_time.nanoseconds().unwrap(),
                f.duration.nanoseconds().unwrap(),
                rgb,
                (p.frame.width, p.frame.height, p.depth),
                control.bitstream_colour(),
                control.bitstream_hdr(),
                f.sample_index,
            ));
        }
        assert!(!expected.is_empty(), "{name}");
        let pts: Vec<u128> = c["pts_ns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap() as u128)
            .collect();
        let origin = pts[expected[0].7];
        let mut actual = WebmVideoReader::open_software(
            Cursor::new(std::fs::read(root().join("hevc-matroska-layered").join(name)).unwrap()),
            BUDGET,
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!actual.hardware_accelerated());
        let mut default = WebmVideoReader::open(
            Cursor::new(std::fs::read(root().join("hevc-matroska-layered").join(name)).unwrap()),
            BUDGET,
        )
        .unwrap();
        assert!(
            !default.hardware_accelerated(),
            "{name} must use owned layered reconstruction"
        );
        let Some(RawFrame::Planar(first)) = default.read_frame_raw().unwrap() else {
            panic!("{name}: default planar expected")
        };
        assert_eq!(first.frame.data, expected[0].0, "{name} default entry");
        for _ in 0..2 {
            let mut n = 0;
            while let Some(RawFrame::Planar(p)) = actual
                .read_frame_raw()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                assert_eq!(p.frame.data, expected[n].0, "{name} frame{n}");
                assert_eq!(
                    (p.frame.width, p.frame.height, p.depth),
                    expected[n].4,
                    "{name} geometry"
                );
                assert_eq!(
                    actual.frame_interval().unwrap().0,
                    pts[expected[n].7] - origin,
                    "{name}"
                );
                if c["irregular"] == true {
                    let explicit = pts[expected[n].7]
                        + c["durations_ns"][expected[n].7].as_u64().unwrap() as u128;
                    let end = expected
                        .get(n + 1)
                        .map_or(explicit, |f| explicit.min(pts[f.7]));
                    assert_eq!(
                        actual.frame_interval().unwrap().1,
                        end - origin,
                        "{name} endpoint"
                    );
                }
                assert_eq!(actual.bitstream_colour(), expected[n].5, "{name} colour");
                assert_eq!(actual.bitstream_hdr(), expected[n].6, "{name} HDR");
                n += 1;
            }
            assert_eq!(n, expected.len(), "{name}");
            actual.rewind();
        }
        for (i, e) in expected.iter().enumerate() {
            assert!(actual.read_frame().unwrap(), "{name} RGB{i}");
            assert_eq!(actual.rgb(), e.3, "{name} RGB{i}");
        }
        assert!(!actual.read_frame().unwrap());
        if expected.len() > 1 {
            let target = (pts[expected[1].7] - origin) as i64 + 1;
            let frame = actual.seek_to_frame_raw(target).unwrap().unwrap();
            let RawFrame::Planar(p) = frame else {
                panic!("{name}: planar expected")
            };
            assert_eq!(p.frame.data, expected[1].0, "{name} seek");
        }
    }
}

#[test]
fn layered_matroska_refuses_contradictory_outputs_and_requires_rewind() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("hevc-matroska-layered/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(m["refusals"].as_array().unwrap().len(), 4);
    for c in m["refusals"].as_array().unwrap() {
        let mut source = WebmVideoReader::open_software(
            Cursor::new(
                std::fs::read(
                    root()
                        .join("hevc-matroska-layered")
                        .join(c["file"].as_str().unwrap()),
                )
                .unwrap(),
            ),
            BUDGET,
        )
        .unwrap();
        for _ in 0..2 {
            let error = loop {
                match source.read_frame_raw() {
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("missing refusal"),
                    Err(e) => break e,
                }
            };
            assert!(
                error.to_string().contains(c["error"].as_str().unwrap()),
                "{error}"
            );
            assert!(
                source
                    .read_frame_raw()
                    .err()
                    .unwrap()
                    .to_string()
                    .contains("requires rewind")
            );
            source.rewind();
        }
    }
}
