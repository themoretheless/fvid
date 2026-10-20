#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod audio_capture;
use fvid::audio::{AudioDecode, AudioStream};
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp3-decoder")
}
fn decode(stream: &mut dyn AudioStream, decoder: &mut dyn AudioDecode) -> Vec<(Duration, Vec<u8>)> {
    let mut result = Vec::new();
    while let Some(source) = stream.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&source.data, source.pts, source.duration.max(0) as u64)
            .unwrap()
        {
            if let Some(packet) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                result.push((packet.presentation_time(), packet.data));
            }
        }
    }
    stream.validate_eof().unwrap();
    result
}
fn flattened(packets: &[(Duration, Vec<u8>)]) -> Vec<u8> {
    packets
        .iter()
        .flat_map(|(_, bytes)| bytes.iter().copied())
        .collect()
}
fn check(mut stream: Box<dyn AudioStream>, expected: Option<Vec<u8>>, sample_clock: bool) {
    let rate = stream.sample_rate();
    let stride = usize::from(stream.channels()) * 4;
    let mut decoder = stream.make_decoder().unwrap();
    let packets = decode(stream.as_mut(), decoder.as_mut());
    let baseline = flattened(&packets);
    assert!(baseline.iter().any(|&v| v != 0));
    if let Some(expected) = expected {
        assert_eq!(baseline, expected);
    }
    let frames = baseline.len() / stride;
    let mut sample = 0;
    for (at, bytes) in &packets {
        assert!(
            at.as_nanos()
                .abs_diff(sample as u128 * 1_000_000_000 / u128::from(rate))
                <= 1
        );
        sample += bytes.len() / stride;
    }
    // Nonmonotonic navigation must produce identical sample suffixes.
    for target in [
        frames + 37,
        1,
        frames / 2,
        0,
        1151,
        1152,
        1153,
        frames - 1,
        frames,
    ] {
        let target = target.min(frames + 37);
        let ticks = if sample_clock {
            target as i64
        } else {
            (target as u128 * 1_000_000_000 / u128::from(rate)) as i64
        };
        let landed = stream.seek_to(ticks);
        decoder.reset();
        if sample_clock {
            assert_eq!(landed, target.min(frames) as i64);
        } else {
            assert_eq!(landed, ticks);
        }
        let actual = decode(stream.as_mut(), decoder.as_mut());
        assert_eq!(
            flattened(&actual),
            baseline[target.min(frames) * stride..],
            "rate {rate}, sample {target}, raw {sample_clock}"
        );
        if let Some((at, _)) = actual.first() {
            assert!(
                at.as_nanos()
                    .abs_diff(target as u128 * 1_000_000_000 / u128::from(rate))
                    <= 1
            );
        }
    }
    stream.rewind();
    decoder.reset();
    assert_eq!(decode(stream.as_mut(), decoder.as_mut()), packets);
    stream.rewind();
    let seek = packets[1].0;
    if !sample_clock || seek.as_nanos() * u128::from(rate) % 1_000_000_000 == 0 {
        audio_capture::worker_check_with_seek(stream, &packets, Some(seek));
    } else {
        audio_capture::worker_check(stream, &packets);
    }
}
#[test]
fn owned_factory_raw_mp3_exact_seek_eof_rewind_and_worker() {
    for rate in [48000, 24000, 8000, 44100, 22050, 11025, 32000, 16000, 12000] {
        for channels in [1, 2] {
            let name = format!("{rate}-{channels}ch");
            let bytes = std::fs::read(root().join(format!("{name}.mp3"))).unwrap();
            let expected = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
            let stream =
                fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default())
                    .unwrap();
            check(Box::new(stream), Some(expected), true);
        }
    }
    let bytes = include_bytes!("fixtures/mp3/tone.mp3");
    check(
        Box::new(
            fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap(),
        ),
        None,
        true,
    );
}
#[test]
fn owned_factory_matroska_mp3_exact_seek_quantized_clocks_and_worker() {
    for rate in [48000, 24000, 8000, 44100, 22050, 11025, 32000, 16000, 12000] {
        for channels in [1, 2] {
            let name = format!("{rate}-{channels}ch");
            let bytes = std::fs::read(root().join(format!("{name}.mkv"))).unwrap();
            let expected = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
            let stream = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            check(Box::new(stream), Some(expected), false);
        }
    }
}

#[test]
fn cold_packet_decode_reproduces_the_overlap_error_that_seek_preroll_fixes() {
    let bytes = std::fs::read(root().join("48000-1ch.mp3")).unwrap();
    let expected = std::fs::read(root().join("48000-1ch.f32le")).unwrap();
    let mut stream =
        fvid::playback_mp3::Mp3AudioReader::open(bytes.as_slice(), Default::default()).unwrap();
    let mut packets = Vec::new();
    while let Some(packet) = stream.next_packet().unwrap() {
        packets.push(packet);
    }
    let source = &packets[2];
    let mut cold = fvid::codec::mp3_decoder::Mp3Decoder::new(48000, 1).unwrap();
    let wrong = cold
        .decode_packet(&source.data, source.pts, source.duration as u64)
        .unwrap()
        .unwrap();
    let start = source.pts as usize * 4;
    assert_ne!(
        wrong.packet.data,
        expected[start..start + wrong.packet.data.len()],
        "fixture must specifically expose lost synthesis overlap after a cold jump"
    );
    stream.seek_to(source.pts);
    let mut recovered = stream.make_decoder().unwrap();
    assert_eq!(
        flattened(&decode(&mut stream, recovered.as_mut())),
        expected[start..]
    );
}
