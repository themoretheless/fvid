use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-audio-middle-gap")
        .join(n)
}
fn bytes(n: &str) -> Vec<u8> {
    std::fs::read(path(n)).unwrap()
}
fn cases() -> Vec<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(&bytes("manifest.json")).unwrap()["cases"]
        .as_array()
        .unwrap()
        .clone()
}
fn num(c: &serde_json::Value, k: &str) -> u64 {
    c[k].as_u64().unwrap()
}
fn expected(c: &serde_json::Value) -> Vec<u8> {
    let pcm = bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap()));
    let block = num(c, "packet_frames") as usize * num(c, "channels") as usize * 4;
    assert!(
        pcm.chunks_exact(4)
            .any(|f| f32::from_le_bytes(f.try_into().unwrap()).abs() > 1e-5)
    );
    let mut out = pcm[..block].to_vec();
    out.resize(block * 2, 0);
    out.extend_from_slice(&pcm[block..]);
    out.repeat(num(c, "repeats") as usize)
}
fn compare(a: &[u8], b: &[u8], name: &str) {
    assert_eq!(a.len(), b.len(), "{name}: PCM length");
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        let (a, b) = (
            f32::from_le_bytes(a.try_into().unwrap()),
            f32::from_le_bytes(b.try_into().unwrap()),
        );
        assert!((a - b).abs() <= 1e-6, "{name} sample{i}: {a} != {b}");
    }
}
fn slice(c: &serde_json::Value, interval: Option<(Duration, Duration)>) -> Vec<u8> {
    let pcm = expected(c);
    let stride = num(c, "channels") as usize * 4;
    if let Some((a, b)) = interval {
        let at = |d: Duration| {
            ((d.as_nanos() * u128::from(num(c, "rate"))).div_ceil(1_000_000_000) as usize * stride)
                .min(pcm.len())
        };
        pcm[at(a)..at(b)].to_vec()
    } else {
        pcm
    }
}
#[test]
fn both_parsers_preserve_the_gap_packet_bytes_clocks_and_decodable_video() {
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut a =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        let mut b =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(a.tracks()[1].samples.len(), 3);
        assert_eq!(b.tracks()[1].samples.len(), 3);
        assert_eq!(a.tracks()[1].duration, num(&c, "media_step") * 4);
        for i in 0..3 {
            let s = a.tracks()[1].samples.get(i).unwrap();
            let t = b.tracks()[1].samples.get(i).unwrap();
            let pts = [0, 2, 3][i] * num(&c, "media_step");
            assert_eq!(
                (s.pts, s.dts, s.duration),
                (pts as i64, pts, num(&c, "media_step") as u32),
                "{n}"
            );
            assert_eq!((t.pts, t.dts, t.duration), (s.pts, s.dts, s.duration));
            let (mut x, mut y) = (Vec::new(), Vec::new());
            a.read_packet(1, i, &mut x).unwrap();
            b.read_packet(1, i, &mut y).unwrap();
            assert_eq!(
                x,
                bytes(&format!("{}-{i}.packet", c["codec"].as_str().unwrap()))
            );
            assert_eq!(x, y);
        }
        let info = fvid_media::owned_mp4_probe::probe_mp4(&path(n)).unwrap();
        assert_eq!(
            info.streams[1].duration,
            Some((num(&c, "media_step") * 4 * num(&c, "repeats")) as i64),
            "{n}"
        );
        let video = fvid_media::decode_video(&path(n)).unwrap();
        assert_eq!(video.video_frames, num(&c, "video_frames"));
        assert_eq!(video.decode_errors, 0);
    }
}
#[test]
fn owned_decode_preserves_history_and_inserts_only_the_middle_silent_interval() {
    let mut wrong = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        for interval in [
            None,
            Some((Duration::from_millis(25), Duration::from_millis(35))),
            Some((Duration::from_millis(45), Duration::from_millis(55))),
            Some((Duration::from_millis(10), Duration::from_millis(65))),
        ] {
            let mut out = Vec::new();
            match fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(n)),
                &mut out,
                interval,
                &Default::default(),
            ) {
                Err(e) => wrong.push(format!("{n} {interval:?}: {e}")),
                Ok(s) => {
                    let oracle = slice(&c, interval);
                    compare(&out, &oracle, n);
                    assert_eq!(
                        s.sample_frames,
                        oracle.len() as u64 / 4 / num(&c, "channels")
                    );
                }
            }
        }
    }
    assert!(wrong.is_empty(), "middle-gap decoding: {wrong:?}");
}
#[test]
fn gap_silence_does_not_consume_the_encoded_packet_work_budget() {
    for c in cases()
        .into_iter()
        .filter(|c| c["mode"] == "none" && c["stamp"] == "v0")
    {
        let n = c["file"].as_str().unwrap();
        for limit in 1..=3 {
            let mut out = Vec::new();
            let s = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(n)),
                &mut out,
                None,
                &fvid_media::CopyOptions {
                    max_packets: Some(limit),
                    ..Default::default()
                },
            )
            .unwrap();
            let frames = num(&c, "packet_frames") * (limit + u64::from(limit > 1));
            assert_eq!(s.decoded_frames, limit);
            assert_eq!(s.sample_frames, frames, "{n} limit{limit}");
            compare(
                &out,
                &expected(&c)[..frames as usize * num(&c, "channels") as usize * 4],
                n,
            );
        }
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn native_gap_playback_keeps_pcm_timestamps_seek_rewind_and_eof() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    let mut wrong = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let end = Duration::from_nanos(num(&c, "frames") * 1_000_000_000 / num(&c, "rate"));
        assert_eq!(r.duration(), Some(end), "{n}");
        for _ in 0..2 {
            let f = transitions::collect(&mut r);
            let out: Vec<_> = f.iter().flat_map(|p| p.3.iter().copied()).collect();
            if out.len() != expected(&c).len() {
                wrong.push(format!(
                    "{n}: {} instead of{} PCM bytes",
                    out.len(),
                    expected(&c).len()
                ));
            } else {
                compare(&out, &expected(&c), n);
                let mut frames = 0;
                for p in f {
                    assert_eq!(
                        p.2,
                        Duration::from_nanos(frames * 1_000_000_000 / num(&c, "rate")),
                        "{n}"
                    );
                    frames += p.3.len() as u64 / 4 / num(&c, "channels");
                }
            }
            assert!(transitions::collect(&mut r).is_empty());
            r.rewind();
        }
        let mut seeks = Vec::new();
        for cycle in 0..num(&c, "repeats") {
            for within in [
                num(&c, "packet_frames") * 3 / 2,
                num(&c, "packet_frames") * 5 / 2,
            ] {
                seeks.push(Duration::from_nanos(
                    (cycle * 4 * num(&c, "packet_frames") + within) * 1_000_000_000
                        / num(&c, "rate"),
                ));
            }
        }
        for from in seeks {
            r.seek_time(from);
            let f = transitions::collect(&mut r);
            let out: Vec<_> = f.iter().flat_map(|p| p.3.iter().copied()).collect();
            let oracle = slice(&c, Some((from, end)));
            if out.len() != oracle.len() {
                wrong.push(format!(
                    "{n} seek{from:?}: {} instead of{} bytes",
                    out.len(),
                    oracle.len()
                ));
            } else {
                compare(&out, &oracle, n);
                assert_eq!(f.first().unwrap().2, from, "{n}");
            }
        }
        r.seek_time(end);
        assert!(
            transitions::collect(&mut r).is_empty(),
            "{n}: EOF replayed source"
        );
    }
    assert!(wrong.is_empty(), "native middle gaps: {wrong:?}");
}
#[cfg(feature = "player")]
#[test]
fn actual_workers_keep_the_gap_and_seek_into_its_silence() {
    use fvid::{audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
    use std::sync::{Arc, Mutex};
    for c in cases()
        .into_iter()
        .filter(|c| (c["mode"] == "none" || c["mode"] == "repeat") && c["stamp"] == "v0")
    {
        let n = c["file"].as_str().unwrap();
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let captured = trace.clone();
        let r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        let mut audio = AudioPlayback::start(Box::new(r), move || {
            Box::new(transitions::Capture::new(captured))
        });
        audio.play();
        transitions::ended(&audio, 0);
        let out: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|p| p.3.iter().copied())
            .collect();
        compare(&out, &expected(&c), n);
        trace.lock().unwrap().packets.clear();
        audio.seek(Duration::from_millis(25));
        audio.play();
        transitions::ended(&audio, audio.generation());
        let packets = trace.lock().unwrap().packets.clone();
        assert_eq!(packets.first().unwrap().2, Duration::from_millis(25));
        let out: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
        let end = Duration::from_nanos(num(&c, "frames") * 1_000_000_000 / num(&c, "rate"));
        compare(&out, &slice(&c, Some((Duration::from_millis(25), end))), n);
    }
}
#[test]
fn whole_pcm_resampling_preserves_middle_gap_boundaries() {
    for c in cases()
        .into_iter()
        .filter(|c| c["mode"] == "none" && c["stamp"] == "v0")
    {
        let n = c["file"].as_str().unwrap();
        let dest = std::env::temp_dir().join(format!(
            "fvid-middle-gap-normalize-{}-{n}.wav",
            std::process::id()
        ));
        assert!(!dest.exists());
        let result = fvid_media::decode_audio_transformed(
            &path(n),
            &dest,
            fvid_media::AudioDecodeTransform {
                sample_rate: Some((num(&c, "rate") * 2) as i32),
                ..Default::default()
            },
            &Default::default(),
        );
        let data = std::fs::read(&dest);
        if dest.exists() {
            std::fs::remove_file(&dest).unwrap();
        }
        let stats = result.unwrap();
        assert_eq!(stats.sample_frames, num(&c, "frames") * 2, "{n}");
        let data = data.unwrap();
        let info = fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&data), None).unwrap();
        assert_eq!(
            info.sample_rate,
            u32::try_from(num(&c, "rate") * 2).unwrap()
        );
        assert_eq!(u64::from(info.channels), num(&c, "channels"));
        assert!(info.float && matches!(info.bits_per_sample, 32 | 64));
        let pcm = &data
            [info.data_offset as usize..(info.data_offset + u64::from(info.data_bytes)) as usize];
        let scalar = usize::from(info.bits_per_sample / 8);
        let expected = bytes(&format!(
            "{}-normalized-2x.f32le",
            c["codec"].as_str().unwrap()
        ));
        assert_eq!(pcm.len() / scalar, expected.len() / 4, "{n}");
        for (i, (a, b)) in pcm
            .chunks_exact(scalar)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let a = if scalar == 8 {
                f64::from_le_bytes(a.try_into().unwrap())
            } else {
                f64::from(f32::from_le_bytes(a.try_into().unwrap()))
            };
            let b = f64::from(f32::from_le_bytes(b.try_into().unwrap()));
            assert!(
                (a - b).abs() <= 2e-6,
                "{n} normalized sample{i}: {a} != {b}"
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn a_burst_of_real_decoded_frames_drains_after_bounded_gap_silence() {
    use fvid::{
        audio::{AudioStep, AudioStream},
        playback_mp4_audio::Mp4AudioReader,
    };
    let c = cases()
        .into_iter()
        .find(|c| c["codec"] == "aac" && c["mode"] == "none" && c["stamp"] == "v0")
        .unwrap();
    let n = c["file"].as_str().unwrap();
    let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    let mut first = match r.next_step().unwrap() {
        Some(AudioStep::ReconfigureDecoder) => {
            r.advance_configuration().unwrap();
            None
        }
        Some(AudioStep::Encoded(packet)) => Some(packet),
        _ => panic!("unexpected initial audio step"),
    };
    let mut decoder = r.make_decoder().unwrap();
    let mut ready = Vec::new();
    while let Some(p) = first.take().or_else(|| r.next_packet().unwrap()) {
        if let Some(frame) = decoder
            .decode_packet(&p.data, r.decoder_stamp(&p), p.duration as u64)
            .unwrap()
        {
            ready.push(frame);
        }
    }
    assert_eq!(ready.len(), 3);
    // Model delayed delivery at the actual decoder-drain boundary, using real
    // owned AAC output rather than a hand-written mock decoder or PCM pattern.
    assert!(matches!(
        r.next_step().unwrap(),
        Some(AudioStep::ReconfigureDecoder)
    ));
    let mut out = Vec::new();
    for frame in ready {
        if let Some(p) = r.present_decoded(frame.packet, frame.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(p) = r.present_decoded(frame.packet, frame.source_pts).unwrap() {
            out.extend(p.data);
        }
    }
    r.advance_configuration().unwrap();
    out.extend(transitions::collect(&mut r).into_iter().flat_map(|f| f.3));
    compare(&out, &expected(&c), n);
}
#[test]
fn a_silent_interval_stops_before_the_oversized_post_gap_ps_packet() {
    let c = cases()
        .into_iter()
        .find(|c| c["codec"] == "ps" && c["mode"] == "none" && c["stamp"] == "v0")
        .unwrap();
    let n = c["file"].as_str().unwrap();
    assert_eq!(bytes("ps-0.packet").len(), 16);
    assert!(bytes("ps-1.packet").len() > 16);
    let interval = Some((Duration::from_millis(60), Duration::from_millis(70)));
    let mut out = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(n)),
        &mut out,
        interval,
        &fvid_media::CopyOptions {
            max_packet_bytes: 16,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.sample_frames, 480);
    assert_eq!(out.len(), 480 * 2 * 4);
    assert!(out.iter().all(|&b| b == 0));
    compare(&out, &slice(&c, interval), n);
}
#[test]
fn middle_gap_controls_keep_cancellation_packet_and_memory_refusals() {
    let cancelled = fvid::media_control::CancelFlag::new();
    cancelled.cancel();
    for c in cases()
        .into_iter()
        .filter(|c| c["mode"] == "none" && c["stamp"] == "v0")
    {
        let n = c["file"].as_str().unwrap();
        let mut out = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut out,
            None,
            &fvid_media::CopyOptions {
                max_controlled_bytes: Some(64 << 20),
                ..Default::default()
            },
        )
        .unwrap();
        compare(&out, &expected(&c), n);
        for (options, reason) in [
            (
                fvid_media::CopyOptions {
                    cancel: Some(cancelled.clone()),
                    ..Default::default()
                },
                "media operation cancelled",
            ),
            (
                fvid_media::CopyOptions {
                    max_packet_bytes: 1,
                    ..Default::default()
                },
                "MP4 packet exceeds budget",
            ),
            (
                fvid_media::CopyOptions {
                    max_packets: Some(0),
                    ..Default::default()
                },
                "packet work limit requires at least one packet",
            ),
            (
                fvid_media::CopyOptions {
                    max_controlled_bytes: Some(1),
                    ..Default::default()
                },
                "controlled memory budget exceeded",
            ),
        ] {
            let mut out = Vec::new();
            let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(n)),
                &mut out,
                None,
                &options,
            )
            .unwrap_err();
            assert!(error.to_string().contains(reason), "{n}: {error}");
            assert!(out.is_empty(), "{n}: resource refusal published PCM");
        }
    }
}
#[test]
fn middle_silence_does_not_mask_a_broken_following_codec_packet() {
    for (codec, reason) in [
        ("aac", "AAC block has no configured audio element"),
        ("alac", "ALAC frame does not hold every channel"),
    ] {
        let n = format!("{codec}-invalid-after-gap.mp4");
        let mut r =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
                .unwrap();
        assert_eq!(r.tracks()[1].samples.len(), 3);
        let mut p = Vec::new();
        r.read_packet(1, 1, &mut p).unwrap();
        assert_eq!(p, [0xff]);
        let mut out = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(&n)),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert_eq!(error.to_string(), reason, "{n}");
        let dest = std::env::temp_dir().join(format!(
            "fvid-middle-gap-refuse-{}-{codec}.wav",
            std::process::id()
        ));
        assert!(!dest.exists());
        let result = fvid_media::decode_audio_transformed(
            &path(&n),
            &dest,
            Default::default(),
            &Default::default(),
        );
        let published = dest.exists();
        if published {
            std::fs::remove_file(&dest).unwrap();
        }
        assert!(result.unwrap_err().contains(reason));
        assert!(!published, "{n}: invalid codec output was published");
        #[cfg(feature = "player")]
        {
            use fvid::{
                audio_thread::{AudioEvent, AudioPlayback},
                playback_mp4_audio::Mp4AudioReader,
            };
            use std::{
                sync::{Arc, Mutex},
                time::Instant,
            };
            let trace = Arc::new(Mutex::new(transitions::Trace::default()));
            let captured = trace.clone();
            let r = Mp4AudioReader::open(Cursor::new(bytes(&n)), Default::default()).unwrap();
            let mut audio = AudioPlayback::start(Box::new(r), move || {
                Box::new(transitions::Capture::new(captured))
            });
            audio.play();
            let until = Instant::now() + Duration::from_secs(3);
            'wait: loop {
                assert!(Instant::now() < until, "{n}: error delivery timeout");
                while let Some(event) = audio.poll() {
                    match event {
                        AudioEvent::Error(error) => {
                            assert!(error.to_string().contains(reason), "{n}: {error}");
                            break 'wait;
                        }
                        AudioEvent::Ended(_) => {
                            panic!("{n}: corrupt post-gap packet silently ended")
                        }
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
#[test]
fn a_long_empty_interval_seeks_and_renders_bounded_pcm_instead_of_allocating_its_duration() {
    let n = "pcm-long-gap.mp4";
    assert!(bytes(n).len() < 65536);
    let r =
        fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
    assert_eq!(r.tracks()[1].samples.len(), 3);
    assert!(r.tracks()[1].duration * 4 > 5_000_000_000);
    let from = Duration::from_secs(24 * 3600);
    let to = from + Duration::from_millis(10);
    let mut pcm = Vec::new();
    let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes(n)),
        &mut pcm,
        Some((from, to)),
        &fvid_media::CopyOptions {
            max_controlled_bytes: Some(8 << 20),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        (stats.sample_frames, stats.sample_rate, stats.channels),
        (80, 8000, 1)
    );
    assert_eq!(pcm.len(), 320);
    assert!(pcm.iter().all(|&b| b == 0));
    #[cfg(feature = "player")]
    {
        use fvid::{
            audio::{AudioStep, AudioStream},
            playback_mp4_audio::Mp4AudioReader,
        };
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
        assert_eq!(
            r.duration(),
            Some(Duration::from_secs(2 * 24 * 3600) + Duration::from_millis(60))
        );
        r.seek_time(from);
        let mut seen = false;
        for _ in 0..3 {
            match r.next_step().unwrap() {
                Some(AudioStep::ReconfigureDecoder) => r.advance_configuration().unwrap(),
                Some(AudioStep::Pcm(p)) => {
                    assert_eq!(p.presentation_time(), from);
                    assert!(!p.data.is_empty() && p.data.len() <= 1024 * 4);
                    assert!(p.data.iter().all(|&b| b == 0));
                    seen = true;
                    break;
                }
                _ => panic!("a seek inside declared silence tried to decode source data"),
            }
        }
        assert!(seen);
    }
}

#[cfg(feature = "player")]
#[test]
fn coarse_clock_gap_preserves_every_native_sample() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    for codec in ["pcm", "alac"] {
        let c = cases().into_iter().find(|c| c["codec"] == codec && c["mode"] == "none" && c["stamp"] == "v0").unwrap();
        let name = format!("{codec}-coarse.mp4");
        let mut r = Mp4AudioReader::open(Cursor::new(bytes(&name)), Default::default()).unwrap();
        let packets = transitions::collect(&mut r);
        let out: Vec<_> = packets.iter().flat_map(|p| p.3.iter().copied()).collect();
        compare(&out, &expected(&c), &name);
        assert_eq!(r.duration(), Some(Duration::from_nanos(40_000_000_000 / 1001 + 40_000_000)));
        assert_eq!(packets[2].2, Duration::from_nanos(40_000_000_000 / 1001));
        r.seek_time(Duration::from_millis(30));
        let suffix = transitions::collect(&mut r);
        let out: Vec<_> = suffix.iter().flat_map(|p| p.3.iter().copied()).collect();
        compare(&out, &expected(&c)[240 * 4..], &name);
        assert_eq!(suffix[0].2, Duration::from_millis(30));
        r.seek_time(r.duration().unwrap());
        assert!(transitions::collect(&mut r).is_empty());
    }
}
