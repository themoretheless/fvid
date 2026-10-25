#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::AudioStream,
    codec::eac3_decoder::{Eac3Decoder, StereoDownmixMode as Mode, StereoDownmixPolicy as Policy},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/eac3-player-stereo")
}
fn reader(name: &str, kind: &str) -> Box<dyn AudioStream> {
    let bytes = std::fs::read(root().join(format!("{name}.{kind}"))).unwrap();
    match kind {
        "ec3" => Box::new(
            fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn expected(name: &str, kind: &str, policy: Policy) -> Vec<(Duration, Vec<u8>)> {
    let bytes = std::fs::read(root().join(format!("{name}.ec3"))).unwrap();
    let mut decoder = if kind == "ec3" {
        Eac3Decoder::new_stereo(&[], 48000)
    } else {
        Eac3Decoder::for_container_stereo(&[], 48000)
    }
    .unwrap()
    .with_stereo_downmix(policy);
    let pcm = decoder
        .decode_complete(&bytes, 0, 2304)
        .unwrap()
        .unwrap()
        .data;
    let mut offset = 0;
    let mut result = Vec::new();
    for frames in if kind == "ec3" {
        [512, 768, 768]
    } else {
        [768, 768, 768]
    } {
        result.push((
            Duration::from_nanos(offset as u64 * 1_000_000_000 / 48000),
            pcm[offset * 8..(offset + frames) * 8].to_vec(),
        ));
        offset += frames;
    }
    assert_eq!(offset * 8, pcm.len());
    result
}
#[test]
fn real_readers_negotiate_stereo_without_losing_native_metadata_or_priming() {
    for name in ["l1-f1-c0-s7", "l7-f1-c0-s1", "l5-f0-c3-s4"] {
        for kind in ["ec3", "mkv", "mp4"] {
            let mut source = reader(name, kind);
            let channels = source.channels();
            let mask = source.speaker_mask().unwrap();
            let duration = source.duration();
            source.set_eac3_stereo_output(true).unwrap();
            assert_eq!(source.channels(), 2);
            assert_eq!(source.speaker_mask().unwrap(), Some(3));
            assert_eq!(source.duration(), duration);
            let mut decoder = source.make_decoder().unwrap();
            let mut data = Vec::new();
            while let Some(p) = source.next_packet().unwrap() {
                if let Some(p) = decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                {
                    data.extend(p.packet.data);
                }
            }
            while let Some(p) = decoder.finish_packet().unwrap() {
                data.extend(p.packet.data);
            }
            assert_eq!(
                data,
                expected(name, kind, Policy::default())
                    .into_iter()
                    .flat_map(|(_, p)| p)
                    .collect::<Vec<_>>(),
                "{name} {kind}"
            );
            source.set_eac3_stereo_output(false).unwrap();
            assert_eq!(source.channels(), channels);
            assert_eq!(source.speaker_mask().unwrap(), mask);
        }
    }
}
#[test]
fn actual_readers_and_worker_apply_stereo_metadata_at_eof_rewind_and_partial_seek() {
    for name in ["l1-f1-c0-s7", "l7-f1-c0-s1", "l5-f0-c3-s4"] {
        for kind in ["ec3", "mkv", "mp4"] {
            for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
                let policy = Policy {
                    mode,
                    headroom: true,
                };
                let full = expected(name, kind, policy);
                let mut offset = 0usize;
                let mut sought = Vec::new();
                for (_, p) in &full {
                    let frames = p.len() / 8;
                    let first = offset.max(960);
                    if first < offset + frames {
                        sought.push((
                            Duration::from_nanos(first as u64 * 1_000_000_000 / 48000),
                            p[(first - offset) * 8..].to_vec(),
                        ));
                    }
                    offset += frames;
                }
                capture::worker_check_with_dolby_policy(
                    reader(name, kind),
                    &full,
                    Duration::from_millis(20),
                    &sought,
                    Some(3),
                    fvid::audio_thread::DolbyPlaybackPolicy {
                        eac3_stereo_output: true,
                        eac3_stereo_downmix: policy,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[test]
fn expanded_presentations_refuse_stereo_without_mutating_native_layout() {
    let base = root()
        .parent()
        .unwrap()
        .join("eac3-expanded-coupling/7.1-aht-cpl-b0-p0-m5-None-unity");
    for kind in ["ec3", "mkv", "mp4"] {
        let bytes = std::fs::read(PathBuf::from(format!("{}.{kind}", base.display()))).unwrap();
        let mut source: Box<dyn AudioStream> = match kind {
            "ec3" => Box::new(
                fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(bytes), Default::default())
                    .unwrap(),
            ),
            "mkv" => Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            ),
            _ => Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(bytes),
                    Default::default(),
                )
                .unwrap(),
            ),
        };
        let channels = source.channels();
        assert!(channels > 6);
        let mask = source.speaker_mask().unwrap();
        let error = source.set_eac3_stereo_output(true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("extended dependent presentation stereo")
        );
        assert_eq!(source.channels(), channels);
        assert_eq!(source.speaker_mask().unwrap(), mask);
        assert!(source.make_decoder().is_ok());
    }
}

#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn mp4_configuration_changes_and_decoder_replacement_retain_stereo_output() {
    use fvid::audio_thread::{AudioPlayback, DolbyPlaybackPolicy};
    use std::sync::{Arc, Mutex, atomic::AtomicUsize};
    let base = root().parent().unwrap().join("mp4-audio-reconfiguration");
    let open = |name: &str| {
        fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(std::fs::read(base.join(name)).unwrap()),
            Default::default(),
        )
        .unwrap()
    };
    for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
        let policy = Policy {
            mode,
            headroom: true,
        };
        let mut wanted = Vec::new();
        for (identity, start) in [(1, 0), (2, 96000), (1, 240000)] {
            let mut control = open(&format!("eac3-control-{identity}.mp4"));
            control.set_eac3_stereo_output(true).unwrap();
            let mut decoder = control.make_decoder().unwrap();
            decoder.set_eac3_stereo_downmix(policy);
            let frames = transitions::collect_with_decoder(&mut control, decoder);
            assert!(frames.iter().all(|f| f.1 == 2));
            wanted.extend(
                frames
                    .into_iter()
                    .map(|(r, c, t, b)| (r, c, t + Duration::from_micros(start), b)),
            );
        }
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let sink = trace.clone();
        let mut audio = AudioPlayback::start_with_dolby_policy(
            Box::new(open("eac3.mp4")),
            DolbyPlaybackPolicy {
                eac3_stereo_output: true,
                eac3_stereo_downmix: policy,
                ..Default::default()
            },
            move || {
                Box::new(transitions::Capture {
                    trace: sink,
                    spec: None,
                    at: Duration::ZERO,
                    remaining: AtomicUsize::new(0),
                    draining: false,
                    fail_next: None,
                    gate: None,
                    paused: None,
                })
            },
        );
        audio.play();
        transitions::ended(&audio, 0);
        transitions::compare(&trace.lock().unwrap().packets, &wanted);
        audio.rewind();
        transitions::ended(&audio, audio.generation());
        transitions::compare(&trace.lock().unwrap().packets, &wanted);
        audio.seek(Duration::from_millis(120));
        transitions::ended(&audio, audio.generation());
        let mut suffix = wanted.clone();
        suffix.drain(..3);
        suffix[0].2 = Duration::from_millis(120);
        suffix[0].3 = suffix[0].3[768 * 8..].to_vec();
        transitions::compare(&trace.lock().unwrap().packets, &suffix);
        assert!(
            trace
                .lock()
                .unwrap()
                .configs
                .iter()
                .all(|s| s.channels == 2)
        );
    }
}
