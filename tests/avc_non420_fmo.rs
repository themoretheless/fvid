use fvid::{
    codec::{
        avc::{Pps, SliceGroups, Sps},
        avc_decoder::AvcDecoder,
        avc_picture::decode_intra_slices,
        avc_slice::{SliceHeader, SliceType},
        config::{AvcConfig, NalUnits},
    },
    container::mp4::{Limits, Mp4Reader},
    playback::video::mp4::Mp4VideoReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

fn fixtures() -> (PathBuf, serde_json::Value, Vec<u16>) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/avc-non420-fmo");
    let manifest =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    let golden = std::fs::read(root.join("samples.y16"))
        .unwrap()
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect();
    (root, manifest, golden)
}

#[test]
fn non_420_fmo_progressive_paff_and_mbaff_decode_and_play() {
    let (root, manifest, golden) = fixtures();
    let cases = manifest["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 768);

    for (idx, case) in cases.iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let mode = case["mode"].as_str().unwrap();
        let fmt = case["format"].as_u64().unwrap() as u8;
        let yd = case["luma_depth"].as_u64().unwrap() as u8;
        let cd = case["chroma_depth"].as_u64().unwrap() as u8;
        let w = case["width"].as_u64().unwrap() as usize;
        let h = case["height"].as_u64().unwrap() as usize;
        let at = case["offset"].as_u64().unwrap() as usize;
        let count = w * h;
        let chroma_count = if fmt == 0 { 0 } else { count / 2 };

        let video = std::fs::read(root.join(format!("{name}.mp4"))).unwrap();
        let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
        let config = input.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        assert!(!matches!(pps.slice_groups, SliceGroups::Single), "{name}");
        assert!(!pps.cabac, "{name}");

        let mut decoder = AvcDecoder::new(&config, 16 << 20).unwrap();
        let sample_count = input.tracks()[0].samples.len();
        assert_eq!(
            sample_count,
            if mode == "paff_field" { 6 } else { 3 },
            "{name}"
        );
        let mut decoded = Vec::new();
        let mut packet = Vec::new();
        for s in 0..sample_count {
            input.read_packet(0, s, &mut packet).unwrap();
            if s == 0 && (mode == "progressive" || mode == "paff_frame") {
                let headers: Vec<SliceHeader> = NalUnits::new(&packet, avc.length_size)
                    .unwrap()
                    .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
                    .collect();
                let refs: Vec<&SliceHeader> = headers.iter().collect();
                let direct = decode_intra_slices(&refs, &sps, &pps, 4 << 20)
                    .unwrap_or_else(|e| panic!("direct {name}: {e}"));
                assert_eq!(direct.y, &golden[at..at + count], "{name} direct Y");
            }
            if let Some(pic) = decoder
                .decode_order(&packet)
                .unwrap_or_else(|e| panic!("{name} sample {s}: {e}"))
            {
                assert_eq!(
                    decoder.output_is_field_pair(),
                    mode == "paff_field",
                    "{name}"
                );
                decoded.push(pic);
            }
        }
        assert_eq!(decoded.len(), 3, "{name}");
        for (frame_idx, pic) in decoded.iter().enumerate() {
            assert_eq!(pic.chroma_format().unwrap(), fmt, "{name} f{frame_idx}");
            assert_eq!(pic.dimensions(), (w, h), "{name} f{frame_idx}");
            assert_eq!(pic.bit_depth, yd, "{name} f{frame_idx}");
            assert_eq!(pic.chroma_bit_depth, cd, "{name} f{frame_idx}");
            assert_eq!(pic.y, &golden[at..at + count], "{name} f{frame_idx} Y");
            assert_eq!(
                pic.cb,
                &golden[at + count..at + count + chroma_count],
                "{name} f{frame_idx} Cb"
            );
            assert_eq!(
                pic.cr,
                &golden[at + count + chroma_count..at + count + 2 * chroma_count],
                "{name} f{frame_idx} Cr"
            );
        }

        if idx % 16 == 0 {
            decoder.reset();
            input.read_packet(0, 0, &mut packet).unwrap();
            let first = decoder.decode_order(&packet).unwrap();
            assert_eq!(first.is_some(), mode != "paff_field", "{name} after reset");

            let mut player =
                Mp4VideoReader::open_software(Cursor::new(&video), Limits::default(), 16 << 20)
                    .unwrap();
            for disp_idx in 0..3 {
                let frame = player
                    .read_frame()
                    .unwrap_or_else(|e| panic!("player {name} {disp_idx}: {e}"))
                    .unwrap();
                assert_eq!(frame.picture.y, &golden[at..at + count], "{name} player Y");
                let packed = frame.packed.expect("non-4:2:0 must expose packed frame");
                assert_eq!(
                    packed.frame.subsampling,
                    Some(if fmt == 2 { [2, 1] } else { [2, 2] }),
                    "{name}"
                );
            }
            assert!(player.read_frame().unwrap().is_none(), "{name}");
            player.rewind();
            assert!(player.read_frame().unwrap().is_some(), "{name} rewind");
        }
    }
}

#[test]
fn non_420_fmo_444_remains_explicitly_rejected() {
    let (root, _, _) = fixtures();
    let config = std::fs::read(
        root.join("prog-fmo-f2-y8-c8-t0-d0-comp0-deb1-aso0.avcc"),
    )
    .unwrap();
    let avc = AvcConfig::parse(&config).unwrap();
    let mut sps = Sps::parse(avc.sps[0]).unwrap();
    let pps = Pps::parse(avc.pps[0], &sps).unwrap();
    sps.profile = 244;
    sps.chroma_format = 3;

    let video = std::fs::read(
        root.join("prog-fmo-f2-y8-c8-t0-d0-comp0-deb1-aso0.mp4"),
    )
    .unwrap();
    let mut input = Mp4Reader::open(Cursor::new(&video), Default::default()).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let headers: Vec<SliceHeader> = NalUnits::new(&packet, avc.length_size)
        .unwrap()
        .map(|n| SliceHeader::parse(n.unwrap(), &sps, &pps).unwrap())
        .collect();
    assert_eq!(headers[0].slice_type, SliceType::I);
    let refs: Vec<&SliceHeader> = headers.iter().collect();
    let err = decode_intra_slices(&refs, &sps, &pps, 4 << 20)
        .err()
        .expect("4:4:4 FMO must remain rejected");
    assert!(
        err.to_string()
            .contains("AVC non-4:2:0 PCM requires progressive single-group I slices"),
        "unexpected error: {err}"
    );
}
