use fvid::{
    codec::aac_native::NativeAacDecoder,
    container::adts::{Aac, Limits},
};
const BASE: &[u8] = include_bytes!("fixtures/playback-errors/aac-independent-coupling.aac");
#[test]
fn ancillary_fill_preserves_nonzero_pcm_and_explicit_pce_configuration() {
    let baseline = Aac::parse(BASE, &Limits::default()).unwrap();
    for bytes in [
        include_bytes!("fixtures/playback-errors/aac-fill-empty.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-fill-short.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-fill-escaped.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-fill-multiple.aac").as_slice(),
        include_bytes!("fixtures/playback-errors/aac-fill-version.aac").as_slice(),
    ] {
        let stream = Aac::parse(bytes, &Limits::default()).unwrap();
        assert_eq!(stream.configuration, baseline.configuration);
        assert_eq!(stream.packets(), baseline.packets());
        let mut expected = NativeAacDecoder::new(&baseline.configuration).unwrap();
        let mut actual = NativeAacDecoder::new(&stream.configuration).unwrap();
        let mut nonzero = false;
        for index in 0..stream.packets() {
            let pcm = expected.decode(baseline.packet(index)).unwrap();
            nonzero |= pcm.iter().any(|v| v.abs() > 0.00001);
            assert_eq!(actual.decode(stream.packet(index)).unwrap(), pcm);
        }
        assert!(nonzero);
    }
}

#[test]
fn malformed_ancillary_and_hidden_audio_tools_leave_decoder_state_unchanged() {
    let baseline = Aac::parse(BASE, &Limits::default()).unwrap();
    for (bytes, message) in [
        (
            include_bytes!("fixtures/playback-errors/aac-fill-overrun.aac").as_slice(),
            "exceeds fill payload",
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-fill-unterminated.aac").as_slice(),
            "exceeds fill payload",
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-fill-hidden-sbr.aac").as_slice(),
            "fill extension tool",
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-fill-bad-fill.aac").as_slice(),
            "fill-data byte",
        ),
    ] {
        assert!(
            Aac::parse(bytes, &Limits::default())
                .err()
                .unwrap()
                .to_string()
                .contains(message)
        );
        let size =
            ((bytes[3] as usize & 3) << 11) | ((bytes[4] as usize) << 3) | (bytes[5] as usize >> 5);
        let mut decoder = NativeAacDecoder::new(&baseline.configuration).unwrap();
        let mut untouched = NativeAacDecoder::new(&baseline.configuration).unwrap();
        decoder.decode(baseline.packet(0)).unwrap();
        untouched.decode(baseline.packet(0)).unwrap();
        assert!(
            decoder
                .decode(&bytes[7..size])
                .unwrap_err()
                .to_string()
                .contains(message)
        );
        assert_eq!(
            decoder.decode(baseline.packet(1)).unwrap(),
            untouched.decode(baseline.packet(1)).unwrap()
        );
    }
}

#[test]
fn every_truncated_escaped_fill_packet_is_rejected_without_advancing_audio() {
    let baseline = Aac::parse(BASE, &Limits::default()).unwrap();
    let stream = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-fill-escaped.aac"),
        &Limits::default(),
    )
    .unwrap();
    let packet = stream.packet(0);
    for length in 0..packet.len() {
        let mut decoder = NativeAacDecoder::new(&baseline.configuration).unwrap();
        let mut untouched = NativeAacDecoder::new(&baseline.configuration).unwrap();
        decoder.decode(baseline.packet(0)).unwrap();
        untouched.decode(baseline.packet(0)).unwrap();
        assert!(
            decoder.decode(&packet[..length]).is_err(),
            "length {length}"
        );
        assert_eq!(
            decoder.decode(baseline.packet(1)).unwrap(),
            untouched.decode(baseline.packet(1)).unwrap(),
            "length {length}"
        );
    }
}

#[test]
fn synthetic_video_companion_matches_the_six_aac_frame_intervals() {
    let input = include_bytes!("fixtures/playback-errors/aac-fill-companion.y4m");
    let mut output = Vec::new();
    let stats = fvid::y4m::process(
        std::io::Cursor::new(input),
        &mut output,
        Default::default(),
        1 << 20,
    )
    .unwrap();
    let stream = Aac::parse(
        include_bytes!("fixtures/playback-errors/aac-fill-short.aac"),
        &Limits::default(),
    )
    .unwrap();
    assert_eq!(stats.frames as usize, stream.packets());
    assert_eq!(stats.frames, 6);
    assert_eq!(output, input);
}
