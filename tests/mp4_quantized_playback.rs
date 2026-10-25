#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn open(path: PathBuf) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(
        Cursor::new(std::fs::read(path).unwrap()),
        Default::default(),
    )
    .unwrap()
}
fn worker(source: Mp4AudioReader<Cursor<Vec<u8>>>) -> transitions::Frames {
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let shared = trace.clone();
    let audio = AudioPlayback::start(Box::new(source), move || {
        Box::new(transitions::Capture::new(shared))
    });
    audio.play();
    transitions::ended(&audio, 0);
    let frames = trace.lock().unwrap().packets.clone();
    frames
}
#[test]
fn the_real_worker_recovers_coarse_aac_frames_instead_of_refusing_the_packet_window() {
    let exact = worker(open(
        root().join("mp4-quantized-playback/exact-constant-aac.mp4"),
    ));
    let actual = worker(open(
        root().join("mp4-quantized-playback/aac-playback-900.mp4"),
    ));
    transitions::compare(&actual, &exact);
}
#[test]
fn packet_time_and_pcm_match_exact_encoded_controls_on_all_coarse_variants() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("mp4-quantized-playback/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut cache = std::collections::HashMap::new();
    for case in m["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let exact = case["exact"].as_str().unwrap();
        eprintln!("variant {name}, control {exact}");
        let expected = cache.entry(exact.to_owned()).or_insert_with(|| {
            transitions::collect(&mut open(root().join("mp4-quantized-playback").join(exact)))
        });
        let mut source = open(
            root()
                .join("mp4-quantized-audio-clock")
                .join(format!("{name}.mp4")),
        );
        let actual = transitions::collect(&mut source);
        transitions::compare(&actual, expected);
    }
}
#[test]
fn ssr_container_clock_keeps_full_native_windows_and_matches_scalar_ipqf() {
    let expected =
        std::fs::read(root().join("mp4-quantized-playback/ssr-native-window.f32le")).unwrap();
    let actual = worker(open(
        root().join("mp4-quantized-playback/ssr-native-window.mp4"),
    ));
    let data: Vec<u8> = actual.into_iter().flat_map(|p| p.3).collect();
    assert_eq!(data.len(), expected.len());
    for (a, b) in data.chunks_exact(4).zip(expected.chunks_exact(4)) {
        let a = f32::from_le_bytes(a.try_into().unwrap());
        let b = f32::from_le_bytes(b.try_into().unwrap());
        assert!((a - b).abs() < 1e-6, "{a} != {b}");
    }
}
#[derive(Clone)]
struct NativePacket {
    rate: u32,
    channels: u16,
    pts: u64,
    num: u32,
    den: u32,
    data: Vec<u8>,
}
fn exact_packets(stream: &mut dyn AudioStream) -> Vec<NativePacket> {
    let mut decoder = stream.make_decoder().unwrap();
    let mut result = Vec::new();
    fn present(
        stream: &dyn AudioStream,
        mut frame: fvid::audio::DecodedAudio,
        out: &mut Vec<NativePacket>,
    ) {
        if let Some(n) = stream
            .decoded_packet_sample_limit(frame.source_pts, frame.source_duration)
            .unwrap()
        {
            let bytes = n * usize::from(stream.channels()) * 4;
            assert!(bytes <= frame.packet.data.len());
            frame.packet.data.truncate(bytes);
        }
        if let Some(p) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            out.push(NativePacket {
                rate: stream.sample_rate(),
                channels: stream.channels(),
                pts: p.pts,
                num: p.timebase_num,
                den: p.timebase_den,
                data: p.data,
            });
        }
    }
    loop {
        match stream.next_step().unwrap() {
            Some(fvid::audio::AudioStep::Encoded(p)) => {
                if let Some(f) = decoder
                    .decode_packet(&p.data, stream.decoder_stamp(&p), p.duration as u64)
                    .unwrap()
                {
                    present(stream, f, &mut result);
                }
            }
            Some(fvid::audio::AudioStep::ReconfigureDecoder) => {
                while let Some(f) = decoder.finish_packet().unwrap() {
                    present(stream, f, &mut result);
                }
                stream.advance_configuration().unwrap();
                decoder = stream.make_decoder().unwrap();
            }
            Some(fvid::audio::AudioStep::ResetDecoder) => {
                stream.reset_decoder(&mut decoder).unwrap()
            }
            Some(fvid::audio::AudioStep::Pcm(p)) => result.push(NativePacket {
                rate: stream.sample_rate(),
                channels: stream.channels(),
                pts: p.pts,
                num: p.timebase_num,
                den: p.timebase_den,
                data: p.data,
            }),
            None => {
                if !stream.drain_decoded_at_eof() {
                    decoder.reset();
                    break;
                }
                if let Some(f) = decoder.finish_packet().unwrap() {
                    present(stream, f, &mut result);
                    continue;
                }
                break;
            }
        }
    }
    stream.validate_eof().unwrap();
    result
}
fn suffix(packets: &[NativePacket], target: std::time::Duration) -> transitions::Frames {
    let mut out = Vec::new();
    for p in packets {
        let stride = usize::from(p.channels) * 4;
        let frames = p.data.len() / stride;
        let n = u128::from(p.pts) * u128::from(p.num);
        let d = u128::from(p.den);
        let wanted = target.as_nanos() * d;
        let skip = if wanted <= n * 1_000_000_000 {
            0
        } else {
            ((wanted - n * 1_000_000_000) * u128::from(p.rate))
                .div_ceil(d * 1_000_000_000)
                .min(frames as u128) as usize
        };
        if skip == frames {
            continue;
        }
        let at =
            (n * u128::from(p.rate) + skip as u128 * d) * 1_000_000_000 / (d * u128::from(p.rate));
        out.push((
            p.rate,
            p.channels,
            std::time::Duration::from_nanos(at as u64),
            p.data[skip * stride..].to_vec(),
        ));
    }
    out
}
#[test]
fn seek_and_rewind_preserve_exact_native_suffixes_below_container_tick_precision() {
    let m: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("mp4-quantized-playback/manifest.json")).unwrap(),
    )
    .unwrap();
    let mut cache = std::collections::HashMap::new();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let exact = c["exact"].as_str().unwrap();
        let control = cache.entry(exact.to_owned()).or_insert_with(|| {
            exact_packets(&mut open(root().join("mp4-quantized-playback").join(exact)))
        });
        let mut source = open(
            root()
                .join("mp4-quantized-audio-clock")
                .join(format!("{name}.mp4")),
        );
        for ns in [
            22_001_001,
            50_000_000,
            94_000_000,
            234_000_000,
            400_000_000,
            999_000_000,
        ] {
            let target = std::time::Duration::from_nanos(ns);
            eprintln!("seek {name}/{target:?}");
            source.seek_time(target);
            let actual = transitions::collect(&mut source);
            transitions::compare(&actual, &suffix(control, target));
        }
        source.rewind();
        transitions::compare(
            &transitions::collect(&mut source),
            &suffix(control, std::time::Duration::ZERO),
        );
    }
}

#[test]
fn worker_seek_after_eof_and_rewind_keep_the_native_pcm_clock() {
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root().join("mp4-quantized-playback/manifest.json")).unwrap(),
    )
    .unwrap();
    for name in [
        "constant-aac-900-floor",
        "ssr-edits-1000-nearest",
        "ps-edits-900-ceil",
        "adpcm-1000-floor",
        "pcm-900-nearest",
        "aac-fragmented-1000-ceil",
    ] {
        eprintln!("worker navigation {name}");
        let case = manifest["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let control = exact_packets(&mut open(
            root()
                .join("mp4-quantized-playback")
                .join(case["exact"].as_str().unwrap()),
        ));
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let shared = trace.clone();
        let mut audio = AudioPlayback::start(
            Box::new(open(
                root()
                    .join("mp4-quantized-audio-clock")
                    .join(format!("{name}.mp4")),
            )),
            move || Box::new(transitions::Capture::new(shared)),
        );
        audio.play();
        transitions::ended(&audio, 0);
        transitions::compare(
            &trace.lock().unwrap().packets,
            &suffix(&control, std::time::Duration::ZERO),
        );
        for (generation, ns) in [22_001_001, 234_000_000].into_iter().enumerate() {
            trace.lock().unwrap().packets.clear();
            let target = std::time::Duration::from_nanos(ns);
            audio.seek(target);
            transitions::ended(&audio, generation as u64 + 1);
            transitions::compare(&trace.lock().unwrap().packets, &suffix(&control, target));
        }
        trace.lock().unwrap().packets.clear();
        audio.rewind();
        transitions::ended(&audio, 3);
        transitions::compare(
            &trace.lock().unwrap().packets,
            &suffix(&control, std::time::Duration::ZERO),
        );
    }
}
