use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn path(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-open-edit-tail")
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
    let stride = num(c, "channels") as usize * 4;
    let mut out = bytes(&format!("{}-oracle.f32le", c["codec"].as_str().unwrap()))
        [num(c, "start_frames") as usize * stride..]
        .to_vec();
    assert!(
        out.chunks_exact(4)
            .any(|f| f32::from_le_bytes(f.try_into().unwrap()).abs() > 1e-5)
    );
    out.resize(out.len() + num(c, "tail_frames") as usize * stride, 0);
    out
}
fn compare(a: &[u8], b: &[u8], n: &str) {
    assert_eq!(a.len(), b.len(), "{n}: PCM length");
    for (i, (a, b)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        assert!(
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                <= 1e-6,
            "{n}: PCM sample{i}"
        );
    }
}
#[test]
fn open_edits_retain_the_declared_packet_free_endpoint_in_both_parsers() {
    let mut wrong = Vec::new();
    for c in cases() {
        let n = c["file"].as_str().unwrap();
        let a = fvid::container::mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let b = fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
            .unwrap();
        let t = &a.tracks()[1];
        let u = &b.tracks()[1];
        assert_eq!(t.duration, num(&c, "end_media_ticks"), "{n}");
        assert_eq!(t.samples.len(), 3);
        assert_eq!(t.edits[0].duration, 0);
        assert_eq!(u.duration, t.duration);
        assert_eq!(u.samples.len(), 3);
        for (label, d, scale) in [
            (
                "root",
                t.resolved_edit_clock(a.movie_timescale()).unwrap().edits()[0].duration,
                t.timescale,
            ),
            (
                "owned",
                u.resolved_edit_clock(b.movie_timescale()).unwrap().edits()[0].duration,
                u.timescale,
            ),
        ] {
            if d != num(&c, "edit_media_ticks") {
                wrong.push(format!(
                    "{n} {label}: endpoint {d}, expected{} at{scale}",
                    num(&c, "edit_media_ticks")
                ));
            }
        }
        assert_eq!(
            t.edits[0].duration, 0,
            "resolution must not rewrite the open marker"
        );
    }
    assert!(wrong.is_empty(), "open-edit empty tails: {wrong:?}");
}
#[test]
fn owned_open_and_finite_edits_preserve_nonzero_pcm_then_exact_tail_silence() {
    for c in cases() {
        let golden = expected(&c);
        for key in ["known", "file"] {
            let n = c[key].as_str().unwrap();
            let mut out = Vec::new();
            let s = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(n)),
                &mut out,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(s.sample_frames, num(&c, "frames"), "{n}");
            compare(&out, &golden, n);
        }
    }
}
#[cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[cfg(feature = "player")]
#[test]
fn open_edit_playback_keeps_duration_tail_seek_rewind_and_eof() {
    use fvid::{audio::AudioStream, playback_mp4_audio::Mp4AudioReader};
    use std::time::Duration;
    for c in cases() {
        for key in ["file", "known"] {
            let n = c[key].as_str().unwrap();
            let rate = num(&c, "rate");
            let golden = expected(&c);
            let mut r = Mp4AudioReader::open(Cursor::new(bytes(n)), Default::default()).unwrap();
            let end = Duration::from_nanos(num(&c, "frames") * 1_000_000_000 / rate);
            assert_eq!(r.duration(), Some(end), "{n}");
            for _ in 0..2 {
                let frames = transitions::collect(&mut r);
                let actual: Vec<_> = frames.iter().flat_map(|f| f.3.iter().copied()).collect();
                compare(&actual, &golden, n);
                let mut count = 0u64;
                for f in frames {
                    assert_eq!(
                        f.2,
                        Duration::from_nanos(count * 1_000_000_000 / rate),
                        "{n}"
                    );
                    count += (f.3.len() / 4 / num(&c, "channels") as usize) as u64;
                }
                assert_eq!(count, num(&c, "frames"));
                assert!(transitions::collect(&mut r).is_empty());
                r.rewind();
            }
            let from = Duration::from_nanos(
                (num(&c, "frames") - num(&c, "tail_frames")) * 1_000_000_000 / rate,
            ) + Duration::from_millis(1);
            r.seek_time(from);
            let frames = transitions::collect(&mut r);
            assert_eq!(frames.first().unwrap().2, from, "{n}");
            let actual: Vec<_> = frames.iter().flat_map(|f| f.3.iter().copied()).collect();
            let first = ((from.as_nanos() * u128::from(rate)).div_ceil(1_000_000_000) as usize)
                * num(&c, "channels") as usize
                * 4;
            compare(&actual, &golden[first..], n);
            assert!(actual.iter().all(|&b| b == 0));
            r.seek_time(end);
            assert!(
                transitions::collect(&mut r).is_empty(),
                "{n}: endpoint replayed media"
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn actual_audio_workers_play_the_empty_tail_and_seek_into_it() {
    use fvid::{audio_thread::AudioPlayback, playback_mp4_audio::Mp4AudioReader};
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    for c in cases()
        .into_iter()
        .filter(|c| c["file"].as_str().unwrap().contains("v0-trim0"))
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
        let actual: Vec<_> = trace
            .lock()
            .unwrap()
            .packets
            .iter()
            .flat_map(|f| f.3.iter().copied())
            .collect();
        compare(&actual, &expected(&c), n);
        trace.lock().unwrap().packets.clear();
        let from = Duration::from_nanos(
            (num(&c, "frames") - num(&c, "tail_frames")) * 1_000_000_000 / num(&c, "rate"),
        ) + Duration::from_millis(1);
        audio.seek(from);
        audio.play();
        transitions::ended(&audio, audio.generation());
        let packets = trace.lock().unwrap().packets.clone();
        assert_eq!(packets.first().unwrap().2, from, "{n}");
        let actual: Vec<_> = packets.iter().flat_map(|f| f.3.iter().copied()).collect();
        assert!(!actual.is_empty());
        assert!(actual.iter().all(|&b| b == 0));
    }
}
#[test]
fn the_short_video_witnesses_still_decode_and_audio_refusals_publish_nothing() {
    for c in cases()
        .into_iter()
        .filter(|c| c["file"].as_str().unwrap().contains("v0-trim0"))
    {
        let n = c["file"].as_str().unwrap();
        let stats = fvid_media::decode_video(&path(n)).unwrap();
        assert_eq!(stats.video_frames, num(&c, "video_frames"), "{n}");
        assert_eq!(stats.decode_errors, 0);
        for options in [
            fvid_media::CopyOptions {
                max_packet_bytes: 1,
                ..Default::default()
            },
            fvid_media::CopyOptions {
                max_packets: Some(0),
                ..Default::default()
            },
        ] {
            let mut out = Vec::new();
            let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(n)),
                &mut out,
                None,
                &options,
            )
            .unwrap_err();
            assert!(error.to_string().contains("packet"), "{n}: {error}");
            assert!(out.is_empty(), "{n}: refused decode published PCM");
        }
    }
}
#[test]
fn probes_report_the_same_complete_edited_duration_for_open_and_finite_controls() {
    for c in cases() {
        for key in ["file", "known"] {
            let n = c[key].as_str().unwrap();
            let info = fvid_media::owned_mp4_probe::probe_mp4(&path(n)).unwrap();
            assert_eq!(
                info.streams[1].duration,
                Some(num(&c, "edit_media_ticks") as i64),
                "{n}"
            );
        }
    }
}
#[test]
fn normalization_preserves_the_declared_tail_with_resampling_and_rematrixing() {
    for c in cases()
        .into_iter()
        .filter(|c| c["file"].as_str().unwrap().contains("v0-trim0"))
    {
        let rate = num(&c, "rate") as u32 * 2;
        let channels = if num(&c, "channels") == 1 { 2 } else { 1 };
        let mut control: Option<Vec<u8>> = None;
        for key in ["known", "file"] {
            let n = c[key].as_str().unwrap();
            let dest = std::env::temp_dir().join(format!(
                "fvid-open-edit-tail-{}-{n}.wav",
                std::process::id()
            ));
            assert!(!dest.exists());
            let result = fvid_media::decode_audio_transformed(
                &path(n),
                &dest,
                fvid_media::AudioDecodeTransform {
                    sample_rate: Some(rate as i32),
                    channels: Some(channels),
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
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&data), None).unwrap();
            assert_eq!((info.sample_rate, info.channels), (rate, channels as u16));
            let pcm = &data[info.data_offset as usize
                ..(info.data_offset + u64::from(info.data_bytes)) as usize];
            assert!(info.float && matches!(info.bits_per_sample, 32 | 64));
            let scalar_bytes = usize::from(info.bits_per_sample / 8);
            assert_eq!(
                pcm.len(),
                num(&c, "frames") as usize * 2 * channels as usize * scalar_bytes,
                "{n}"
            );
            assert!(pcm.chunks_exact(scalar_bytes).any(|s| {
                let value = if scalar_bytes == 8 {
                    f64::from_le_bytes(s.try_into().unwrap())
                } else {
                    f64::from(f32::from_le_bytes(s.try_into().unwrap()))
                };
                value.abs() > 1e-5
            }));
            let tail =
                (num(&c, "tail_frames") as usize * 2 - 128) * channels as usize * scalar_bytes;
            assert!(
                pcm[pcm.len() - tail..].iter().all(|&b| b == 0),
                "{n}: final declared tail must be silent"
            );
            if let Some(expected) = control.as_ref() {
                assert_eq!(pcm, expected.as_slice(), "{n}: normalized control PCM");
            } else {
                control = Some(pcm.to_vec());
            }
        }
    }
}
#[test]
fn a_declared_silent_tail_does_not_hide_an_invalid_final_encoded_packet() {
    for n in ["aac-invalid-last.mp4", "alac-invalid-last.mp4"] {
        let mut reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(n)), Default::default())
                .unwrap();
        assert_eq!(reader.tracks()[1].samples.len(), 3);
        let mut packet = Vec::new();
        reader.read_packet(1, 2, &mut packet).unwrap();
        assert_eq!(packet, [0xff]);
        let mut pcm = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(n)),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap_err();
        let expected = if n.starts_with("aac") {
            "AAC block has no configured audio element"
        } else {
            "ALAC frame does not hold every channel"
        };
        assert_eq!(error.to_string(), expected, "{n}: specific codec refusal");
    }
}
#[test]
fn silence_is_limited_to_declared_source_time_and_does_not_make_overlong_edits_playable() {
    for codec in ["aac", "pcm", "alac"] {
        let n = format!("{codec}-outside-tail.mp4");
        let reader =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(bytes(&n)), Default::default())
                .unwrap();
        let track = &reader.tracks()[1];
        assert!(
            u128::from(track.edits[0].duration) * u128::from(track.timescale)
                > u128::from(track.duration) * u128::from(reader.movie_timescale())
        );
        let mut pcm = Vec::new();
        let error = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(&n)),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("audio edit extends outside available samples"),
            "{n}: {error}"
        );
        let dest = std::env::temp_dir().join(format!(
            "fvid-open-edit-overlong-{}-{codec}.wav",
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
        assert!(result.is_err(), "{n}: overlong export accepted");
        assert!(!published, "{n}: overlong export published output");
        #[cfg(feature = "player")]
        {
            use fvid::{
                audio_thread::{AudioEvent, AudioPlayback},
                playback_mp4_audio::Mp4AudioReader,
            };
            use std::{
                sync::{Arc, Mutex},
                time::{Duration, Instant},
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
                assert!(Instant::now() < until, "{n}: worker refusal timeout");
                while let Some(event) = audio.poll() {
                    match event {
                        AudioEvent::Error(error) => {
                            assert!(
                                error
                                    .to_string()
                                    .contains("MP4 audio edit extends beyond source packets"),
                                "{n}: {error}"
                            );
                            break 'wait;
                        }
                        AudioEvent::Ended(_) => panic!("{n}: unbacked source time silently played"),
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
