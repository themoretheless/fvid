use fvid::{
    codec::{
        avc::{Pps, Sps},
        avc_access_unit,
        avc_decoder::AvcDecoder,
        config::AvcConfig,
    },
    container::mp4::Mp4Reader,
};
use std::io::Cursor;
const BASE: &[u8] = include_bytes!("fixtures/playback-errors/avc-multislice-ipb.mp4");
fn cases() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/avc-end-markers.json"
    ))
    .unwrap()
}
fn video(c: &serde_json::Value) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["file"].as_str().unwrap()),
    )
    .unwrap()
}
#[test]
fn end_marker_videos_preserve_all_vcl_slices_references_and_decoded_pictures() {
    for c in cases()["cases"].as_array().unwrap() {
        let data = video(c);
        let mut marked = Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let mut baseline = Mp4Reader::open(Cursor::new(BASE), Default::default()).unwrap();
        let config = marked.tracks()[0].configuration.clone();
        let avc = AvcConfig::parse(&config).unwrap();
        let sps = Sps::parse(avc.sps[0]).unwrap();
        let pps = Pps::parse(avc.pps[0], &sps).unwrap();
        let mut d = AvcDecoder::new(&config, 16 << 20).unwrap();
        let mut reference = AvcDecoder::new(&baseline.tracks()[0].configuration, 16 << 20).unwrap();
        for i in 0..marked.tracks()[0].samples.len() {
            let mut a = vec![];
            let mut b = vec![];
            marked.read_packet(0, i, &mut a).unwrap();
            baseline.read_packet(0, i, &mut b).unwrap();
            let prepared =
                avc_access_unit::prepare(&a, avc.length_size, &sps, &pps, a.len()).unwrap();
            assert_eq!(prepared.len(), 2);
            let picture = d.decode_order(&a).unwrap().unwrap();
            let expected = reference.decode_order(&b).unwrap().unwrap();
            assert_eq!(picture.dimensions(), expected.dimensions());
            assert_eq!(picture.y, expected.y);
            assert_eq!(picture.cb, expected.cb);
            assert_eq!(picture.cr, expected.cr);
        }
        // Marker-only packets carry no VCL picture. Caller-owned output ordering
        // remains the caller's responsibility; the decoder must not invent output.
        for kind in [10, 11] {
            let mut packet = (2u32).to_be_bytes()[4 - avc.length_size as usize..].to_vec();
            packet.extend_from_slice(&[kind, 128]);
            assert!(
                avc_access_unit::prepare(&packet, avc.length_size, &sps, &pps, packet.len())
                    .unwrap()
                    .is_empty()
            );
            assert!(d.decode_order(&packet).unwrap().is_none());
        }
        d.reset();
        let mut first = vec![];
        marked.read_packet(0, 0, &mut first).unwrap();
        assert!(d.decode_order(&first).unwrap().is_some());
    }
}
#[test]
fn marker_video_playback_drains_bframes_and_matches_saved_yuv_after_rewind_and_seek() {
    let gold = include_bytes!("fixtures/playback-errors/avc-multislice-ipb.yuv");
    for c in cases()["cases"].as_array().unwrap() {
        let data = video(c);
        let mut player = fvid::playback_mp4::Mp4VideoReader::open_software(
            Cursor::new(&data),
            Default::default(),
            16 << 20,
        )
        .unwrap();
        for pass in 0..3 {
            let mut out = vec![];
            let mut count = 0;
            while let Some(frame) = player.read_frame().unwrap() {
                frame.picture.write_planar(&mut out).unwrap();
                count += 1;
            }
            assert_eq!(count, 8);
            assert_eq!(out, gold);
            if pass == 0 {
                player.rewind();
            } else if pass == 1 {
                assert_eq!(player.seek_to_sync(4), 0);
            }
        }
    }
}
