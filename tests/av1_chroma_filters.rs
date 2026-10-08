//! Native reconstruction acceptance for owned 4:2:2/4:4:4 streams.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
#[test]
fn owned_odd_422_444_filter_pixels_reset_webm_and_seek() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("av1-chroma-filters-generated.json")).unwrap(),
    )
    .unwrap();
    let records = m["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), 18);
    let mut active = std::collections::BTreeMap::<(String, u8), [bool; 3]>::new();
    let mut applied =
        std::collections::BTreeMap::<(String, u8), ([u32; 3], [[u32; 3]; 3], [u32; 3])>::new();
    for r in records {
        let [width, height] = [191usize, 127usize];
        let name = r["file"].as_str().unwrap();
        let bytes = std::fs::read(root.join(name)).unwrap();
        let obus: Vec<_> = Obus::new(&bytes).map(Result::unwrap).collect();
        let s = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        let layout = r["layout"].as_str().unwrap();
        let sub = if layout == "422" {
            [true, false]
        } else {
            [false, false]
        };
        assert_eq!(s.color.subsampling, sub, "{name}: actual layout");
        assert_eq!(s.color.depth, r["depth"].as_u64().unwrap() as u8);
        let h = Header::parse_intra(&s, obus.iter().find(|o| o.kind == 6).unwrap().payload, 0, 0)
            .unwrap();
        assert_eq!(h.size, [width as u32, height as u32]);
        if r["forced_deblocking"].as_bool().unwrap() {
            assert_eq!(
                h.filter.levels,
                [16, 24, 20, 28],
                "{name}: actual forced levels"
            );
            assert_eq!(h.filter.sharpness, 2);
        }
        let coverage = active
            .entry((layout.to_string(), s.color.depth))
            .or_default();
        coverage[0] |= h.filter.levels.iter().any(|&v| v != 0);
        coverage[1] |= h
            .cdef
            .strengths
            .iter()
            .any(|v| v[2..].iter().any(|&x| x != 0));
        coverage[2] |= h.restoration_types != [0; 3];
        assert_eq!(
            h.lossless.iter().all(|v| *v),
            r["lossless"].as_bool().unwrap()
        );
        let cw = width.div_ceil(1 << usize::from(sub[0]));
        let ch = height.div_ceil(1 << usize::from(sub[1]));
        let word = if s.color.depth == 8 { 1 } else { 2 };
        assert_eq!(
            std::fs::read(root.join(r["reference"].as_str().unwrap()))
                .unwrap()
                .len(),
            (width * height + 2 * cw * ch) * word * 2
        );
        let mut decoder = Decoder::new(16 << 20);
        for replay in 0..2 {
            let frames = decoder
                .decode_packet(&bytes)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut actual = Vec::new();
            let mut shown = 0;
            for frame in frames {
                if !frame.show {
                    continue;
                }
                shown += 1;
                if replay == 0 {
                    let totals = applied
                        .entry((layout.to_string(), s.color.depth))
                        .or_default();
                    for p in 0..3 {
                        totals.0[p] += frame.picture.cdef_filtered_blocks[p];
                        totals.2[p] += frame.picture.deblocking_edges[p];
                        for kind in 0..3 {
                            totals.1[p][kind] += frame.picture.restoration_unit_counts[p][kind];
                        }
                    }
                }
                assert_eq!(frame.picture.subsampling, sub);
                for p in 0..3 {
                    let shifts = if p == 0 { [0; 2] } else { sub.map(usize::from) };
                    let w = width.div_ceil(1 << shifts[0]);
                    let h = height.div_ceil(1 << shifts[1]);
                    let plane = &frame.picture.planes[p];
                    for y in 0..h {
                        for v in &plane.samples[y * plane.width..y * plane.width + w] {
                            if word == 1 {
                                actual.push(*v as u8);
                            } else {
                                actual.extend_from_slice(&v.to_le_bytes());
                            }
                        }
                    }
                }
            }
            assert_eq!(shown, 2, "{name}");
            let expected = std::fs::read(root.join(r["reference"].as_str().unwrap())).unwrap();
            if let Some(i) = actual.iter().zip(&expected).position(|(a, b)| a != b) {
                panic!(
                    "{name}: pixel byte {i}: actual {}, expected {}",
                    actual[i], expected[i]
                );
            }
            assert_eq!(actual.len(), expected.len(), "{name}");
            decoder.finish().unwrap();
            decoder.reset();
        }
        let expected = std::fs::read(root.join(r["reference"].as_str().unwrap())).unwrap();
        let frame_bytes = expected.len() / 2;
        let container = std::fs::read(root.join(r["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(container), 16 << 20)
                .unwrap();
        for _ in 0..2 {
            for i in 0..2 {
                let frame = reader.read_frame_raw().unwrap().unwrap();
                let actual = match frame {
                    fvid::playback_native::RawFrame::Planar8(p) => {
                        assert_eq!([p.chroma_width, p.chroma_height], [cw, ch]);
                        p.y.iter()
                            .chain(&p.cb)
                            .chain(&p.cr)
                            .copied()
                            .collect::<Vec<_>>()
                    }
                    fvid::playback_native::RawFrame::Planar(p) => {
                        assert_eq!(p.frame.subsampling, Some(sub.map(|s| 1 << usize::from(s))));
                        p.frame.data.clone()
                    }
                    _ => panic!("{name}: unexpected raw frame"),
                };
                assert_eq!(
                    actual,
                    expected[i * frame_bytes..(i + 1) * frame_bytes],
                    "{name}: WebM {i}"
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        i as u128 * 20_000_000,
                        (i as u128 + 1) * 20_000_000,
                        1_000_000_000
                    ))
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(20_000_000).unwrap(), 0);
        for i in 0..2 {
            let frame = reader.read_frame_raw().unwrap().unwrap();
            let actual = match frame {
                fvid::playback_native::RawFrame::Planar8(p) => {
                    p.y.iter()
                        .chain(&p.cb)
                        .chain(&p.cr)
                        .copied()
                        .collect::<Vec<_>>()
                }
                fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
                _ => panic!("unexpected seek frame"),
            };
            assert_eq!(
                actual,
                expected[i * frame_bytes..(i + 1) * frame_bytes],
                "{name}: seek {i}"
            );
        }
    }
    assert_eq!(applied.len(), 6);
    for (key, (cdef, restoration, deblocking)) in &applied {
        eprintln!(
            "{key:?}: applied CDEF={cdef:?} restoration={restoration:?} deblocking={deblocking:?}"
        );
        assert!(
            deblocking.iter().all(|v| *v > 0),
            "{key:?}: missing applied deblocking"
        );
        assert!(
            cdef.iter().all(|v| *v > 0),
            "{key:?}: missing applied CDEF plane"
        );
        assert!(
            restoration.iter().all(|v| v[1] + v[2] > 0),
            "{key:?}: missing applied restoration"
        );
    }
    assert_eq!(active.len(), 6);
    for (key, coverage) in active {
        assert!(
            coverage[1..].iter().all(|v| *v),
            "{key:?}: missing filter header coverage {coverage:?}"
        );
    }
}
