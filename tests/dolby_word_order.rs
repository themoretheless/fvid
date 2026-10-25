#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/dolby-word-order")
}
fn open(name: &str, p: u8) -> Box<dyn AudioStream> {
    let bytes = Cursor::new(std::fs::read(root().join(name)).unwrap());
    if name.ends_with(".raw") {
        return Box::new(
            fvid::playback_ac3::Ac3AudioReader::open_programme(bytes, Default::default(), p)
                .unwrap(),
        );
    }
    if name.ends_with(".mkv") {
        return if name.starts_with("eac3") {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open_programme(
                    bytes,
                    Default::default(),
                    0,
                    p,
                )
                .unwrap(),
            )
        } else {
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(bytes, Default::default())
                    .unwrap(),
            )
        };
    }
    if name.starts_with("eac3") {
        Box::new(Mp4AudioReader::open_programme(bytes, Default::default(), 0, p).unwrap())
    } else {
        Box::new(Mp4AudioReader::open(bytes, Default::default()).unwrap())
    }
}

#[test]
fn little_endian_dolby_mp4_matches_big_endian_native_pcm_and_clock() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let p = c["programme"].as_u64().unwrap() as u8;
        eprintln!("{name}");
        let mut control = open(c["control"].as_str().unwrap(), p);
        let mut actual = open(name, p);
        assert_eq!(
            (actual.sample_rate(), actual.channels(), actual.duration()),
            (
                control.sample_rate(),
                control.channels(),
                control.duration()
            )
        );
        transitions::compare(
            &transitions::collect(actual.as_mut()),
            &transitions::collect(control.as_mut()),
        );
    }
}

#[test]
fn an_incomplete_little_endian_word_cannot_supply_an_unknown_header_byte() {
    for name in [
        "ac3",
        "ac3-bad-first",
        "eac3",
        "eac3-expanded",
        "eac3-coarse",
    ] {
        let be = std::fs::read(root().join(format!("{name}-be.raw"))).unwrap();
        let le = std::fs::read(root().join(format!("{name}-le.raw"))).unwrap();
        for n in 0..be.len().min(512) {
            let complete = n & !1;
            if name.starts_with("ac3") {
                assert_eq!(
                    fvid::codec::ac3_decoder::syncframe(&le[..n]),
                    fvid::codec::ac3_decoder::syncframe(&be[..complete]),
                    "{name} prefix {n}"
                );
            } else {
                let meta = |f: fvid::codec::eac3_decoder::Frame| {
                    (
                        f.sample_rate,
                        f.channels,
                        f.frame_bytes,
                        f.blocks,
                        f.audio_bit,
                    )
                };
                assert_eq!(
                    fvid::codec::eac3_decoder::programme_frame(&le[..n], 0).map(meta),
                    fvid::codec::eac3_decoder::programme_frame(&be[..complete], 0).map(meta),
                    "{name} prefix {n}"
                );
            }
        }
    }
}

#[test]
fn word_order_survives_seek_and_rewind_without_changing_pcm_history() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let p = c["programme"].as_u64().unwrap() as u8;
        eprintln!("seek {name}");
        let mut control = open(c["control"].as_str().unwrap(), p);
        let mut actual = open(name, p);
        for target in [
            std::time::Duration::from_nanos(22_001_001),
            std::time::Duration::from_millis(63),
        ] {
            let a = actual.seek_time(target);
            let b = control.seek_time(target);
            assert_eq!(actual.time_of(a), control.time_of(b));
            transitions::compare(
                &transitions::collect(actual.as_mut()),
                &transitions::collect(control.as_mut()),
            );
        }
        actual.rewind();
        control.rewind();
        transitions::compare(
            &transitions::collect(actual.as_mut()),
            &transitions::collect(control.as_mut()),
        );
    }
}
fn worker(source: Box<dyn AudioStream>) -> transitions::Frames {
    use std::sync::{Arc, Mutex};
    let trace = Arc::new(Mutex::new(transitions::Trace::default()));
    let shared = trace.clone();
    let audio = fvid::audio_thread::AudioPlayback::start(source, move || {
        Box::new(transitions::Capture::new(shared))
    });
    audio.play();
    transitions::ended(&audio, 0);
    let out = trace.lock().unwrap().packets.clone();
    out
}
#[test]
fn real_worker_plays_little_endian_and_per_frame_order_changes_with_native_geometry() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["file"].as_str().unwrap();
        let p = c["programme"].as_u64().unwrap() as u8;
        eprintln!("worker {name}");
        transitions::compare(
            &worker(open(name, p)),
            &worker(open(c["control"].as_str().unwrap(), p)),
        );
    }
}

#[test]
fn canonical_word_order_preserves_crc_refusal_and_demuxed_payload_bytes() {
    let bad = std::fs::read(root().join("ac3-bad-first-le.raw")).unwrap();
    assert!(
        fvid::codec::ac3_decoder::syncframe(&bad).is_some(),
        "LE header must be recognized before CRC refusal"
    );
    assert!(
        fvid::codec::ac3_decoder::checked_syncframe(&bad).is_none(),
        "word swapping must not repair a damaged CRC"
    );
    let good = std::fs::read(root().join("ac3-le.raw")).unwrap();
    assert!(fvid::codec::ac3_decoder::checked_syncframe(&good).is_some());
    let bytes = std::fs::read(root().join("eac3-corrupt-unselected-le.raw")).unwrap();
    assert!(fvid::codec::eac3_decoder::checked_programme_frame(&bytes, 1).is_some());
    assert!(fvid::codec::eac3_decoder::checked_programme_frame(&bytes, 7).is_none());
    for name in ["ac3-le.mp4", "ac3-grouped-le.mp4", "eac3-expanded-le.mp4"] {
        let mut reader = fvid::container::mp4::Mp4Reader::open_indexed(
            Cursor::new(std::fs::read(root().join(name)).unwrap()),
            Default::default(),
        )
        .unwrap();
        let index = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut packet = Vec::new();
        reader.read_packet(index, 0, &mut packet).unwrap();
        assert_eq!(
            &packet[..2],
            &[0x77, 0x0b],
            "demux must preserve encoded word order"
        );
    }
}

#[test]
fn damaged_raw_ac3_header_preserves_its_silent_window_and_later_native_setup() {
    let movie = std::fs::read(root().join("ac3-bad-first-be.mp4")).unwrap();
    let mut demux =
        fvid::container::mp4::Mp4Reader::open_indexed(Cursor::new(movie), Default::default())
            .unwrap();
    let index = demux
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let mut raw = Vec::new();
    let mut packet = Vec::new();
    for n in 0..demux.tracks()[index].samples.len() {
        demux.read_packet(index, n, &mut packet).unwrap();
        raw.extend_from_slice(&packet);
    }
    assert_eq!(
        raw,
        std::fs::read(root().join("ac3-bad-first-be.raw")).unwrap()
    );
    let mut audio =
        fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(raw), Default::default()).unwrap();
    assert_eq!((audio.sample_rate(), audio.channels()), (48000, 6));
    let pcm = transitions::collect(&mut audio);
    assert_eq!(pcm.len(), 3);
    assert_eq!(pcm[0].3.len(), (1536 - 256) * 6 * 4);
    assert!(
        pcm[0].3.iter().all(|v| *v == 0),
        "CRC-damaged first frame must conceal its entire audible window"
    );
    assert!(
        pcm[1..].iter().any(|p| p.3.iter().any(|v| *v != 0)),
        "later valid tone must remain audible"
    );
}
