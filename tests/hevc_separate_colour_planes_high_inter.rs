use fvid::{
    codec::{hevc_cabac::SliceType, hevc_decoder::HevcDecoder},
    container::mp4::Mp4Reader,
};
use std::io::Cursor;

#[test]
fn high_depth_inter_planes_keep_filtered_and_wpp_pixels_after_reset_and_seek() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for (name, seed, depth) in [
        (
            "hevc-separate-colour-planes-filtered10-synthetic",
            "hevc-monochrome-filtered-rext10",
            10,
        ),
        (
            "hevc-separate-colour-planes-wpp12-synthetic",
            "hevc-monochrome-wpp-rext12",
            12,
        ),
        (
            "hevc-separate-colour-planes-parallel12-synthetic",
            "hevc-monochrome-parallel-rext12",
            12,
        ),
        (
            "hevc-separate-colour-planes-mixed-tiles12-synthetic",
            "hevc-monochrome-mixed-tiles-rext12",
            12,
        ),
    ] {
        let bytes = std::fs::read(path.join(format!("{name}.mp4"))).unwrap();
        let gold = std::fs::read(path.join(format!("{seed}.yuv"))).unwrap();
        let mut reader = Mp4Reader::open(Cursor::new(&bytes), Default::default()).unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&reader.tracks()[0].configuration, 16 << 20).unwrap();
        let count = reader.tracks()[0].samples.len();
        assert!(count > 1);
        let sps = decoder.parameters().0;
        assert_eq!(sps.depth[0], depth);
        let pixels = sps.dimensions[0] as usize * sps.dimensions[1] as usize;
        assert_eq!(gold.len(), count * pixels * 2);
        assert!(sps.sao);
        if depth == 12 && !name.contains("tiles") {
            assert!(decoder.parameters().1.entropy_sync);
        }
        if name.contains("tiles") {
            assert!(decoder.parameters().1.tiles.is_some());
            assert!(decoder.parameters().1.dependent_slices);
        }
        let expected: Vec<_> = gold
            .chunks_exact(pixels * 2)
            .flat_map(|f| f.iter().chain(f).chain(f).copied())
            .collect();
        for _ in 0..2 {
            let mut packet = vec![];
            let mut inter = 0;
            let mut active_sao = false;
            for frame in 0..count {
                reader.read_packet(0, frame, &mut packet).unwrap();
                let headers = decoder.slice_headers(&packet).unwrap();
                if name.contains("tiles") {
                    assert!(headers.len() > 3);
                    assert!(headers.iter().any(|h| h.dependent));
                    for plane in 0..3 {
                        assert!(
                            headers
                                .iter()
                                .filter(|h| h.colour_plane == plane && !h.dependent)
                                .count()
                                > 1
                        );
                        assert!(
                            headers
                                .iter()
                                .any(|h| h.colour_plane == plane && h.dependent)
                        );
                    }
                } else {
                    assert_eq!(headers.len(), 3);
                }
                if name.contains("parallel") {
                    assert!(headers.iter().all(|h| h.entropy_substreams.len() > 1));
                }
                if headers[0].slice_type != SliceType::I {
                    inter += 1;
                    assert!(headers.iter().all(|h| h.references[0] > 0));
                }
                let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
                active_sao |= decoded.picture.sao.iter().any(|ctu| {
                    use fvid::codec::hevc_sao::Sao;
                    match ctu[0] {
                        Sao::Band { offsets, .. } | Sao::Edge { offsets, .. } => {
                            offsets.iter().any(|&v| v != 0)
                        }
                        Sao::Off => false,
                    }
                });
                assert_eq!(decoded.picture.depth, [depth; 2]);
                let samples: Vec<_> = gold[frame * pixels * 2..(frame + 1) * pixels * 2]
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect();
                for p in &decoded.picture.planes {
                    assert_eq!(p.samples(), samples);
                }
            }
            assert!(inter > 0);
            assert!(
                active_sao,
                "{name}: fixture must exercise SAO, not only enable its syntax"
            );
            decoder.reset();
        }
        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(&bytes),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut output = vec![];
            while let Some(frame) = player.read_frame().unwrap() {
                let packed = frame.packed.unwrap();
                assert_eq!(packed.depth, depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                output.extend_from_slice(&packed.frame.data);
            }
            assert!(output == expected, "{name}: pass {pass}");
            if pass == 0 {
                player.rewind();
            }
            if pass == 1 {
                assert_eq!(player.seek_to_sync(2), 0);
            }
        }
    }
}
