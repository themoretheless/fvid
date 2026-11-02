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
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/wave64-playback")
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
fn source(c: &serde_json::Value, wave64: bool) -> Box<dyn AudioStream> {
    let name = if wave64 {
        format!("{}.w64", c["name"].as_str().unwrap())
    } else {
        c["control"].as_str().unwrap().into()
    };
    Box::new(
        fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes(&name)), Default::default())
            .expect("authored Wave must open"),
    )
}
#[test]
fn riff_waveform_and_neutral_video_controls_are_independent_of_wave64() {
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
fn wave64_sizes_tables_and_finite_fields_preserve_exact_native_pcm_and_packets() {
    for c in cases("cases") {
        let file = format!("{}.w64", c["name"].as_str().unwrap());
        let parsed = fvid::playback_wav::Wav::parse(&bytes(&file), &Default::default()).unwrap();
        assert!(!parsed.is_big_endian());
        assert_eq!(
            parsed.declared_bytes as u64,
            c["data_bytes"].as_u64().unwrap()
        );
        assert_eq!(
            parsed.declared_frames,
            if c["variant"] == "nofact" {
                None
            } else {
                Some(c["frames"].as_u64().unwrap() as usize)
            }
        );
        let mut s = source(&c, true);
        let mut r = source(&c, false);
        assert_eq!(s.speaker_mask().unwrap(), r.speaker_mask().unwrap());
        assert_eq!(s.codec(), r.codec());
        let full = rational::collect(&mut *r);
        rational::exact(&rational::collect(&mut *s), &full, "Wave64 full");
        s.rewind();
        r.rewind();
        while let Some(p) = s.next_packet().unwrap() {
            let q = r.next_packet().unwrap().unwrap();
            assert_eq!((p.pts, p.duration, p.data), (q.pts, q.duration, q.data));
        }
        assert!(r.next_packet().unwrap().is_none());
        s.validate_eof().unwrap();
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        for at in [
            Duration::from_nanos(1),
            Duration::from_millis(30),
            Duration::MAX,
        ] {
            s.seek_time(at);
            rational::exact(
                &rational::collect(&mut *s),
                &rational::suffix(&full, at, end),
                "Wave64 seek",
            );
        }
        s.rewind();
        rational::exact(&rational::collect(&mut *s), &full, "Wave64 rewind");
    }
}
#[test]
fn actual_workers_play_wave64_through_seek_and_eof() {
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
            rational::exact(&got.packets, &wanted, "Wave64 worker");
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
#[test]
fn wave64_metadata_failures_refuse_specifically_and_limits_remain_enforced() {
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
        let name = format!("{}.w64", c["name"].as_str().unwrap());
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
fn prefix(mut s: fvid::playback_wav::WavAudioReader) -> (Vec<u8>, String) {
    let mut d = s.make_decoder().unwrap();
    let mut out = Vec::new();
    while let Some(p) = s.next_packet().unwrap() {
        if let Some(f) = d
            .decode_packet(&p.data, s.decoder_stamp(&p), p.duration as u64)
            .unwrap()
        {
            if let Some(p) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                out.extend(p.data);
            }
        }
    }
    let e = s.validate_eof().unwrap_err().to_string();
    s.seek_to(i64::MAX);
    assert!(s.next_packet().unwrap().is_none());
    assert!(s.validate_eof().is_err());
    (out, e)
}
#[test]
fn missing_wave64_tail_and_large_declared_data_fail_eof_after_exact_bounded_prefix() {
    let s = fvid::playback_wav::WavAudioReader::open(
        Cursor::new(bytes("truncated-tail.w64")),
        Default::default(),
    )
    .unwrap();
    let (out, e) = prefix(s);
    assert_eq!(out, bytes("prefix.f32le"));
    assert!(e.contains("Wave64") && e.contains("truncated"), "{e}");
    for n in ["large-declared.w64", "large64-declared.w64"] {
        assert!(
            fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes(n)), Default::default())
                .is_err()
        );
        if usize::BITS >= 64 {
            let limits = fvid::playback_wav::Limits {
                file_bytes: usize::try_from(1u64 << 34).unwrap(),
                data_bytes: usize::try_from(1u64 << 34).unwrap(),
                ..Default::default()
            };
            let s =
                fvid::playback_wav::WavAudioReader::open(Cursor::new(bytes(n)), limits).unwrap();
            assert_eq!(s.wav().frames, 4097);
            let (out, e) = prefix(s);
            assert_eq!(out, bytes("prefix.f32le"));
            assert!(e.contains("data payload is truncated"), "{e}");
        }
    }
}
