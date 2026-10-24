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
            .join("tests/fixtures/playback-errors/eac3-coupling-updates")
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
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
            AudioStep::ResetDecoder => stream.reset_decoder(decoder).unwrap(),
        }
    }
    out.extend(decoder.drain_presented(stream).into_iter().flat_map(|p| p.data));
    stream.validate_eof().unwrap();
    out
}
fn assert_pcm(got: &[u8], expected: &[u8], case: &serde_json::Value, context: &str) {
    assert_eq!(got.len(), expected.len(), "{} {context}", case["name"]);
    if case["exact"].as_bool() == Some(true) {
        assert!(got == expected, "{} exact {context}", case["name"]);
        return;
    }
    let mut max = 0.0f32;
    for (a, b) in got.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!(a.is_finite() && b.is_finite());
        max = max.max((a - b).abs());
    }
    let bound = case["quantization_bound"].as_f64().unwrap() + 1e-7;
    assert!(
        f64::from(max) <= bound,
        "{} {context}: max {max} bound {bound}",
        case["name"]
    );
}
fn speaker_mask(c: &serde_json::Value) -> u32 {
    let base = match c["layout"].as_u64().unwrap() {
        2 => 3,
        3 => 7,
        4 => 0x103,
        5 => 0x107,
        6 => 0x603,
        7 => 0x607,
        _ => unreachable!(),
    };
    base | if c["lfe"].as_bool().unwrap() { 8 } else { 0 }
}
#[test]
fn coupling_updates_participants_and_modes_preserve_pcm_and_clocks() {
    let manifest: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    assert_eq!(manifest["cases"].as_array().unwrap().len(), 448);
    let cases: Vec<_> = manifest["cases"].as_array().unwrap().iter().collect();

    for c in cases {
        let blocks = c["blocks"].as_u64().unwrap() as usize;
        let raw = read(c["file"].as_str().unwrap());
        let h = frame(&raw).unwrap_or_else(|| panic!("{}: AHT header refused", c["name"]));
        assert_eq!(
            (h.sample_rate, h.channels, h.blocks, h.frame_bytes),
            (48000, c["channels"].as_u64().unwrap() as u16, blocks, 4096)
        );
        assert!(h.coupling[..blocks].iter().all(|v| *v));
        let control = read(c["control"].as_str().unwrap());
        let mut reference =
            Eac3Decoder::new(&[], 48000, c["channels"].as_u64().unwrap() as u16).unwrap();
        let expected = reference
            .decode_complete(&control, 0, 0)
            .unwrap()
            .unwrap()
            .data;
        let mut decoder =
            Eac3Decoder::new(&[], 48000, c["channels"].as_u64().unwrap() as u16).unwrap();
        let got = decoder.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        assert_eq!(
            expected.len(),
            (3 * blocks * 256 - 256) * 4 * c["channels"].as_u64().unwrap() as usize
        );
        assert!(
            expected
                .chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.00001)
        );
        assert_pcm(&got, &expected, c, "raw PCM");
        for container in [false, true] {
            let open = |converted: bool| -> Box<dyn AudioStream> {
                let key = match (container, converted) {
                    (false, false) => "control",
                    (false, true) => "file",
                    (true, false) => "control_video",
                    (true, true) => "video",
                };
                let bytes = Cursor::new(read(c[key].as_str().unwrap()));
                if container {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_ac3::Ac3AudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                }
            };
            let mut control = open(false);
            let mut d = control.make_decoder().unwrap();
            let expected = drain(&mut *control, &mut d);
            let mut stream = open(true);
            assert_eq!(stream.channels(), c["channels"].as_u64().unwrap() as u16);
            assert_eq!(stream.speaker_mask().unwrap(), Some(speaker_mask(c)));
            let mut d = stream.make_decoder().unwrap();
            assert_pcm(
                &drain(&mut *stream, &mut d),
                &expected,
                c,
                &format!("container={container}"),
            );
            assert_eq!(stream.duration(), control.duration());
            stream.rewind();
            d.reset();
            assert_pcm(
                &drain(&mut *stream, &mut d),
                &expected,
                c,
                &format!("rewind container={container}"),
            );
        }
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
            AudioStep::ReconfigureDecoder => panic!("unexpected configuration change in constant-setup fixture"),
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
fn audio_worker_preserves_coupling_updates_on_rewind_and_partial_seek() {
    use std::time::Duration;
    let m: serde_json::Value = serde_json::from_slice(&read("manifest.json")).unwrap();
    for c in m["cases"].as_array().unwrap().iter().filter(|c| {
        !c["explicit"].as_bool().unwrap()
            && (matches!(c["kind"].as_str(), Some("updates" | "geometry" | "join"))
                || (c["kind"] == "e-o-e" && c["reuse_exponents"] == true))
    }) {
        for container in [false, true] {
            let open = |control: bool| -> Box<dyn AudioStream> {
                let key = match (container, control) {
                    (false, false) => "file",
                    (false, true) => "control",
                    (true, false) => "video",
                    (true, true) => "control_video",
                };
                let bytes = Cursor::new(read(c[key].as_str().unwrap()));
                if container {
                    Box::new(
                        fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                } else {
                    Box::new(
                        fvid::playback_ac3::Ac3AudioReader::open(bytes, Default::default())
                            .unwrap(),
                    )
                }
            };
            let expected = packets(&mut *open(false));
            assert_eq!(expected.len(), 3);
            let seek = Duration::from_millis(5);
            assert_eq!(expected[0].0, Duration::ZERO);
            let mut wanted = expected.clone();
            // Five ms / 240 samples lies inside both the 3- and 6-block first packets.
            wanted[0].0 = seek;
            wanted[0]
                .1
                .drain(..240 * 4 * c["channels"].as_u64().unwrap() as usize);
            audio_capture::worker_check_with_layout_and_seek_expected(
                open(false),
                &expected,
                seek,
                &wanted,
                speaker_mask(c),
            );
        }
    }
}
