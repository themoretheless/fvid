use fvid::{codec::{config::HevcConfig, hevc_decoder::HevcDecoder, hevc_pps::Pps, hevc_sps::Sps}, container::mp4::Mp4Reader};
use std::io::Cursor;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors").join(name)).unwrap()
}

fn decode(name: &str) -> Vec<(i32, Vec<u16>)> {
    let mut movie = Mp4Reader::open(Cursor::new(fixture(name)), Default::default()).unwrap();
    let mut decoder = HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20).unwrap();
    let mut frames = Vec::new();
    let mut packet = Vec::new();
    for index in 0..movie.tracks()[0].samples.len() {
        movie.read_packet(0, index, &mut packet).unwrap();
        let frame = decoder.decode_packet(&packet).unwrap_or_else(|e| panic!("{name} packet {index}: {e}")).unwrap();
        frames.push((frame.poc, frame.picture.planes[0].samples().to_vec()));
    }
    frames
}

#[test]
fn owned_wrap_baseline_decodes_all_references() {
    let baseline = decode("hevc-poc-baseline-synthetic.mp4");
    assert_eq!(baseline.len(), 35);
    assert_eq!(baseline.iter().map(|f| f.0).collect::<Vec<_>>(), (0..35).collect::<Vec<_>>());
    let original = decode("hevc-pcm-mono-reference-rext8.mp4");
    assert_eq!(&baseline[..3], original.as_slice());
    let gold = fixture("hevc-pcm-mono-reference-rext8.yuv");
    assert_eq!(gold.len(), 3 * 64 * 64);
    for (frame, bytes) in baseline.iter().take(3).zip(gold.chunks_exact(64 * 64)) {
        assert_eq!(frame.1, bytes.iter().copied().map(u16::from).collect::<Vec<_>>());
    }
}

#[test]
fn reset_fixture_pps_flag_is_admitted() {
    for kind in ["msb-reset", "msb-reset-signalled"] {
        let bytes = fixture(&format!("hevc-poc-{kind}-synthetic.mp4"));
        let movie = Mp4Reader::open(Cursor::new(bytes), Default::default()).unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        let sps = Sps::parse(config.arrays.iter().find(|a| a.nal_type == 33).unwrap().units[0], 32 << 20).unwrap();
        let pps = Pps::parse(config.arrays.iter().find(|a| a.nal_type == 34).unwrap().units[0], &sps, 32 << 20).unwrap();
        assert!(pps.poc_reset_info_present && pps.slice_header_extension);
    }
}

#[test]
fn reset_video_preserves_pixels_and_translates_poc_once() {
    let baseline = decode("hevc-poc-baseline-synthetic.mp4");
    for kind in ["msb-reset", "msb-reset-signalled"] {
        let actual = decode(&format!("hevc-poc-{kind}-synthetic.mp4"));
        assert_eq!(actual.len(), baseline.len());
        for (index, ((poc, pixels), (_, expected))) in actual.iter().zip(&baseline).enumerate() {
            assert_eq!(pixels, expected, "{kind} picture {index}");
            let decrement = if index >= 33 { 32 } else if index >= 17 { 16 } else { 0 };
            assert_eq!(*poc, index as i32 - decrement, "{kind} picture {index}");
        }
    }
}

#[test]
fn reset_video_software_seek_and_rewind_match_baseline() {
    use fvid::playback_mp4::Mp4VideoReader;
    let open = |name: &str| Mp4VideoReader::open_software(Cursor::new(fixture(name)), Default::default(), 32 << 20).unwrap();
    for kind in ["msb-reset", "msb-reset-signalled"] {
        let mut baseline = open("hevc-poc-baseline-synthetic.mp4");
        let mut actual = open(&format!("hevc-poc-{kind}-synthetic.mp4"));
        assert!(!actual.hardware_accelerated());
        for target in [None, Some(18), Some(34), None] {
            if let Some(index) = target {
                assert_eq!(actual.seek_to_sync(index), baseline.seek_to_sync(index));
            } else { actual.rewind(); baseline.rewind(); }
            let mut count = 0;
            loop {
                match (actual.read_frame().unwrap(), baseline.read_frame().unwrap()) {
                    (Some(a), Some(b)) => {
                        assert_eq!(a.sample_index, b.sample_index);
                        assert_eq!(a.presentation_time, b.presentation_time);
                        assert_eq!(a.duration, b.duration);
                        assert_eq!(a.picture.y, b.picture.y);
                        assert_eq!(a.picture.cb, b.picture.cb);
                        assert_eq!(a.picture.cr, b.picture.cr);
                        count += 1;
                    }
                    (None, None) => break,
                    _ => panic!("reset playback frame count differs"),
                }
            }
            assert_eq!(count, 35, "fixture has one sync picture; seek prerolls across both resets");
        }
    }
}
