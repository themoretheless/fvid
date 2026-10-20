use fvid::{
    container::mp4::Limits,
    playback::video::{
        mp4::LayeredMp4VideoReader,
        native::{NativeReader, RawFrame},
    },
};
use std::{io::Cursor, time::Duration};
#[test]
fn cu_illumination_suppresses_vsp_then_vsp_resumes_and_rederives() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("illumination-vsp.json")).unwrap())
            .unwrap();
    let golden: Vec<u16> = std::fs::read(root.join("illumination-vsp.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let vectors: Vec<i64> = std::fs::read(root.join("illumination-vsp-motion.i64"))
        .unwrap()
        .chunks_exact(8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .collect();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 32);
    let mut changed = 0;
    for case in cases {
        let name = case["file"].as_str().unwrap();
        let size = case["size"].as_u64().unwrap() as usize;
        let at = case["oracle_offset"].as_u64().unwrap() as usize;
        let mt = case["motion_offset"].as_u64().unwrap() as usize;
        changed += case["changed_samples"].as_u64().unwrap();
        let expected = |poc| -> Vec<u16> {
            if poc == 1 {
                return golden[at..at + size * size * 3 / 2].to_vec();
            }
            (0..3)
                .flat_map(|c| {
                    let side = if c == 0 { size } else { size / 2 };
                    (0..side).flat_map(move |y| {
                        (0..side).map(move |x| {
                            ((if c == 0 {
                                3 * x + 5 * y + 17
                            } else {
                                13 * x + 7 * y + 29 + 83 * (c - 1) + [71, 113][c - 1]
                            }) % 256) as u16
                        })
                    })
                })
                .collect()
        };
        let data = std::fs::read(root.join(name)).unwrap();
        let mut reader = LayeredMp4VideoReader::open(
            Cursor::new(data.clone()),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            while let Some(frame) = reader
                .read_frame()
                .unwrap_or_else(|e| panic!("{name}: {e}"))
            {
                let id = frame.decoded.layer_id;
                let poc = frame.decoded.poc;
                seen.push((id, poc));
                if id != 5 {
                    continue;
                }
                let picture = &frame.decoded.picture;
                let wanted = expected(poc);
                let mut position = 0;
                for c in 0..3 {
                    let samples = picture.planes[c].samples();
                    for (i, (&a, &b)) in samples
                        .iter()
                        .zip(&wanted[position..position + samples.len()])
                        .enumerate()
                    {
                        assert_eq!(a, b, "{name}: POC{poc} c{c} sample{i}");
                    }
                    position += samples.len();
                }
                if poc == 1 {
                    for y in (0..size).step_by(8) {
                        for x in (0..size).step_by(8) {
                            let point = [x as u32, y as u32];
                            let motion = picture.prediction_motion_at(point).unwrap();
                            let offset = mt + 2 * (y * size + x);
                            assert_eq!(motion[0].unwrap().reference, 1, "{name}: reference{x},{y}");
                            assert_eq!(
                                motion[0].unwrap().mv,
                                [vectors[offset], vectors[offset + 1]],
                                "{name}: motion{x},{y}"
                            );
                            assert!(motion[1].is_none());
                            assert_eq!(
                                picture.prediction_is_skipped_at(point).unwrap(),
                                !(x >= 32 && x < 64 && y < 32),
                                "{name}: CU skip{x},{y}"
                            );
                            assert!(picture.inter_view_disparity_at(point).unwrap().is_none());
                        }
                    }
                }
            }
            assert_eq!(seen, [(0, 0), (3, 0), (5, 0), (0, 1), (3, 1), (5, 1)]);
            reader.rewind();
        }
        let mut native = NativeReader::software(Cursor::new(data), 64 << 20).unwrap();
        for target in [40, 0, 40, 40] {
            let RawFrame::Planar(frame) = native
                .seek_raw(Duration::from_millis(target))
                .unwrap()
                .unwrap()
            else {
                panic!("{name}: planar seek")
            };
            assert_eq!(
                frame.frame.data,
                expected(if target == 0 { 0 } else { 1 })
                    .iter()
                    .map(|&v| v as u8)
                    .collect::<Vec<_>>(),
                "{name}: seek{target}"
            );
        }
    }
    assert!(
        changed > 0,
        "IC CU must modify actual samples after VSP boundary reconstruction"
    );
}
