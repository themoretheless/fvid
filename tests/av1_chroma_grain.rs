//! Full-chroma grain presets, odd geometry and ungrained inter-frame ownership.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_sequence::Sequence};
fn pixels(p: &fvid::codec::av1_picture::Picture) -> Vec<u8> {
    let mut out = Vec::new();
    for (i, plane) in p.planes.iter().enumerate() {
        let [sx, sy] = if i == 0 {
            [1; 2]
        } else {
            p.subsampling.map(|s| 1 << usize::from(s))
        };
        for y in 0..(p.size[1] as usize).div_ceil(sy) {
            for x in 0..(p.size[0] as usize).div_ceil(sx) {
                let v = plane.samples[y * plane.width + x];
                if p.depth == 8 {
                    out.push(v as u8);
                } else {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
        }
    }
    out
}
fn raw(frame: fvid::playback_native::RawFrame) -> Vec<u8> {
    match frame {
        fvid::playback_native::RawFrame::Planar8(p) => {
            p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect()
        }
        fvid::playback_native::RawFrame::Planar(p) => p.frame.data.clone(),
        _ => panic!("unexpected AV1 frame"),
    }
}
fn qualify(manifest_name: &str, expected_records: usize, expected_frames: usize) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join(manifest_name)).unwrap()).unwrap();
    let records = manifest["fixtures"].as_array().unwrap();
    assert_eq!(records.len(), expected_records);
    let mut groups = [[0usize; 2]; 3];
    let mut inherited_groups = [[0usize; 2]; 3];
    let mut lag_groups = [[[0usize; 4]; 2]; 3];
    let mut luma_groups = [[[0usize; 2]; 2]; 3];
    let mut overlap_groups = [[[0usize; 2]; 2]; 3];
    let mut range_groups = [[[0usize; 2]; 2]; 3];
    let mut inherited = 0;
    let mut overlaps = [0; 2];
    let mut ranges = [0; 2];
    let mut lags = [0; 4];
    let mut from_luma = [0; 2];
    for record in records {
        let name = record["file"].as_str().unwrap();
        let data = std::fs::read(root.join(name)).unwrap();
        let expected = std::fs::read(root.join(record["reference"].as_str().unwrap())).unwrap();
        let count = record["frames"].as_u64().unwrap() as usize;
        assert_eq!(count, expected_frames);
        let frame_bytes = expected.len() / count;
        let obus: Vec<_> = Obus::new(&data).map(Result::unwrap).collect();
        let sequence = Sequence::parse(obus.iter().find(|o| o.kind == 1).unwrap().payload).unwrap();
        let layout = record["layout"].as_str().unwrap();
        let sub = if layout == "422" {
            [true, false]
        } else {
            [false, false]
        };
        assert_eq!(sequence.color.subsampling, sub);
        assert_eq!(
            sequence.color.depth as u64,
            record["depth"].as_u64().unwrap()
        );
        let depth = sequence.color.depth;
        let group = (depth as usize - 8) / 2;
        let layout_index = usize::from(layout == "444");
        groups[group][layout_index] += 1;
        let size = record["size"].as_array().unwrap();
        let width = size[0].as_u64().unwrap() as usize;
        let height = size[1].as_u64().unwrap() as usize;
        let chroma_width = width.div_ceil(1 << usize::from(sub[0]));
        let chroma_height = height.div_ceil(1 << usize::from(sub[1]));
        assert_eq!(
            expected.len(),
            count
                * (width * height + 2 * chroma_width * chroma_height)
                * if depth == 8 { 1 } else { 2 }
        );
        let mut saved: [Option<Header>; 8] = std::array::from_fn(|_| None);
        for frame in obus.iter().filter(|o| o.kind == 6) {
            let h = Header::parse(
                &sequence,
                frame.payload,
                frame.temporal_id,
                frame.spatial_id,
                &saved.each_ref().map(|h| h.as_ref()),
            )
            .unwrap();
            let g = h.grain.as_ref().unwrap();
            if let Some(lag) = record["ar_lag"].as_u64() {
                assert_eq!(g.ar_lag, lag as usize);
            }
            if let Some(flag) = record["chroma_from_luma"].as_bool() {
                assert_eq!(g.chroma_from_luma, flag);
            }
            inherited += usize::from(g.reference.is_some());
            inherited_groups[group][layout_index] += usize::from(g.reference.is_some());
            overlaps[usize::from(g.overlap)] += 1;
            ranges[usize::from(g.restricted_range)] += 1;
            lags[g.ar_lag] += 1;
            from_luma[usize::from(g.chroma_from_luma)] += 1;
            lag_groups[group][layout_index][g.ar_lag] += 1;
            luma_groups[group][layout_index][usize::from(g.chroma_from_luma)] += 1;
            overlap_groups[group][layout_index][usize::from(g.overlap)] += 1;
            range_groups[group][layout_index][usize::from(g.restricted_range)] += 1;
            for (i, s) in saved.iter_mut().enumerate() {
                if h.refresh_flags & (1 << i) != 0 {
                    *s = Some(h.clone());
                }
            }
        }
        if let Some(slot) = record["show_existing_slot"].as_u64() {
            assert!(saved[slot as usize].as_ref().unwrap().showable);
            assert!(saved[slot as usize].as_ref().unwrap().grain.is_some());
            assert_eq!(obus.iter().filter(|o| o.kind == 3).count(), 1);
            assert!(sequence.frame_id_bits.is_none());
            assert!(sequence.timing.is_none());
        }
        let mut decoder = Decoder::new(32 << 20);
        for _ in 0..2 {
            let frames = decoder
                .decode_packet(&data)
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(frames.len(), count);
            for (i, f) in frames.iter().enumerate() {
                assert!(f.show);
                let bytes = pixels(&f.picture);
                assert_eq!(bytes.len(), frame_bytes);
                assert_eq!(
                    bytes
                        .iter()
                        .zip(&expected[i * frame_bytes..(i + 1) * frame_bytes])
                        .position(|(a, b)| a != b),
                    None,
                    "{name} frame {i}: first differing byte"
                );
            }
            decoder.finish().unwrap();
            decoder.reset();
        }
        let container = std::fs::read(root.join(record["webm"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm::WebmVideoReader::open(std::io::Cursor::new(container), 32 << 20)
                .unwrap();
        for _ in 0..2 {
            for i in 0..count {
                assert_eq!(
                    raw(reader.read_frame_raw().unwrap().unwrap()),
                    expected[i * frame_bytes..(i + 1) * frame_bytes],
                    "{name}: WebM frame {i}"
                );
                assert_eq!(
                    reader.frame_interval(),
                    Some((
                        i as u128 * 20_000_000,
                        (i as u128 + 1) * 20_000_000,
                        1_000_000_000
                    )),
                    "{name}: presentation {i}"
                );
            }
            assert!(reader.read_frame_raw().unwrap().is_none());
            reader.rewind();
        }
        assert_eq!(reader.seek_to_sync(40_000_000).unwrap(), 0);
        for i in 0..count {
            assert_eq!(
                raw(reader.read_frame_raw().unwrap().unwrap()),
                expected[i * frame_bytes..(i + 1) * frame_bytes],
                "{name}: seek frame {i}"
            );
            assert_eq!(
                reader.frame_interval(),
                Some((
                    i as u128 * 20_000_000,
                    (i as u128 + 1) * 20_000_000,
                    1_000_000_000
                )),
                "{name}: seek presentation {i}"
            );
        }
    }
    eprintln!(
        "grain coverage: inherited={inherited} overlap={overlaps:?} range={ranges:?} lag={lags:?} luma={from_luma:?}"
    );
    assert!(groups.iter().flatten().all(|n| *n == expected_records / 6));
    if manifest_name.contains("tools") {
        assert!(inherited > 0);
        assert!(inherited_groups.iter().flatten().all(|n| *n > 0));
    }
    assert!(overlaps.iter().all(|v| *v > 0));
    assert!(overlap_groups.iter().flatten().flatten().all(|n| *n > 0));
    if !manifest_name.contains("custom") {
        assert!(ranges.iter().all(|v| *v > 0));
        assert!(range_groups.iter().flatten().flatten().all(|n| *n > 0));
    }
    if !manifest_name.contains("show-existing") {
        assert!(from_luma.iter().all(|v| *v > 0));
        assert!(luma_groups.iter().flatten().flatten().all(|n| *n > 0));
    }
    if manifest_name.contains("custom") {
        assert!(lags.iter().all(|n| *n > 0));
        assert!(lag_groups.iter().flatten().flatten().all(|n| *n > 0));
    }
}

#[test]
fn owned_chroma_grain_tools_pixels_inter_references_reset_and_seek() {
    qualify("av1-chroma-grain-tools-generated.json", 96, 4);
}
#[test]
fn owned_chroma_custom_grain_all_ar_lags_pixels_and_seek() {
    qualify("av1-chroma-grain-custom-generated.json", 48, 4);
}
#[test]
fn owned_chroma_grain_show_existing_reuses_ungrained_reference() {
    qualify("av1-chroma-grain-show-existing-generated.json", 12, 5);
}
