use fvid::{codec::hevc_decoder::HevcDecoder, container::mp4::Mp4Reader};
use std::io::Cursor;

#[test]
fn independent_multislice_picture_headers_have_ordered_shared_identity() {
    let mut input = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let track = input.tracks()[0].clone();
    let decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    assert_eq!(track.samples.len(), 3);
    let mut packet = Vec::new();
    for index in 0..track.samples.len() {
        input.read_packet(0, index, &mut packet).unwrap();
        let headers = decoder.slice_headers(&packet).unwrap();
        assert_eq!(headers.len(), 2);
        assert!(headers[0].first);
        assert!(!headers[1].first);
        assert_eq!((headers[0].address, headers[1].address), (0, 8));
        assert_eq!(headers[0].poc_lsb, headers[1].poc_lsb);
        // Appending a second copy starts a new picture inside this access unit.
        let mut duplicate = packet.clone();
        duplicate.extend_from_slice(&packet);
        assert!(decoder.slice_headers(&duplicate).is_err());
        for cut in 1..packet.len() {
            // A prefix ending exactly at a NAL boundary may be one valid slice;
            // every other truncation must be a checked error, never a panic.
            let _ = decoder.slice_headers(&packet[..cut]);
        }
    }
    assert_eq!(
        include_bytes!("fixtures/playback-errors/hevc-multislice-main.yuv").len(),
        3 * 128 * 128 * 3 / 2
    );
}

#[test]
fn multislice_reconstruction_limit_is_reproduced_without_an_unrelated_error() {
    let mut input = Mp4Reader::open(
        Cursor::new(include_bytes!(
            "fixtures/playback-errors/hevc-multislice-main.mp4"
        )),
        Default::default(),
    )
    .unwrap();
    let track = input.tracks()[0].clone();
    let mut decoder = HevcDecoder::from_configuration(&track.configuration, 16 << 20).unwrap();
    let mut packet = Vec::new();
    input.read_packet(0, 0, &mut packet).unwrap();
    let error = match decoder.decode_packet(&packet) {
        Err(error) => error,
        Ok(_) => panic!("update refusal regression when multi-slice reconstruction is implemented"),
    };
    assert!(
        error.to_string().contains("multi-slice access units"),
        "{error}"
    );
}
