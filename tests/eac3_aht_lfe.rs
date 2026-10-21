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
            .join("tests/fixtures/playback-errors/eac3-aht-lfe")
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
fn open_case(c: &serde_json::Value, kind: &str, control: bool) -> Box<dyn AudioStream> {
    let key = match (kind, control) {
        ("raw", false) => "file",
        ("raw", true) => "control",
        ("mkv", false) => "video",
        ("mkv", true) => "control_video",
        ("mp4", false) => "mp4",
        ("mp4", true) => "control_mp4",
        _ => unreachable!(),
    };
    let bytes = Cursor::new(read(c[key].as_str().unwrap()));
    match kind {
        "raw" => {
            Box::new(fvid::playback_ac3::Ac3AudioReader::open(bytes, Default::default()).unwrap())
        }
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default()).unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(bytes, Default::default()).unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
#[test]
fn aht_lfe_and_mixed_full_band_planes_preserve_native_pcm_in_all_readers() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 48);
    for c in m["cases"].as_array().unwrap() {
        let raw = read(c["file"].as_str().unwrap());
        let h = frame(&raw).unwrap();
        assert_eq!((h.channels, h.blocks, h.frame_bytes), (3, 6, 4096));
        assert!(h.low_frequency);
        for (plane, bit) in [(0, 0), (1, 1), (6, 2)] {
            assert_eq!(
                h.aht[plane],
                c["mask"].as_u64().unwrap() & (1 << bit) != 0,
                "{} plane {plane}",
                c["name"]
            );
        }
        let mut d = Eac3Decoder::new(&[], 48000, 3).unwrap();
        let got = d.decode_complete(&raw, 0, 0).unwrap().unwrap().data;
        let mut d = Eac3Decoder::new(&[], 48000, 3).unwrap();
        let expected = d
            .decode_complete(&read(c["control"].as_str().unwrap()), 0, 0)
            .unwrap()
            .unwrap()
            .data;
        assert_eq!(got.len(), (3 * 1536 - 256) * 12);
        assert_pcm(&got, &expected, c, "direct LFE PCM");
        let peak = got
            .chunks_exact(12)
            .map(|v| f32::from_le_bytes(v[8..12].try_into().unwrap()).abs())
            .fold(0.0f32, f32::max);
        assert!(
            peak > 1e-5,
            "{} LFE must be nonzero, peak {peak}",
            c["name"]
        );
        for kind in ["raw", "mkv", "mp4"] {
            let mut control = open_case(c, kind, true);
            let mut d = control.make_decoder().unwrap();
            let expected = drain(&mut *control, &mut d);
            let mut stream = open_case(c, kind, false);
            assert_eq!(stream.channels(), 3);
            assert_eq!(stream.speaker_mask().unwrap(), Some(0x0b));
            let mut d = stream.make_decoder().unwrap();
            assert_pcm(&drain(&mut *stream, &mut d), &expected, c, kind);
            assert_eq!(stream.duration(), control.duration());
            stream.rewind();
            d.reset();
            assert_pcm(
                &drain(&mut *stream, &mut d),
                &expected,
                c,
                &format!("{kind} rewind"),
            );
        }
    }
}
fn assignment(kind: &str) {
    let m = manifest();
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "t0-m4-dc")
        .unwrap();
    let stream = open_case(c, kind, false);
    assert_eq!(
        stream.speaker_mask().unwrap(),
        Some(0x0b),
        "{kind}: FL/FR/LFE must not become FL/FR/FC"
    );
    let router = stream
        .stereo_router()
        .unwrap()
        .expect("Dolby speaker routing missing");
    assert_eq!(router.source_mask(), Some(0x0b));
    let impulse: Vec<u8> = [0f32, 0., 1., 1., 0., 0., 0., 1., 0.]
        .into_iter()
        .flat_map(f32::to_le_bytes)
        .collect();
    assert_eq!(
        router.route(&impulse, 24).unwrap(),
        [0., 0., 1., 0., 0., 1.]
    );
}
#[test]
fn native_lfe_assignment_reaches_backend_router() {
    assignment("raw");
}
#[test]
fn matroska_lfe_assignment_reaches_backend_router() {
    assignment("mkv");
}
#[test]
fn mp4_lfe_assignment_reaches_backend_router() {
    assignment("mp4");
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
fn lfe_pcm_and_assignment_survive_worker_rewind_and_partial_seek() {
    use std::time::Duration;
    let m = manifest();
    let selected: Vec<_> = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["exact"].as_bool() == Some(true) && c["mask"].as_u64().unwrap() >= 4)
        .collect();
    assert_eq!(selected.len(), 16);
    for c in selected {
        for kind in ["raw", "mkv", "mp4"] {
            let expected = packets(&mut *open_case(c, kind, true));
            assert_eq!(expected.len(), 3);
            let seek = Duration::from_millis(20);
            assert_eq!(expected[0].0, Duration::ZERO);
            let mut wanted = expected.clone();
            wanted[0].0 = seek;
            wanted[0].1.drain(..960 * 12);
            audio_capture::worker_check_with_layout_and_seek_expected(
                open_case(c, kind, false),
                &expected,
                seek,
                &wanted,
                0x0b,
            );
        }
    }
}
