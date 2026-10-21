#![cfg(feature = "player")]
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::{AudioDecode, AudioStep, AudioStream},
    codec::eac3_decoder::{Eac3Decoder, frame},
};
use std::{io::Cursor, path::Path};
fn read(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/eac3-spx-noise")
            .join(name),
    )
    .unwrap()
}
fn drain(stream: &mut dyn AudioStream, decoder: &mut Box<dyn AudioDecode>) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(step) = stream.next_step().unwrap() {
        match step {
            AudioStep::Encoded(p) => {
                if let Some(d) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    if let Some(p) = stream.present_decoded(d.packet, d.source_pts).unwrap() {
                        out.extend(p.data);
                    }
                }
            }
            AudioStep::Pcm(p) => out.extend(p.data),
            AudioStep::ResetDecoder => stream.reset_decoder(decoder).unwrap(),
        }
    }
    out.extend(decoder.drain_presented(stream).into_iter().flat_map(|p| p.data));
    stream.validate_eof().unwrap();
    out
}
fn cases() -> Vec<serde_json::Value> {
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    let cases = m["cases"].as_array().unwrap().clone();
    assert_eq!(cases.len(), 15);
    cases
}
fn open(c: &serde_json::Value, container: bool) -> Box<dyn AudioStream> {
    let bytes = Cursor::new(read(
        c[if container { "video" } else { "file" }]
            .as_str()
            .unwrap(),
    ));
    if container {
        Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default()).unwrap(),
        )
    } else {
        Box::new(fvid::playback_ac3::Ac3AudioReader::open(bytes, Default::default()).unwrap())
    }
}
fn direct_reset(noisy: bool) {
    for c in cases()
        .iter()
        .filter(|c| c["noise"].as_bool() == Some(noisy))
    {
        let raw = read(c["file"].as_str().unwrap());
        let h = frame(&raw).unwrap();
        assert_eq!((h.blocks, h.channels, h.frame_bytes), (6, 2, 1024));
        let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let first = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert_eq!(first.len(), (3 * 1536 - 256) * 8);
        assert!(
            first
                .chunks_exact(4)
                .any(|x| f32::from_le_bytes(x.try_into().unwrap()).abs() > 1e-4)
        );
        d.reset();
        let replay = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        let error = first
            .chunks_exact(4)
            .zip(replay.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0.0f32, f32::max);
        assert!(
            first == replay,
            "{} reset must match a fresh decode, maximum error {error}",
            c["name"]
        );
    }
}
#[test]
fn signal_only_spx_reset_matches_fresh_decoder() {
    direct_reset(false);
}
#[test]
fn noisy_spx_reset_matches_fresh_decoder() {
    direct_reset(true);
}
#[test]
fn spx_noise_reader_rewind_preserves_pcm() {
    for c in cases() {
        for container in [false, true] {
            let mut stream = open(&c, container);
            let mut d = stream.make_decoder().unwrap();
            let expected = drain(&mut *stream, &mut d);
            let duration = stream.duration();
            stream.rewind();
            d.reset();
            let replay = drain(&mut *stream, &mut d);
            assert!(
                replay == expected,
                "{} reader container={container}: PCM changed on rewind",
                c["name"]
            );
            assert_eq!(stream.duration(), duration);
        }
    }
}

#[test]
fn stationary_signal_repeats_but_spx_noise_advances_between_frames() {
    for c in cases() {
        let raw = read(c["file"].as_str().unwrap());
        let mut d = Eac3Decoder::new(&[], 48000, 2).unwrap();
        let pcm = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        let second = &pcm[(1536 - 256) * 8..(3072 - 256) * 8];
        let third = &pcm[(3072 - 256) * 8..];
        assert_eq!(second.len(), third.len());
        assert_eq!(
            second == third,
            !c["noise"].as_bool().unwrap(),
            "{}: noise history must advance between frames, signal-only control must remain stationary",
            c["name"]
        );
    }
}

#[path = "support/audio_capture.rs"]
mod audio_capture;
fn packets(stream: &mut dyn AudioStream) -> Vec<(std::time::Duration, Vec<u8>)> {
    let mut d = stream.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(step) = stream.next_step().unwrap() {
        let packet = match step {
            AudioStep::Encoded(p) => d
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap()
                .and_then(|p| stream.present_decoded(p.packet, p.source_pts).unwrap()),
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ResetDecoder => {
                stream.reset_decoder(&mut d).unwrap();
                None
            }
        };
        if let Some(p) = packet {
            out.push((p.presentation_time(), p.data));
        }
    }
    out.extend(d.drain_presented(stream).into_iter().map(|p| (p.presentation_time(), p.data)));
    stream.validate_eof().unwrap();
    out
}
#[test]
fn audio_worker_preserves_spx_noise_on_rewind_and_partial_seek() {
    use std::time::Duration;
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap().iter() {
        for container in [false, true] {
            let expected = packets(&mut *open(c, container));
            assert_eq!(expected.len(), 3);
            let seek = Duration::from_millis(20);
            assert_eq!(expected[0].0, Duration::ZERO);
            let mut wanted = expected.clone();
            // 20 ms is inside the first decoded packet on both reader paths.
            wanted[0].0 = seek;
            wanted[0].1.drain(..960 * 8);
            audio_capture::worker_check_with_layout_and_seek_expected(
                open(c, container),
                &expected,
                seek,
                &wanted,
                3,
            );
        }
    }
}
