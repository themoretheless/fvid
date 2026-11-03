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
            .join("tests/fixtures/playback-errors/wave-block-seek")
            .join(n),
    )
    .unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn source(c: &serde_json::Value, kind: &str) -> Box<dyn AudioStream> {
    Box::new(
        fvid::playback_wav::WavAudioReader::open(
            Cursor::new(bytes(&format!(
                "{}-{kind}.wav",
                c["name"].as_str().unwrap()
            ))),
            Default::default(),
        )
        .unwrap(),
    )
}
fn control(c: &serde_json::Value) -> Vec<rational::Packet> {
    let mut r = fvid::playback_wav::WavAudioReader::open(
        Cursor::new(bytes(&format!("{}.pcm.wav", c["name"].as_str().unwrap()))),
        Default::default(),
    )
    .unwrap();
    rational::collect(&mut r)
}
#[test]
fn healthy_block_media_and_literal_pcm_video_controls_remain_valid() {
    for c in cases() {
        let full = control(&c);
        assert_eq!(
            full.iter().flat_map(|p| p.data.clone()).collect::<Vec<_>>(),
            bytes(&format!("{}.f32le", c["name"].as_str().unwrap()))
        );
        rational::exact(
            &rational::collect(&mut *source(&c, "good")),
            &full,
            "natural ADPCM",
        );
    }
    let mut r = fvid::playback_mp4::Mp4VideoReader::open_software(
        Cursor::new(bytes("neutral-video.mp4")),
        Default::default(),
        32 << 20,
    )
    .unwrap();
    for i in 0..3 {
        let f = r.read_frame().unwrap().unwrap();
        let pixels = f
            .picture
            .y
            .iter()
            .chain(&f.picture.cb)
            .chain(&f.picture.cr)
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(pixels, bytes(&format!("frame-{i}.yuv16")));
    }
    assert!(r.read_frame().unwrap().is_none());
}
fn try_collect(s: &mut dyn AudioStream) -> fvid::Result<Vec<rational::Packet>> {
    let mut decoder = s.make_decoder()?;
    let mut out = Vec::new();
    while let Some(p) = s.next_packet()? {
        if let Some(f) = decoder.decode_packet(&p.data, s.decoder_stamp(&p), p.duration as u64)? {
            if let Some(p) = s.present_decoded(f.packet, f.source_pts)? {
                out.push(rational::Packet::from_audio(
                    p,
                    s.sample_rate(),
                    s.channels(),
                ));
            }
        }
    }
    s.validate_eof()?;
    Ok(out)
}
#[test]
fn seeking_valid_later_blocks_bypasses_unrelated_corrupt_headers() {
    let mut failures = Vec::new();
    for c in cases() {
        let full = control(&c);
        let end = full.last().unwrap().point(full.last().unwrap().frames());
        let at = Duration::from_nanos(c["target_ns"].as_u64().unwrap());
        let mut s = source(&c, "earlier-bad");
        assert_eq!(s.seek_time(at), c["target_frame"].as_i64().unwrap());
        match try_collect(&mut *s) {
            Ok(got) => rational::exact(
                &got,
                &rational::suffix(&full, at, end),
                "seek past bad prefix",
            ),
            Err(e) => {
                failures.push(format!("{}: {e}", c["name"]));
                continue;
            }
        }
        for target in [
            c["target_frame"].as_i64().unwrap() - 1,
            c["target_frame"].as_i64().unwrap() + 1,
            i64::MAX,
        ] {
            let frame = (target as u64).min(c["frames"].as_u64().unwrap());
            assert_eq!(s.seek_to(target), frame as i64);
            let at = rational::duration(u128::from(frame), u128::from(c["rate"].as_u64().unwrap()));
            rational::exact(
                &rational::collect(&mut *s),
                &rational::suffix(&full, at, end),
                "raw block seek",
            );
        }
    }
    assert!(
        failures.is_empty(),
        "independent-block seek failures: {failures:?}"
    );
}
#[test]
fn clean_seek_decodes_less_than_one_block_of_preroll() {
    for c in cases() {
        let mut s = source(&c, "good");
        let target = c["target_frame"].as_u64().unwrap();
        s.seek_time(Duration::from_nanos(c["target_ns"].as_u64().unwrap()));
        let mut d = s.make_decoder().unwrap();
        let mut before = 0u64;
        loop {
            let p = s.next_packet().unwrap().unwrap();
            let frames = (p.data.len() / c["block_bytes"].as_u64().unwrap() as usize) as u64
                * c["block_frames"].as_u64().unwrap();
            before += target.saturating_sub(p.pts as u64).min(frames);
            if let Some(f) = d
                .decode_packet(&p.data, s.decoder_stamp(&p), p.duration as u64)
                .unwrap()
            {
                if let Some(p) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                    assert_eq!(p.pts, target);
                    break;
                }
            }
        }
        assert!(
            before < c["block_frames"].as_u64().unwrap(),
            "{}:decoded {before} earlier frames",
            c["name"]
        );
    }
}
#[test]
fn actual_workers_seek_past_bad_blocks_with_exact_pcm_and_anchor() {
    for c in cases() {
        let full = control(&c);
        let last = full.last().unwrap();
        let end = last.point(last.frames());
        for at in [
            Duration::from_nanos(c["target_ns"].as_u64().unwrap()),
            rational::duration(
                u128::from(c["target_frame"].as_u64().unwrap() + 1),
                u128::from(c["rate"].as_u64().unwrap()),
            ),
        ] {
            let wanted = rational::suffix(&full, at, end);
            let raw = Arc::new(Mutex::new(rational::RawTrace::default()));
            let copy = raw.clone();
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let mut worker =
                fvid::audio_thread::AudioPlayback::start(source(&c, "earlier-bad"), move || {
                    Box::new(rational::RawCapture {
                        inner: transitions::Capture::new(trace),
                        raw: copy,
                    })
                });
            worker.seek(at);
            worker.play();
            transitions::ended(&worker, worker.generation());
            let got = raw.lock().unwrap();
            rational::exact(&got.packets, &wanted, "ADPCM block worker");
            assert_eq!(got.anchors, vec![rational::anchor(&wanted, end)]);
        }
    }
}
#[test]
fn selected_corrupt_headers_and_rewound_bad_prefixes_still_refuse() {
    for c in cases() {
        for (kind, at) in [
            ("earlier-bad", Duration::ZERO),
            (
                "target-bad",
                Duration::from_nanos(c["target_ns"].as_u64().unwrap()),
            ),
        ] {
            let mut s = source(&c, kind);
            s.seek_time(at);
            let mut d = s.make_decoder().unwrap();
            let mut failed = None;
            while let Some(p) = s.next_packet().unwrap() {
                match d.decode_packet(&p.data, s.decoder_stamp(&p), p.duration as u64) {
                    Err(e) => {
                        failed = Some(e);
                        break;
                    }
                    Ok(_) => {}
                }
            }
            let e = failed.expect("selected corrupt block must fail");
            assert!(matches!(e, fvid::Error::Invalid(_)));
            assert!(e.to_string().contains(c["needle"].as_str().unwrap()), "{e}");
        }
        let mut s = source(&c, "earlier-bad");
        s.seek_time(Duration::MAX);
        assert!(s.next_packet().unwrap().is_none());
        s.rewind();
        let p = s.next_packet().unwrap().unwrap();
        assert!(
            s.make_decoder()
                .unwrap()
                .decode_packet(&p.data, s.decoder_stamp(&p), p.duration as u64)
                .is_err()
        );
    }
}
#[test]
fn file_payload_and_channel_budgets_still_precede_decode() {
    for c in cases() {
        let data = bytes(&format!("{}-good.wav", c["name"].as_str().unwrap()));
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
            let e = fvid::playback_wav::WavAudioReader::open(Cursor::new(data.clone()), limits)
                .err()
                .unwrap();
            assert!(matches!(e, fvid::Error::Invalid(_)));
        }
    }
}
