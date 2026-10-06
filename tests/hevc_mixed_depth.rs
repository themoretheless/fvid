use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::io::Cursor;
#[test]
fn mixed_component_depths_match_all_hm_samples_and_reset() {
    macro_rules! sample {
        ($stem:literal,$y:literal,$c:literal) => {
            (
                include_bytes!(concat!("fixtures/playback-errors/", $stem, ".mp4")).as_slice(),
                include_bytes!(concat!("fixtures/playback-errors/", $stem, ".yuv")).as_slice(),
                [$y, $c],
            )
        };
    }
    for (sample_index, (source, oracle, depths)) in [
        sample!("hevc-mixed-depth-y8-c10", 8, 10),
        sample!("hevc-mixed-depth-y8-c10-wpp", 8, 10),
        sample!("hevc-mixed-depth-y8-c10-mixed-tiles", 8, 10),
        sample!("hevc-mixed-depth-y8-c10-parallel", 8, 10),
        sample!("hevc-mixed-depth-y8-c10-cross", 8, 10),
        sample!("hevc-mixed-depth-y10-c8", 10, 8),
        sample!("hevc-mixed-depth-y10-c8-wpp", 10, 8),
        sample!("hevc-mixed-depth-y10-c8-mixed-tiles", 10, 8),
        sample!("hevc-mixed-depth-y10-c8-parallel", 10, 8),
        sample!("hevc-mixed-depth-y10-c8-cross", 10, 8),
        sample!("hevc-mixed-depth-y12-c10", 12, 10),
        sample!("hevc-mixed-depth-y12-c10-wpp", 12, 10),
        sample!("hevc-mixed-depth-y12-c10-mixed-tiles", 12, 10),
        sample!("hevc-mixed-depth-y12-c10-parallel", 12, 10),
        sample!("hevc-mixed-depth-y12-c10-cross", 12, 10),
    ]
    .into_iter()
    .enumerate()
    {
        let mut input = Mp4Reader::open(Cursor::new(source), Default::default()).unwrap();
        let mut decoder =
            HevcDecoder::from_configuration(&input.tracks()[0].configuration, 16 << 20).unwrap();
        assert_eq!(decoder.parameters().0.depth, depths);
        assert_eq!(decoder.parameters().0.chroma_format, 3);
        assert!(decoder.parameters().0.sao);
        assert!(!decoder.parameters().1.deblocking.disabled);
        assert_eq!(input.tracks()[0].samples.len(), 3);
        let mode = sample_index % 5;
        let pps = decoder.parameters().1;
        assert_eq!(pps.entropy_sync, matches!(mode, 1 | 3));
        assert_eq!(pps.tiles.is_some(), mode == 2);
        assert_eq!(pps.cross_component_prediction, mode == 4);
        if mode == 3 {
            assert_eq!(decoder.parameters().0.dimensions, [128, 96]);
            assert!(!pps.constrained_intra);
        }
        if mode == 2 {
            assert_eq!(pps.tiles.as_ref().unwrap().column_widths.len(), 2);
            let mut packet = Vec::new();
            input.read_packet(0, 0, &mut packet).unwrap();
            let headers = decoder.slice_headers(&packet).unwrap();
            assert!(headers.iter().any(|s| s.dependent));
            assert!(headers.iter().filter(|s| !s.dependent).count() > 1);
        }
        let [width, height] = decoder.parameters().0.dimensions.map(|n| n as usize);
        let display_depth = depths[0].max(depths[1]);
        let expected_display: Vec<u8> = oracle
            .chunks_exact(2)
            .enumerate()
            .flat_map(|(i, b)| {
                let component = (i % (width * height * 3)) / (width * height);
                (u16::from_le_bytes([b[0], b[1]])
                    << (display_depth - depths[usize::from(component != 0)]))
                .to_le_bytes()
            })
            .collect();
        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(source),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for _ in 0..2 {
            let mut displayed = Vec::new();
            while let Some(frame) = player.read_frame().unwrap() {
                let packed = frame
                    .packed
                    .expect("HEVC display must retain component geometry and precision");
                assert_eq!(packed.depth, display_depth);
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                assert_eq!([packed.frame.width, packed.frame.height], [width, height]);
                displayed.extend_from_slice(&packed.frame.data);
            }
            assert_eq!(
                displayed, expected_display,
                "mixed-depth display differs from HM samples"
            );
            player.rewind();
        }
        for _ in 0..2 {
            let mut pixels = Vec::new();
            for index in 0..3 {
                let mut packet = Vec::new();
                input.read_packet(0, index, &mut packet).unwrap();
                let frame = decoder.decode_packet(&packet).unwrap().unwrap();
                assert_eq!(frame.picture.depth, depths);
                for plane in &frame.picture.planes {
                    for &v in plane.samples() {
                        pixels.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
            assert_eq!(pixels.len(), oracle.len());
            assert!(
                pixels == oracle,
                "depths {depths:?}, mismatch {:?}",
                pixels.iter().zip(oracle).position(|(a, b)| a != b)
            );
            decoder.reset();
        }
    }
}
