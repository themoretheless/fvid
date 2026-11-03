#![cfg(feature = "player")]
#[path = "support/rational_audio_seek.rs"]
mod rational;
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/avi-adpcm-start")
            .join(n),
    )
    .unwrap()
}
fn cases(k: &str) -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()[k]
        .as_array()
        .unwrap()
        .clone()
}
fn open(n: &str) -> fvid::Result<fvid::playback_avi_audio::AviAudioReader> {
    fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(bytes(n)), Default::default())
}
fn expected(c: &serde_json::Value) -> Vec<rational::Packet> {
    let mut control = open(c["control"].as_str().unwrap()).unwrap();
    let mut p = rational::collect(&mut control);
    for q in &mut p {
        q.num = q.num * 8000 + u128::from(c["origin"].as_u64().unwrap()) * q.den;
        q.den *= 8000;
    }
    p
}
#[test]
fn raw_start_units_ungrouped_controls_and_neutral_video_are_valid() {
    for c in cases("cases") {
        let data = bytes(c["file"].as_str().unwrap());
        let avi = fvid::container::avi::Avi::parse(&data, &Default::default()).unwrap();
        assert_eq!(
            u64::from(avi.audio()[0].start),
            c["start"].as_u64().unwrap()
        );
        let mut r = open(c["control"].as_str().unwrap()).unwrap();
        let pcm = rational::collect(&mut r)
            .into_iter()
            .flat_map(|p| p.data)
            .collect::<Vec<_>>();
        assert_eq!(pcm, bytes(c["expected"].as_str().unwrap()));
    }
    let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = r.read_frame().unwrap().unwrap();
        assert_eq!(
            f.picture
                .y
                .iter()
                .chain(&f.picture.cb)
                .chain(&f.picture.cr)
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>(),
            bytes(&format!("frame-{i}.yuv16"))
        );
    }
    assert!(r.read_frame().unwrap().is_none());
}
#[test]
fn delayed_adpcm_packets_pcm_duration_seek_and_rewind_keep_absolute_origin() {
    let mut failures = Vec::new();
    for c in cases("cases") {
        let name = c["file"].as_str().unwrap();
        let mut s = match open(name) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let origin = c["origin"].as_u64().unwrap();
        let frames = c["frames"].as_u64().unwrap();
        let spb = c["block_frames"].as_u64().unwrap();
        let end = origin + frames;
        let full = expected(&c);
        assert_eq!(
            s.duration(),
            Some(rational::duration(u128::from(end), 8000))
        );
        rational::exact(&rational::collect(&mut s), &full, "delayed natural PCM");
        for target in [
            0,
            origin.saturating_sub(1),
            origin + spb - 1,
            origin + spb,
            end,
            i64::MAX as u64,
        ] {
            let wanted = target.clamp(origin, end);
            let landed = if wanted == end {
                end
            } else {
                origin + (wanted - origin) / spb * spb
            };
            assert_eq!(s.seek_to(target as i64), landed as i64);
            let at = rational::duration(u128::from(landed), 8000);
            rational::exact(
                &rational::collect(&mut s),
                &rational::suffix(&full, at, (u128::from(end), 8000)),
                "delayed raw seek",
            );
        }
        s.rewind();
        rational::exact(&rational::collect(&mut s), &full, "delayed rewind");
        s.rewind();
        for i in 0..3 {
            let p = s.next_packet().unwrap().unwrap();
            assert_eq!(p.pts, (origin + i * spb) as i64);
            assert_eq!(p.duration, spb as i64);
        }
        assert!(s.next_packet().unwrap().is_none());
        s.validate_eof().unwrap();
    }
    assert!(failures.is_empty(), "delayed AVI refusals: {failures:?}");
}
#[test]
fn actual_workers_keep_delayed_start_and_eof_anchors() {
    for c in cases("cases") {
        let full = expected(&c);
        let end = (
            u128::from(c["origin"].as_u64().unwrap() + c["frames"].as_u64().unwrap()),
            8000,
        );
        for at in [
            Duration::ZERO,
            rational::duration(
                u128::from(c["origin"].as_u64().unwrap() + c["block_frames"].as_u64().unwrap()),
                8000,
            ),
            Duration::MAX,
        ] {
            let wanted = rational::suffix(&full, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker = fvid::audio_thread::AudioPlayback::start(
                Box::new(open(c["file"].as_str().unwrap()).unwrap()),
                move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                },
            );
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, "delayed AVI worker");
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
#[test]
fn malformed_records_and_resource_limits_still_refuse() {
    for c in cases("bad") {
        let error = match open(c["file"].as_str().unwrap()) {
            Err(error) => error,
            Ok(_) => panic!("malformed delayed record accepted"),
        };
        assert!(matches!(error, fvid::Error::Invalid(_)), "{error:?}");
        assert!(error.to_string().contains(c["needle"].as_str().unwrap()), "{error}");
    }
    for c in cases("cases") {
        let data = bytes(c["file"].as_str().unwrap());
        for limits in [
            fvid::container::avi::Limits {
                file_bytes: data.len() - 1,
                ..Default::default()
            },
            fvid::container::avi::Limits {
                channels: c["channels"].as_u64().unwrap() as usize - 1,
                ..Default::default()
            },
            fvid::container::avi::Limits {
                records: 2,
                ..Default::default()
            },
        ] {
            assert!(
                fvid::playback_avi_audio::AviAudioReader::open(Cursor::new(data.clone()), limits)
                    .is_err()
            );
        }
    }
}
