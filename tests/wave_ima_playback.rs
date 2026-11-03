#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wave-ima-playback")
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(root().join(n)).unwrap()
}
fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}
fn source(c: &serde_json::Value, wave_ima: bool) -> Box<dyn AudioStream> {
    let name = if wave_ima {
        c["file"].as_str().unwrap().to_owned()
    } else {
        c["control"].as_str().unwrap().into()
    };
    Box::new(
        fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes(&name)), Default::default())
            .expect("authored Wave must open"),
    )
}
#[test]
fn riff_waveform_and_neutral_video_controls_are_independent_of_wave_ima() {
    for c in cases("cases") {
        let full = rational::collect(&mut *source(&c, false));
        assert_eq!(
            full.iter().map(rational::Packet::frames).sum::<usize>(),
            c["frames"].as_u64().unwrap() as usize
        );
        if let Some(n) = c["oracle"].as_str() {
            assert_eq!(
                full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
                bytes(n)
            );
        }
    }
    let mut v = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = v.read_frame().unwrap().unwrap();
        let got: Vec<_> = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect();
        assert_eq!(got, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(v.read_frame().unwrap().is_none());
}
#[test]
fn wave_ima_sizes_tables_and_finite_fields_preserve_exact_native_pcm_and_packets() {
    for c in cases("cases") {
        let file = c["file"].as_str().unwrap().to_owned();
        let parsed = fvid::playback_wav::Wav::parse(&bytes(&file), &Default::default()).unwrap();
        assert!(!parsed.is_big_endian());
        assert_eq!(
            parsed.declared_bytes as u64,
            c["data_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            parsed.declared_frames.map(|v| v as u64),
            c["declared_frames"].as_u64()
        );
        let mut s = source(&c, true);
        let mut r = source(&c, false);
        assert_eq!(s.speaker_mask().unwrap(), r.speaker_mask().unwrap());
        assert_eq!(s.codec(), "adpcm_ima_wav");
        let full = rational::collect(&mut *r);
        rational::exact(&rational::collect(&mut *s), &full, "WAVE IMA full");
        s.rewind();
        let mut encoded = Vec::new();
        let mut frames = 0;
        while let Some(p) = s.next_packet().unwrap() {
            assert!(!p.data.is_empty());
            assert_eq!(
                p.data.len() % c["block_bytes"].as_u64().unwrap() as usize,
                0
            );
            assert!(p.duration > 0);
            assert_eq!(p.pts, frames);
            frames += p.duration;
            encoded.extend(p.data);
        }
        assert_eq!(frames as u64, c["frames"].as_u64().unwrap());
        assert_eq!(encoded, bytes(c["encoded"].as_str().unwrap()));
        s.validate_eof().unwrap();
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        for at in [
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::from_nanos(255_375_000),
            Duration::from_nanos(255_375_001),
            Duration::from_nanos(511_125_001),
            Duration::MAX,
        ] {
            s.seek_time(at);
            rational::exact(
                &rational::collect(&mut *s),
                &rational::suffix(&full, at, end),
                "WAVE IMA seek",
            );
        }
        s.rewind();
        rational::exact(&rational::collect(&mut *s), &full, "WAVE IMA rewind");
    }
}
#[test]
fn actual_workers_play_wave_ima_through_seek_and_eof() {
    for c in cases("cases") {
        let full = rational::collect(&mut *source(&c, false));
        let last = full.last().unwrap();
        let end = last.point(last.frames());
        for at in [
            Duration::ZERO,
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(source(&c, true), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, "WAVE IMA worker");
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
#[test]
fn wave_ima_metadata_failures_refuse_specifically_and_limits_remain_enforced() {
    for c in cases("bad") {
        let e = fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes(c["file"].as_str().unwrap())),
            Default::default(),
        )
        .err()
        .unwrap();
        if c["unsupported"] == true {
            assert!(matches!(e, fvid::Error::Unsupported(_)), "{e}");
        } else {
            assert!(matches!(e, fvid::Error::Invalid(_)), "{e}");
        }
        assert!(
            e.to_string().contains(c["needle"].as_str().unwrap()),
            "{}: {e}",
            c["file"]
        );
    }
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap().to_owned();
        let data = bytes(&name);
        for limits in [
            fvid::playback_wav::Limits {
                file_bytes: data.len() - 1,
                ..Default::default()
            },
            fvid::playback_wav::Limits {
                data_bytes: c["data_bytes"].as_u64().unwrap() as usize - 1,
                ..Default::default()
            },
            fvid::playback_wav::Limits {
                channels: c["channels"].as_u64().unwrap() as usize - 1,
                ..Default::default()
            },
        ] {
            assert!(
                fvid::playback_wav::WavAudioReader::open(Cursor::new(data.clone()), limits)
                    .is_err()
            );
        }
    }
}

#[test]
fn truncated_ima_prefix_and_corrupt_block_refuse_at_the_specific_stage() {
    let mut reader = fvid::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("truncated-block-prefix.wav")),
        Default::default(),
    )
    .unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let mut got = Vec::new();
    while let Some(p) = reader.next_packet().unwrap() {
        if let Some(d) = decoder
            .decode_packet(&p.data, reader.decoder_stamp(&p), p.duration as u64)
            .unwrap()
        {
            if let Some(p) = reader.present_decoded(d.packet, d.source_pts).unwrap() {
                got.extend(p.data);
            }
        }
    }
    assert_eq!(got, bytes("truncated-block-prefix.f32le"));
    assert!(
        reader
            .validate_eof()
            .unwrap_err()
            .to_string()
            .contains("data payload is truncated")
    );
    reader.seek_time(Duration::MAX);
    assert!(reader.next_packet().unwrap().is_none());
    assert!(reader.validate_eof().is_err());
    let mut reader = fvid::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("bad-step-index.wav")),
        Default::default(),
    )
    .unwrap();
    let mut decoder = reader.make_decoder().unwrap();
    let p = reader.next_packet().unwrap().unwrap();
    let error = decoder
        .decode_packet(&p.data, reader.decoder_stamp(&p), p.duration as u64)
        .err()
        .unwrap();
    assert!(
        error.to_string().contains("step index exceeds 88"),
        "{error}"
    );
}

#[test]
fn unknown_data_length_does_not_hide_an_unfulfilled_fact_sample_count() {
    for (file, broken) in [
        ("unknown-data-complete.wav", false),
        ("unknown-data-short.wav", true),
    ] {
        let mut reader =
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes(file)), Default::default())
                .unwrap();
        let mut decoder = reader.make_decoder().unwrap();
        let mut got = Vec::new();
        while let Some(p) = reader.next_packet().unwrap() {
            if let Some(d) = decoder
                .decode_packet(&p.data, reader.decoder_stamp(&p), p.duration as u64)
                .unwrap()
            {
                if let Some(p) = reader.present_decoded(d.packet, d.source_pts).unwrap() {
                    got.extend(p.data);
                }
            }
        }
        assert_eq!(got, bytes("c1-b8-full.f32le"));
        if broken {
            let e = reader.validate_eof().unwrap_err();
            assert!(matches!(e, fvid::Error::Invalid(_)));
            assert!(e.to_string().contains("IMA fact"), "{e}");
            reader.seek_time(Duration::MAX);
            assert!(reader.next_packet().unwrap().is_none());
            assert!(reader.validate_eof().is_err());
        } else {
            reader.validate_eof().unwrap();
        }
    }
}
