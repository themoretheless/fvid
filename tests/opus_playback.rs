#![cfg(feature = "player")]
use fvid::{audio::AudioStream, container::webm::Limits, playback_webm_audio::WebmAudioReader};
use std::io::Cursor;

fn decode(bytes: &[u8], seek: Option<i64>) -> (Vec<f32>, Vec<u64>, u16) {
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    assert_eq!(stream.sample_rate(), 48000);
    let channels = stream.channels();
    let mut decoder =
        fvid::codec::make_audio_decoder(stream.codec(), stream.extra_data(), 48000, channels, 0)
            .unwrap();
    if let Some(target) = seek {
        assert_eq!(stream.seek_to(target), target);
        decoder.reset();
    }
    let mut output = Vec::new();
    let mut stamps = Vec::new();
    let mut end = 0u128;
    while let Some(packet) = stream.next_packet().unwrap() {
        let pcm = decoder
            .decode_encoded(
                &packet.data,
                packet.pts.max(0) as u64,
                packet.duration as u64,
            )
            .unwrap()
            .unwrap();
        if let Some(pcm) = stream.present_decoded(pcm, packet.pts).unwrap() {
            stamps.push(pcm.pts);
            end = u128::from(pcm.pts)
                + (pcm.data.len() / (usize::from(channels) * 4)) as u128 * 1_000_000_000 / 48000;
            output.extend(
                pcm.data
                    .chunks_exact(4)
                    .map(|s| f32::from_le_bytes(s.try_into().unwrap())),
            );
        }
    }
    assert!(stamps.windows(2).all(|s| s[0] < s[1]));
    assert!(stream.duration().unwrap().as_nanos().abs_diff(end) <= 1);
    (output, stamps, channels)
}

fn check(bytes: &[u8], reference: &[u8], channels: u16, mode: u8, origin: u64) {
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    while let Some(packet) = stream.next_packet().unwrap() {
        let config = packet.data[0] >> 3;
        let found = if config < 12 {
            0
        } else if config < 16 {
            1
        } else {
            2
        };
        assert_eq!(found, mode, "fixture does not exercise its named Opus mode");
    }
    let (pcm, stamps, found) = decode(bytes, None);
    assert_eq!(found, channels);
    assert_eq!(pcm.len(), 48000 * usize::from(channels));
    assert_eq!(stamps[0], origin);
    assert_eq!(reference.len(), pcm.len() * 4);
    let error = pcm
        .iter()
        .zip(reference.chunks_exact(4))
        .map(|(&a, b)| (a - f32::from_le_bytes(b.try_into().unwrap())).abs())
        .fold(0.0f32, f32::max);
    assert!(
        error < 0.00002,
        "Opus PCM differs from libopus: max_error={error}"
    );
    let (again, _, _) = decode(bytes, None);
    assert_eq!(again, pcm);
    for target in [origin as i64, 500_000_000, 999_000_000] {
        let (tail, stamps, _) = decode(bytes, Some(target));
        assert!(!tail.is_empty());
        assert!(stamps[0] >= target as u64 && stamps[0] - (target as u64) < 21_000);
        // Matroska timestamps are quantized to milliseconds. Seeking uses those
        // stamps, so choose the suffix by the exact emitted sample count.
        assert_eq!(tail, pcm[pcm.len() - tail.len()..]);
    }
}

#[test]
fn mono_celt_matches_libopus_and_trims_priming_and_tail() {
    check(
        include_bytes!("fixtures/playback-errors/opus-mono.webm"),
        include_bytes!("fixtures/playback-errors/opus-mono.f32"),
        1,
        2,
        0,
    );
}
#[test]
fn stereo_celt_preserves_both_channels() {
    check(
        include_bytes!("fixtures/playback-errors/opus-stereo.webm"),
        include_bytes!("fixtures/playback-errors/opus-stereo.f32"),
        2,
        2,
        0,
    );
}
#[test]
fn silk_speech_mode_matches_libopus() {
    check(
        include_bytes!("fixtures/playback-errors/opus-silk.webm"),
        include_bytes!("fixtures/playback-errors/opus-silk.f32"),
        1,
        0,
        0,
    );
}
#[test]
fn hybrid_silk_celt_matches_libopus() {
    check(
        include_bytes!("fixtures/playback-errors/opus-hybrid.webm"),
        include_bytes!("fixtures/playback-errors/opus-hybrid.f32"),
        1,
        1,
        0,
    );
}

#[test]
fn surround_51_matches_libopus_in_wave_channel_order() {
    check(
        include_bytes!("fixtures/playback-errors/opus-surround.webm"),
        include_bytes!("fixtures/playback-errors/opus-surround.f32"),
        6,
        2,
        0,
    );
}

#[test]
fn invalid_headers_and_packets_are_reported_and_reset_restores_pcm() {
    use fvid::audio::AudioDecode;
    let bytes = include_bytes!("fixtures/playback-errors/opus-mono.webm");
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    let mut head = stream.extra_data().to_vec();
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head[..18], 48000, 1).is_err());
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head, 44100, 1).is_err());
    assert!(fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 2).is_err());
    let mut decoder = fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 1).unwrap();
    let packet = stream.next_packet().unwrap().unwrap();
    let first = decoder
        .decode_encoded(&packet.data, 0, 0)
        .unwrap()
        .unwrap()
        .data;
    assert!(decoder.decode_encoded(&[], 0, 0).is_err());
    assert!(decoder.decode_encoded(&[3], 0, 0).is_err());
    decoder.reset();
    assert_eq!(
        decoder
            .decode_encoded(&packet.data, 0, 0)
            .unwrap()
            .unwrap()
            .data,
        first
    );
    head[16..18].copy_from_slice(&1536i16.to_le_bytes());
    let mut louder = fvid::codec::opus_decoder::OpusDecoder::new(&head, 48000, 1).unwrap();
    let scaled = louder
        .decode_encoded(&packet.data, 0, 0)
        .unwrap()
        .unwrap()
        .data;
    for (a, b) in first.chunks_exact(4).zip(scaled.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((b - a * 10f32.powf(6.0 / 20.0)).abs() < 0.000001);
    }
}

#[test]
fn positive_first_packet_still_discards_encoder_priming() {
    let bytes = include_bytes!("fixtures/playback-errors/opus-positive-first.webm");
    let mut stream = WebmAudioReader::open(Cursor::new(bytes), Limits::default()).unwrap();
    let first = stream.read_packet().unwrap().unwrap();
    assert!(
        first.pts_ns > 0,
        "fixture must reproduce the positive first packet"
    );
    check(
        bytes,
        include_bytes!("fixtures/playback-errors/opus-positive-first.f32"),
        1,
        2,
        1_500_000,
    );
}
