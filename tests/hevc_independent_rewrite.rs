use fvid::{
    codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_nal::NalHeader},
    container::mp4::{Limits, Mp4Reader},
};
use std::{io::Cursor, path::PathBuf};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-independent-rewrite")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("manifest.json")).unwrap()
}
#[test]
fn rewritten_independent_streams_decode_without_an_invented_vps() {
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = bytes(&(name.to_owned() + ".mp4"));
        let oracle = bytes(c["oracle"].as_str().unwrap());
        let mut demux = Mp4Reader::open_indexed(Cursor::new(&data), Limits::default()).unwrap();
        let cfg = demux.tracks()[0].configuration.clone();
        let config = HevcConfig::parse(&cfg).unwrap();
        assert!(
            config
                .arrays
                .iter()
                .all(|a| a.nal_type != 32 || a.units.is_empty())
        );
        for n in config.arrays.iter().flat_map(|a| &a.units) {
            assert_eq!(NalHeader::parse(n).unwrap().layer_id, 0);
        }
        let mut d = HevcDecoder::from_inband_configuration(&cfg, BUDGET).unwrap();
        for _ in 0..2 {
            let mut seen = 0;
            for i in 0..demux.tracks()[0].samples.len() {
                let mut packet = Vec::new();
                demux.read_packet(0, i, &mut packet).unwrap();
                if let Some(f) = d
                    .decode_packet(&packet)
                    .unwrap_or_else(|e| panic!("{name}: {e}"))
                {
                    let pixels: Vec<_> = f
                        .picture
                        .planes
                        .iter()
                        .flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap()))
                        .collect();
                    assert!(
                        pixels == oracle[..pixels.len()],
                        "{name}: decoded INBLD pixels"
                    );
                    assert_eq!(d.hdr().light.max_cll, 100.0);
                    seen += 1;
                }
            }
            assert_eq!(seen, 2, "{name}");
            d.reset();
            demux.invalidate_position();
        }
    }
}

#[test]
fn independent_rewrite_preserves_saved_bytes_and_checks_temporal_capability() {
    use fvid::codec::{config::NalUnits, hevc_decoder::SubstreamScan, hevc_vps::LayeredVps};
    use std::sync::Arc;
    for family in ["pcm", "multislice", "colour-planes", "subset", "external"] {
        for available in [0, 1] {
            let name = format!("{family}-available{available}");
            let input = bytes(&(name.clone() + ".packet"));
            let expected = bytes(&(name + ".rewritten"));
            let vps = NalUnits::new(&input, 4).unwrap().next().unwrap().unwrap();
            let vps = Arc::new(LayeredVps::parse(vps, BUDGET).unwrap());
            let mut scan = SubstreamScan::default();
            scan.observe(&input, 4).unwrap();
            let selection = scan.select(vps.clone(), 2, 0).unwrap();
            assert_eq!(
                selection
                    .rewrite_independent_packet(&input, 4, BUDGET)
                    .unwrap(),
                expected
            );
            assert!(selection.rewrite_independent_packet(&input, 4, 16).is_err());
            for width in [1u8, 2] {
                let reframe = |data: &[u8]| -> Vec<u8> {
                    NalUnits::new(data, 4)
                        .unwrap()
                        .map(|n| n.unwrap())
                        .filter(|n| n.len() < (1usize << (width * 8)))
                        .flat_map(|n| {
                            let mut v =
                                (n.len() as u32).to_be_bytes()[4 - width as usize..].to_vec();
                            v.extend_from_slice(n);
                            v
                        })
                        .collect()
                };
                assert_eq!(
                    selection
                        .rewrite_independent_packet(&reframe(&input), width, BUDGET)
                        .unwrap(),
                    reframe(&expected)
                );
            }
            if family != "external" {
                let mut absent = (*vps).clone();
                let profile = absent.vps.profile.profile.as_mut().unwrap();
                let mut sub = *profile;
                sub.constraints &= !1;
                absent.vps.profile.sub_layers[0].profile = Some(sub);
                let absent = Arc::new(absent);
                let low = scan.select(absent.clone(), 2, 0).unwrap();
                assert_eq!(
                    low.rewrite_independent_packet(&input, 4, BUDGET)
                        .err()
                        .unwrap()
                        .to_string(),
                    "HEVC independent-layer rewrite requires signalled INBLD capability"
                );
                let high = scan.select(absent, 2, 2).unwrap();
                assert_eq!(
                    high.rewrite_independent_packet(&input, 4, BUDGET).unwrap(),
                    expected
                );
            }
        }
    }
    let input = bytes("no-inbld.packet");
    let nal = NalUnits::new(&input, 4).unwrap().next().unwrap().unwrap();
    let vps = Arc::new(LayeredVps::parse(nal, BUDGET).unwrap());
    let mut scan = SubstreamScan::default();
    scan.observe(&input, 4).unwrap();
    assert_eq!(
        scan.select(vps, 2, 0)
            .unwrap()
            .rewrite_independent_packet(&input, 4, BUDGET)
            .err()
            .unwrap()
            .to_string(),
        "HEVC independent-layer rewrite requires signalled INBLD capability"
    );
}
fn raw_pixels(f: fvid::playback_native::RawFrame) -> Vec<u8> {
    use fvid::playback_native::RawFrame;
    match f {
        RawFrame::Planar(p) => p.frame.data.clone(),
        RawFrame::Planar8(p) => p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect(),
        RawFrame::Avc { picture, .. } => picture
            .y
            .iter()
            .chain(&picture.cb)
            .chain(&picture.cr)
            .map(|&v| u8::try_from(v).unwrap())
            .collect(),
        _ => panic!("expected owned planar output"),
    }
}
#[test]
fn rewritten_independent_movies_play_and_seek_through_public_adapters() {
    use fvid::{playback::video::mp4::LayeredMp4VideoReader, playback_native::NativeReader};
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let oracle = bytes(c["oracle"].as_str().unwrap());
        let source = bytes(&(name.to_owned() + "-source.mp4"));
        let mut original =
            LayeredMp4VideoReader::open(Cursor::new(source), Limits::default(), 2, 0, BUDGET)
                .unwrap();
        for _ in 0..2 {
            let f = original.read_frame().unwrap().unwrap();
            assert_eq!(f.decoded.layer_id, 3);
            assert_eq!(f.decoded.hdr.light.max_cll, 100.0);
            let p: Vec<_> = f
                .decoded
                .picture
                .planes
                .iter()
                .flat_map(|p| p.samples().iter().map(|&v| u8::try_from(v).unwrap()))
                .collect();
            assert!(p == oracle[..p.len()]);
        }
        for ext in ["mp4", "mkv"] {
            let data = bytes(&format!("{name}.{ext}"));
            let mut reader = NativeReader::software(Cursor::new(data), BUDGET)
                .unwrap_or_else(|e| panic!("{name}.{ext}: {e}"));
            for _ in 0..2 {
                for _ in 0..2 {
                    let f = reader
                        .read_frame_raw()
                        .unwrap_or_else(|e| panic!("{name}.{ext}: {e}"))
                        .unwrap();
                    let p = raw_pixels(f);
                    assert!(p == oracle[..p.len()], "{name}.{ext}");
                    assert_eq!(reader.hdr().light.max_cll, 100.0);
                    assert_eq!(reader.video_codec(), "H.265");
                }
                assert!(reader.read_frame_raw().unwrap().is_none());
                reader.rewind().unwrap();
            }
            let first = if ext == "mp4" {
                c["first"].as_u64().unwrap()
            } else {
                0
            };
            for index in [first + 1, first] {
                let f = reader
                    .seek_raw(std::time::Duration::from_millis(index * 40))
                    .unwrap()
                    .unwrap();
                let p = raw_pixels(f);
                assert!(p == oracle[..p.len()]);
                assert_eq!(reader.hdr().light.max_cll, 100.0);
                let (ticks, scale) = reader.current_pts().unwrap();
                assert_eq!(
                    u128::try_from(ticks).unwrap() * 25,
                    u128::from(index) * u128::from(scale)
                );
            }
        }
    }
}
