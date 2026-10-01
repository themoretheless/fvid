use fvid::{codec::aac_native::NativeAacDecoder, container::adts::StreamReader};
use std::io::Cursor;
#[test]
fn independent_coupling_matches_saved_pcm_reset_and_checkpoint() {
    for (data, expected, channels) in [
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling.aac").as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling.f32le").as_slice(),
            1,
        ),
        (
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo.aac")
                .as_slice(),
            include_bytes!("fixtures/playback-errors/aac-independent-coupling-stereo.f32le")
                .as_slice(),
            2,
        ),
    ] {
        let mut reader = StreamReader::open(Cursor::new(data)).unwrap();
        let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
        assert_eq!(usize::from(decoder.channels()), channels);
        let mut packets = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            packets.push(p);
        }
        assert_eq!(packets.len(), 6);
        let mut actual = Vec::new();
        let mut state = None;
        for (i, p) in packets.iter().enumerate() {
            actual.extend(decoder.decode(p).unwrap());
            if i == 2 {
                state = Some(decoder.checkpoint());
            }
        }
        assert_eq!(actual.len() * 4, expected.len());
        assert!(
            actual.iter().any(|v| v.abs() > 0.00001),
            "silent target must receive nonzero coupling PCM"
        );
        let mut peak = 0f32;
        for (sample, bytes) in actual.iter().zip(expected.chunks_exact(4)) {
            peak = peak.max((sample - f32::from_le_bytes(bytes.try_into().unwrap())).abs());
        }
        assert!(
            peak < 0.0000001,
            "independent coupling PCM oracle mismatch: {peak}"
        );
        decoder.reset();
        decoder.restore(&state.unwrap()).unwrap();
        let tail: Vec<_> = packets[3..]
            .iter()
            .flat_map(|p| decoder.decode(p).unwrap())
            .collect();
        assert_eq!(tail, actual[3 * 1024 * channels..]);
        decoder.reset();
        let replay: Vec<_> = packets
            .iter()
            .flat_map(|p| decoder.decode(p).unwrap())
            .collect();
        assert_eq!(replay, actual);
    }
}

#[test]
fn absent_coupling_target_does_not_mutate_overlap_or_noise() {
    let good = include_bytes!("fixtures/playback-errors/aac-independent-coupling.aac");
    let bad =
        include_bytes!("fixtures/playback-errors/aac-independent-coupling-missing-target.aac");
    let mut reader = StreamReader::open(Cursor::new(good)).unwrap();
    let mut decoder = NativeAacDecoder::new(reader.audio_specific_config()).unwrap();
    let first = reader.next_packet().unwrap().unwrap();
    let second = reader.next_packet().unwrap().unwrap();
    decoder.decode(&first).unwrap();
    let state = decoder.checkpoint();
    let expected = decoder.decode(&second).unwrap();
    decoder.restore(&state).unwrap();
    let mut invalid = StreamReader::open(Cursor::new(bad)).unwrap();
    let packet = invalid.next_packet().unwrap().unwrap();
    assert!(
        decoder
            .decode(&packet)
            .unwrap_err()
            .to_string()
            .contains("coupling target is absent")
    );
    assert_eq!(decoder.decode(&second).unwrap(), expected);
}
