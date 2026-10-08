//! Exact externally anchored camera tile list; no external codec in tests.
use fvid::codec::{av1::Obus, av1_decoder::Decoder, av1_frame::Header, av1_tile_list::TileList};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(format!("av1-tile-list-{name}")),
    )
    .unwrap()
}
#[test]
fn owned_camera_tile_list_pixels_external_context_and_preserved_sparse_output() {
    camera_tile_list_case("", 64, 8, 420, 0);
}
#[test]
fn sb128_camera_tile_list_pixels_distinct_anchors_and_sparse_canvas() {
    camera_tile_list_case("sb128-", 128, 8, 420, 0);
}
#[test]
fn camera_tile_list_high_depth_and_full_chroma_matrix() {
    for sb in [64, 128] {
        for depth in [8, 10, 12] {
            for chroma in [420, 422, 444] {
                if depth == 8 && chroma == 420 {
                    continue;
                }
                camera_tile_list_case(&format!("d{depth}-c{chroma}-sb{sb}-"), sb, depth, chroma, 0);
            }
        }
    }
}
#[test]
fn lossy_camera_tile_list_transform_reconstruction_matrix() {
    for sb in [64, 128] {
        for depth in [8, 10, 12] {
            for chroma in [420, 422, 444] {
                camera_tile_list_case(
                    &format!("q32-d{depth}-c{chroma}-sb{sb}-"),
                    sb,
                    depth,
                    chroma,
                    32,
                );
            }
        }
    }
}
#[test]
fn adapted_anchor_cdf_camera_tile_list_matrix() {
    camera_context_matrix("cdf");
}
#[test]
fn primary_ref_none_camera_tile_list_matrix() {
    camera_context_matrix("none");
}
fn camera_context_matrix(context: &str) {
    for q in [0, 32] {
        for sb in [64, 128] {
            for depth in [8, 10, 12] {
                for chroma in [420, 422, 444] {
                    let tag = if q > 0 {
                        format!("q{q}-d{depth}-c{chroma}-sb{sb}-")
                    } else if depth == 8 && chroma == 420 {
                        if sb == 64 {
                            String::new()
                        } else {
                            "sb128-".to_owned()
                        }
                    } else {
                        format!("d{depth}-c{chroma}-sb{sb}-")
                    };
                    camera_tile_list_case(&format!("{context}-{tag}"), sb, depth, chroma, q);
                }
            }
        }
    }
}
fn camera_tile_list_case(prefix: &str, sb: usize, depth: u8, chroma: usize, q: u8) {
    let reset = prefix.starts_with("none-");
    let adapted = reset || prefix.starts_with("cdf-");
    let bytes = |name: &str| bytes(&format!("{prefix}{name}"));
    let anchor = bytes("anchor.obu");
    let header = bytes("header.obu");
    let list = bytes("list.obu");
    let golden = bytes("list.yuv");
    let mut decoder = Decoder::new(32 << 20);
    let frames = decoder.decode_packet(&anchor).unwrap();
    assert_eq!(frames.len(), 1);
    let anchors = vec![frames[0].picture.clone()];
    let seq = Obus::new(&anchor)
        .map(Result::unwrap)
        .find(|o| o.kind == 1)
        .unwrap();
    let sequence = fvid::codec::av1_sequence::Sequence::parse(seq.payload).unwrap();
    let original = Obus::new(&anchor)
        .map(Result::unwrap)
        .find(|o| o.kind == 6)
        .unwrap();
    let ah = Header::parse_intra(&sequence, original.payload, 0, 0).unwrap();
    assert_eq!(
        !ah.disable_cdf_update, adapted,
        "{prefix}: anchor CDF adaptation"
    );
    if adapted {
        assert!(
            !ah.disable_frame_end_update,
            "{prefix}: anchor must save adapted CDF"
        );
    }
    let frame = Obus::new(&header)
        .map(Result::unwrap)
        .find(|o| o.kind == 6)
        .unwrap();
    let camera = Header::parse(&sequence, frame.payload, 0, 0, &[Some(&ah); 8]).unwrap();
    assert_eq!(camera.frame_type, 1);
    if adapted {
        assert_eq!(
            camera.primary_reference,
            if reset { 7 } else { 0 },
            "{prefix}: encoded primary CDF selection"
        );
    }
    assert_eq!(camera.refresh_flags, 0);
    assert_eq!(camera.quant.base > 0, q > 0);
    assert!(camera.disable_cdf_update && camera.disable_frame_end_update);
    let obu = Obus::new(&list).next().unwrap().unwrap();
    assert_eq!(obu.kind, 8);
    let parsed = TileList::parse(obu.payload).unwrap();
    assert_eq!(parsed.grid, [2, 2]);
    assert_eq!(
        parsed
            .entries
            .iter()
            .map(|e| (e.row, e.column))
            .collect::<Vec<_>>(),
        vec![(1, 1), (0, 0), (1, 0), (0, 1)]
    );
    let output = decoder
        .decode_tile_list(&camera, &anchors, obu.payload, None)
        .unwrap();
    assert_eq!(output.size, [sb * 2, sb * 2]);
    assert_eq!(sequence.superblock128, sb == 128);
    let pixels = |output: &fvid::codec::av1_tile_list::Output| {
        output
            .planes
            .iter()
            .flat_map(|p| p.samples.iter())
            .flat_map(|v| {
                if depth == 8 {
                    vec![*v as u8]
                } else {
                    v.to_le_bytes().to_vec()
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(output.depth, depth);
    assert_eq!(output.subsampling, [chroma != 444, chroma == 420]);
    assert_eq!(pixels(&output), golden, "{prefix}");
    if depth > 8 {
        assert!(output.planes[0].samples.iter().any(|v| *v > 255));
    }
    if adapted {
        let mut default_cdf = camera.clone();
        default_cdf.primary_reference = if reset { 0 } else { 7 };
        match decoder.decode_tile_list(&default_cdf, &anchors, obu.payload, None) {
            Ok(changed) => assert_ne!(pixels(&changed), golden, "{prefix}: primary CDF mutation"),
            Err(_) => decoder
                .finish()
                .expect("advanced context refusal must not poison ordinary decoder"),
        }
    }
    // Distinct anchors make selecting the wrong entry's anchor observable.
    let mut alternate = (*anchors[0]).clone();
    for plane in &mut alternate.planes {
        for sample in &mut plane.samples {
            *sample += 9 << (depth - 8);
        }
    }
    let distinct = vec![anchors[0].clone(), std::sync::Arc::new(alternate)];
    let multi_bytes = bytes("multi-list.obu");
    let multi_obu = Obus::new(&multi_bytes).next().unwrap().unwrap();
    let multi_entries = TileList::parse(multi_obu.payload).unwrap();
    assert_eq!(
        multi_entries
            .entries
            .iter()
            .map(|e| e.anchor)
            .collect::<Vec<_>>(),
        vec![0, 1, 0, 1]
    );
    let multi = decoder
        .decode_tile_list(&camera, &distinct, multi_obu.payload, None)
        .unwrap();
    assert_eq!(multi.decoded_tiles, 4);
    assert!(multi.inter_blocks > 0);
    let multi_pixels = pixels(&multi);
    assert_eq!(multi_pixels, bytes("multi-list.yuv"));
    assert_ne!(multi_pixels, golden);
    let mut differs_from_source = false;
    for p in 0..3 {
        let width = sb >> usize::from(p != 0 && chroma != 444);
        let height = sb >> usize::from(p != 0 && chroma == 420);
        for y in 0..height * 2 {
            for x in 0..width * 2 {
                let index = y / height * 2 + x / width;
                let source = [3, 0, 2, 1][index];
                let source_x = source % 2 * width + x % width;
                let source_y = source / 2 * height + y % height;
                let authored =
                    ((71 + (3 * source_x + 5 * source_y + 23 * p) % 96) << (depth - 8)) as u16;
                let offset = (9 << (depth - 8)) * (index % 2) as u16;
                let i = y * width * 2 + x;
                if q == 0 {
                    assert_eq!(
                        output.planes[p].samples[i], authored,
                        "{prefix}, plane {p}, {x},{y}"
                    );
                    assert_eq!(
                        multi.planes[p].samples[i],
                        output.planes[p].samples[i] + offset
                    );
                } else {
                    let detail = ((source_x / 8 + source_y / 8 + p) % 7) as i32 - 3;
                    let source_sample = (i32::from(authored) + (detail << (depth - 8))) as u16;
                    differs_from_source |= output.planes[p].samples[i] != source_sample;
                }
            }
        }
    }
    assert_eq!(
        differs_from_source,
        q > 0,
        "{prefix}: quantization must be observable"
    );
    if q > 0 {
        let mut wrong_quant = camera.clone();
        wrong_quant.quant.base = wrong_quant.quant.base.saturating_add(16);
        let changed = decoder
            .decode_tile_list(&wrong_quant, &anchors, obu.payload, None)
            .expect("dequantization control must reconstruct, not merely refuse");
        assert_ne!(
            pixels(&changed),
            golden,
            "{prefix}: dequantization mutation"
        );
    }
    // Mutation sensitivity: identical pointers cannot accidentally pass this oracle.
    let wrong = decoder
        .decode_tile_list(
            &camera,
            &[anchors[0].clone(), anchors[0].clone()],
            multi_obu.payload,
            None,
        )
        .unwrap();
    assert_ne!(wrong.planes[0].samples, multi.planes[0].samples);
    // A sparse list replaces the first tile, preserving all other canvas samples.
    let entry = &parsed.entries[1];
    let mut partial = vec![1, 1, 0, 0, 0, entry.row as u8, entry.column as u8];
    partial.extend_from_slice(&((entry.data.len() - 1) as u16).to_be_bytes());
    partial.extend_from_slice(entry.data);
    let sparse = decoder
        .decode_tile_list(&camera, &anchors, &partial, Some(&output))
        .unwrap();
    for p in 0..3 {
        let width = sb >> usize::from(p != 0 && chroma != 444);
        let height = sb >> usize::from(p != 0 && chroma == 420);
        let stride = width * 2;
        for y in 0..height * 2 {
            for x in 0..stride {
                let expected = if x < width && y < height {
                    output.planes[p].samples[y * stride + x + width]
                } else {
                    output.planes[p].samples[y * stride + x]
                };
                assert_eq!(sparse.planes[p].samples[y * stride + x], expected);
            }
        }
    }
    // Dedicated tile decoding leaves the ordinary reference/CDF state intact.
    let again = decoder
        .decode_tile_list(&camera, &anchors, obu.payload, None)
        .unwrap();
    assert_eq!(again.planes[0].samples, output.planes[0].samples);
    assert!(
        decoder
            .decode_tile_list(&camera, &[], obu.payload, None)
            .err()
            .unwrap()
            .to_string()
            .contains("missing AV1 external tile anchor")
    );
    let mut bad = obu.payload.to_vec();
    bad[4] = 128;
    assert!(
        TileList::parse(&bad)
            .err()
            .unwrap()
            .to_string()
            .contains("anchor index")
    );
    for n in 0..obu.payload.len() {
        assert!(TileList::parse(&obu.payload[..n]).is_err());
    }
    assert_eq!(output.decoded_tiles, 4);
    assert!(output.inter_blocks > 0);
    assert_eq!(sparse.decoded_tiles, 1);
    let mut indexed = obu.payload.to_vec();
    indexed[4] = 127;
    let many = vec![anchors[0].clone(); 128];
    let indexed = decoder
        .decode_tile_list(&camera, &many, &indexed, None)
        .unwrap();
    assert_eq!(indexed.planes[0].samples, output.planes[0].samples);
    let mut huge = obu.payload.to_vec();
    huge[0] = 255;
    huge[1] = 255;
    assert!(
        decoder
            .decode_tile_list(&camera, &anchors, &huge, None)
            .err()
            .unwrap()
            .to_string()
            .contains("memory budget")
    );
    let mut bad_coordinate = obu.payload.to_vec();
    bad_coordinate[5] = 2;
    assert!(
        decoder
            .decode_tile_list(&camera, &anchors, &bad_coordinate, None)
            .err()
            .unwrap()
            .to_string()
            .contains("coordinate")
    );
    let mut bad_context = camera.clone();
    bad_context.refresh_flags = 1;
    assert!(
        decoder
            .decode_tile_list(&bad_context, &anchors, obu.payload, None)
            .err()
            .unwrap()
            .to_string()
            .contains("camera context")
    );
    let mut bad_canvas = output.clone();
    bad_canvas.planes[0].width = 1;
    assert!(
        decoder
            .decode_tile_list(&camera, &anchors, obu.payload, Some(&bad_canvas))
            .err()
            .unwrap()
            .to_string()
            .contains("previous tile output")
    );
    let mut too_many = huge;
    too_many[2] = 2;
    too_many[3] = 0;
    assert!(
        TileList::parse(&too_many)
            .err()
            .unwrap()
            .to_string()
            .contains("count")
    );
    let mut trailing = obu.payload.to_vec();
    trailing.push(0);
    assert!(
        TileList::parse(&trailing)
            .err()
            .unwrap()
            .to_string()
            .contains("trailing")
    );
    decoder.finish().unwrap();
}
#[test]
fn general_packet_tile_list_refusal_reproduces_missing_side_information() {
    let mut decoder = Decoder::new(32 << 20);
    decoder.decode_packet(&bytes("anchor.obu")).unwrap();
    let error = decoder
        .decode_packet(&bytes("list.obu"))
        .err()
        .unwrap()
        .to_string();
    assert!(
        error.contains("tile list requires external camera context"),
        "{error}"
    );
}
