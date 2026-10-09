use fvid::{
    codec::{
        config::{HevcConfig, NalUnits},
        hevc_decoder::HevcDecoder,
        hevc_slice::SliceHeader,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;

#[test]
fn separate_colour_planes_reconstruct_all_three_planes() {
    let bytes =
        include_bytes!("fixtures/playback-errors/hevc-separate-colour-planes-pcm-synthetic.mp4");
    let mut reader = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(reader.tracks()[0].samples.len(), 1);
    let config = reader.tracks()[0].configuration.clone();
    let h = HevcConfig::parse(&config).unwrap();
    let mut decoder = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let (sps, pps) = decoder.parameters();
    assert!(sps.separate_colour_plane);
    assert_eq!(sps.chroma_format, 3);
    let mut packet = vec![];
    reader.read_packet(0, 0, &mut packet).unwrap();
    let headers: Vec<_> = NalUnits::new(&packet, h.length_size)
        .unwrap()
        .map(|n| SliceHeader::parse(n.unwrap(), sps, pps, 16 << 20).unwrap())
        .collect();
    assert_eq!(
        headers.iter().map(|h| h.colour_plane).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert!(headers.iter().all(|h| h.first && h.entropy_byte_offset > 0));
    // Each plane's entropy stream must remain decodable as the authored mono
    // seed. This rules out a damaged CABAC/PCM payload as the failure cause.
    let mut mono = sps.clone();
    mono.chroma_format = 0;
    mono.separate_colour_plane = false;
    let gold = include_bytes!("fixtures/playback-errors/hevc-pcm-mono-active-rext8.yuv");
    for header in &headers {
        let picture =
            fvid::codec::hevc_picture::decode(&mono, pps, header, 0, &[vec![], vec![]], 16 << 20)
                .unwrap();
        assert_eq!(
            picture.planes[0]
                .samples()
                .iter()
                .map(|&v| v as u8)
                .collect::<Vec<_>>(),
            gold
        );
    }
    assert_eq!(decoder.slice_headers(&packet).unwrap().len(), 3);
    let decoded = decoder.decode_packet(&packet).unwrap().unwrap();
    for plane in &decoded.picture.planes {
        assert_eq!(
            plane.samples().iter().map(|&v| v as u8).collect::<Vec<_>>(),
            gold
        );
    }
}

#[test]
fn separate_colour_plane_partition_errors_are_specific() {
    for (bytes, error) in [
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-separate-colour-planes-missing-synthetic.mp4"
            )
            .as_slice(),
            "HEVC access unit is missing a colour plane",
        ),
        (
            include_bytes!(
                "fixtures/playback-errors/hevc-separate-colour-planes-duplicate-synthetic.mp4"
            )
            .as_slice(),
            "HEVC slice addresses must increase within one picture",
        ),
    ] {
        let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let mut d =
            HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
        let mut packet = vec![];
        r.read_packet(0, 0, &mut packet).unwrap();
        assert_eq!(d.decode_packet(&packet).err().unwrap().to_string(), error);
    }
}

#[test]
fn reordered_and_inter_wpp_planes_match_saved_pixels_after_rewind() {
    for (name, seed) in [
        (
            "hevc-separate-colour-planes-reordered-synthetic.mp4",
            "hevc-pcm-mono-active-rext8",
        ),
        (
            "hevc-separate-colour-planes-reference-wpp-synthetic.mp4",
            "hevc-pcm-mono-reference-wpp-rext8",
        ),
    ] {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
        let bytes = std::fs::read(path.join(name)).unwrap();
        let gold = std::fs::read(path.join(format!("{seed}.yuv"))).unwrap();
        let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(bytes),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut output = vec![];
            while let Some(frame) = r.read_frame().unwrap() {
                let packed = frame.packed.as_ref().expect("444 playback geometry");
                assert_eq!(packed.frame.subsampling, Some([1, 1]));
                output.extend_from_slice(&packed.frame.data);
            }
            let pixels = 64 * 64;
            let expected: Vec<u8> = gold
                .chunks_exact(pixels)
                .flat_map(|frame| frame.iter().chain(frame).chain(frame).copied())
                .collect();
            assert!(
                output == expected,
                "{name}: decoded bytes {} expected {}",
                output.len(),
                expected.len()
            );
            if pass == 0 {
                r.rewind();
            } else if pass == 1 {
                let seed_bytes = std::fs::read(path.join(format!("{seed}.mp4"))).unwrap();
                let mut baseline = fvid::playback_mp4::Mp4VideoReader::open_software(
                    Cursor::new(seed_bytes),
                    Default::default(),
                    16 << 20,
                )
                .unwrap();
                let sync = baseline.seek_to_sync(2);
                assert_eq!(sync, 0);
                assert_eq!(r.seek_to_sync(2), sync);
            }
        }
    }
}

#[test]
fn distinct_colour_planes_follow_ids_and_budget() {
    let bytes = include_bytes!(
        "fixtures/playback-errors/hevc-separate-colour-planes-distinct-synthetic.mp4"
    );
    let gold = include_bytes!(
        "fixtures/playback-errors/hevc-separate-colour-planes-distinct-synthetic.yuv"
    );
    let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    let config = r.tracks()[0].configuration.clone();
    let mut packet = vec![];
    r.read_packet(0, 0, &mut packet).unwrap();
    let mut d = HevcDecoder::from_configuration(&config, 16 << 20).unwrap();
    let decoded = d.decode_packet(&packet).unwrap().unwrap();
    for (i, plane) in decoded.picture.planes.iter().enumerate() {
        let expected = &gold[i * 4096..(i + 1) * 4096];
        assert!(
            plane
                .samples()
                .iter()
                .zip(expected)
                .all(|(&a, &b)| a == u16::from(b))
        );
    }
    assert_ne!(&gold[..4096], &gold[4096..8192]);
    d.reset();
    assert!(d.decode_packet(&packet).unwrap().is_some());
    let mut limited = HevcDecoder::from_configuration(&config, 300_000).unwrap();
    assert_eq!(
        limited.decode_packet(&packet).err().unwrap().to_string(),
        "HEVC colour planes exceed decode budget"
    );
}

#[test]
fn distinct_inter_planes_keep_their_own_reference_pixels_across_au_orders() {
    let bytes = include_bytes!(
        "fixtures/playback-errors/hevc-separate-colour-planes-distinct-reference-wpp-synthetic.mp4"
    );
    let gold = include_bytes!(
        "fixtures/playback-errors/hevc-separate-colour-planes-distinct-reference-wpp-synthetic.yuv"
    );
    assert_eq!(gold.len(), 3 * 3 * 4096);
    let mut r = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
    assert_eq!(r.tracks()[0].samples.len(), 3);
    let mut d = HevcDecoder::from_configuration(&r.tracks()[0].configuration, 16 << 20).unwrap();
    let orders = [[2, 0, 1], [1, 2, 0], [0, 2, 1]];
    for _ in 0..2 {
        let mut packet = vec![];
        for (frame, order) in orders.iter().enumerate() {
            r.read_packet(0, frame, &mut packet).unwrap();
            let headers = d.slice_headers(&packet).unwrap();
            assert_eq!(
                headers.iter().map(|h| h.colour_plane).collect::<Vec<_>>(),
                order
            );
            if frame > 0 {
                assert!(
                    headers
                        .iter()
                        .all(|h| h.slice_type != fvid::codec::hevc_cabac::SliceType::I
                            && h.references[0] > 0)
                );
            }
            let decoded = d.decode_packet(&packet).unwrap().unwrap();
            assert_eq!(decoded.poc, frame as i32);
            for (plane, pixels) in decoded.picture.planes.iter().enumerate() {
                let start = (frame * 3 + plane) * 4096;
                let expected = &gold[start..start + 4096];
                assert_eq!(pixels.samples().len(), expected.len());
                assert!(
                    pixels
                        .samples()
                        .iter()
                        .zip(expected)
                        .all(|(&a, &b)| a == u16::from(b)),
                    "frame {frame} plane {plane}"
                );
            }
        }
        d.reset();
    }
    let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes),
        Default::default(),
        16 << 20,
    )
    .unwrap();
    for pass in 0..3 {
        let mut output = vec![];
        while let Some(frame) = player.read_frame().unwrap() {
            output.extend_from_slice(&frame.packed.unwrap().frame.data);
        }
        assert!(output == gold, "playback pass {pass}");
        if pass == 0 {
            player.rewind();
        }
        if pass == 1 {
            assert_eq!(player.seek_to_sync(2), 0);
        }
    }
}
