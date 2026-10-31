#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{
    audio::{AudioDecode, AudioStream},
    codec::mace_decoder::{MaceCoding, MaceDecoder},
    container::mp4::Mp4Reader,
    playback_mp4_audio::Mp4AudioReader,
};
use std::{io::Cursor, path::Path};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(if name.contains("zero-grouped") {
                "tests/fixtures/playback-errors/mp4-mace-composition-runs"
            } else {
                "tests/fixtures/playback-errors/mp4-mace-transitions"
            })
            .join(format!("{name}.mp4")),
    )
    .unwrap()
}
fn open(data: &[u8]) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(data.to_vec()), Default::default()).unwrap()
}
fn reference(data: &[u8]) -> Vec<u8> {
    let mut demux = Mp4Reader::open(Cursor::new(data), Default::default()).unwrap();
    let index = demux
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let track = &demux.tracks()[index];
    let coding = if track.codec == *b"MAC3" {
        MaceCoding::ThreeToOne
    } else {
        MaceCoding::SixToOne
    };
    let mut decoder = MaceDecoder::new(coding, track.sample_rate, track.channels).unwrap();
    let count = track.samples.len();
    let mut result = Vec::new();
    let mut packet = Vec::new();
    for i in 0..count {
        demux.read_packet(index, i, &mut packet).unwrap();
        result.extend(decoder.decode_encoded(&packet, 0, 0).unwrap().unwrap().data);
    }
    result
}
#[test]
fn preroll_seeks_preserve_exact_predictor_pcm_and_endpoints() {
    for name in [
        "control-0",
        "control-1",
        "control-2",
        "control-6mono",
        "control-0-zero-grouped",
        "control-1-zero-grouped",
        "control-2-zero-grouped",
        "control-6mono-zero-grouped",
    ] {
        let data = bytes(name);
        let wanted = reference(&data);
        let mut reader = open(&data);
        let rate = u128::from(reader.sample_rate());
        let scale = u128::from(reader.track().timescale);
        let stride = usize::from(reader.channels()) * 4;
        let end = reader.track().duration as i64;
        // Fractional sample targets, reverse jumps, EOF and repeat after EOF.
        for target in [0, 1, 35, end - 1, end, i64::MAX, -1, 37, 36, 0] {
            let clamped = target.max(0).min(end);
            assert_eq!(reader.seek_to(target), clamped, "{name} target{target}");
            let actual = transitions::collect(&mut reader);
            let flat: Vec<_> = actual.iter().flat_map(|p| p.3.iter().copied()).collect();
            let first = (clamped as u128 * rate).div_ceil(scale) as usize;
            assert_eq!(
                flat,
                wanted[first.min(wanted.len() / stride) * stride..],
                "{name} target{target}"
            );
            if let Some(packet) = actual.first() {
                assert_eq!(
                    packet.2.as_nanos(),
                    first as u128 * 1_000_000_000 / rate,
                    "{name} timestamp{target}"
                );
            }
        }
        reader.rewind();
        let flat: Vec<_> = transitions::collect(&mut reader)
            .into_iter()
            .flat_map(|p| p.3)
            .collect();
        assert_eq!(flat, wanted);
    }
}
#[test]
fn direct_packet_seek_keeps_its_public_landing_contract() {
    for name in [
        "control-0",
        "control-1",
        "control-2",
        "control-6mono",
        "control-0-zero-grouped",
        "control-1-zero-grouped",
        "control-2-zero-grouped",
        "control-6mono-zero-grouped",
    ] {
        let data = bytes(name);
        let mut reader = open(&data);
        let samples: Vec<_> = (0..reader.track().samples.len())
            .map(|i| reader.track().samples.get(i).unwrap())
            .collect();
        for target in [-1, 0, 1, 35, 36, 37, i64::MAX] {
            let index = samples
                .iter()
                .enumerate()
                .filter(|(_, s)| s.pts <= target)
                .max_by_key(|(_, s)| s.pts)
                .map_or(0, |(i, _)| i);
            assert_eq!(reader.seek(target), samples[index].pts);
            assert_eq!(
                reader.read_packet().unwrap().unwrap().pts,
                samples[index].pts
            );
        }
    }
}

#[test]
fn actual_audio_worker_replays_history_after_repeated_seeks() {
    use fvid::audio_thread::AudioPlayback;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use transitions::{Capture, Trace, ended};
    for name in [
        "control-0",
        "control-1",
        "control-2",
        "control-6mono",
        "control-0-zero-grouped",
        "control-1-zero-grouped",
        "control-2-zero-grouped",
        "control-6mono-zero-grouped",
    ] {
        let data = bytes(name);
        let wanted = reference(&data);
        let stream = open(&data);
        let rate = u128::from(stream.sample_rate());
        let stride = usize::from(stream.channels()) * 4;
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut audio =
            AudioPlayback::start(Box::new(stream), move || Box::new(Capture::new(shared)));
        audio.play();
        ended(&audio, 0);
        for micros in [125, 5000, 29000, 30000, 100000, 0] {
            trace.lock().unwrap().packets.clear();
            audio.seek(Duration::from_micros(micros));
            ended(&audio, audio.generation());
            let frames = &trace.lock().unwrap().packets;
            let got: Vec<_> = frames.iter().flat_map(|p| p.3.iter().copied()).collect();
            let first = (u128::from(micros) * rate).div_ceil(1_000_000) as usize;
            assert_eq!(
                got,
                wanted[first.min(wanted.len() / stride) * stride..],
                "{name} {micros}us"
            );
        }
    }
}
