#![cfg(feature = "player")]
#[path = "support/audio_capture.rs"]
mod capture;
#[path = "support/audio_decode.rs"]
mod decode_support;
use decode_support::DecodeComplete;
use fvid::{
    audio::AudioStream,
    codec::ac3_decoder::{Ac3Decoder, StereoDownmixMode as Mode, StereoDownmixPolicy as Policy},
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
    time::Duration,
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/ac3-player-stereo")
}
fn rate(name: &str) -> u32 {
    if name == "single" { 32000 } else { 48000 }
}
fn reader(name: &str, kind: &str) -> Box<dyn AudioStream> {
    let raw = std::fs::read(root().join(format!("{name}.{kind}"))).unwrap();
    match kind {
        "ac3" => Box::new(
            fvid::playback_ac3::Ac3AudioReader::open(Cursor::new(raw), Default::default()).unwrap(),
        ),
        "mkv" => Box::new(
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(raw), Default::default())
                .unwrap(),
        ),
        "mp4" => Box::new(
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(raw), Default::default())
                .unwrap(),
        ),
        _ => unreachable!(),
    }
}
fn expected(name: &str, kind: &str, policy: Policy) -> Vec<(Duration, Vec<u8>)> {
    let raw = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
    let rate = rate(name);
    let mut d = if kind == "ac3" {
        Ac3Decoder::new_stereo(&[], rate)
    } else {
        Ac3Decoder::for_container_stereo(&[], rate)
    }
    .unwrap()
    .with_stereo_downmix(policy);
    let pcm = d.decode_complete(&raw, 0, 4608).unwrap().unwrap().data;
    let mut offset = 0;
    let mut result = Vec::new();
    for frames in if kind == "ac3" {
        [1280, 1536, 1536]
    } else {
        [1536, 1536, 1536]
    } {
        result.push((
            Duration::from_nanos(offset as u64 * 1_000_000_000 / u64::from(rate)),
            pcm[offset * 8..(offset + frames) * 8].to_vec(),
        ));
        offset += frames;
    }
    assert_eq!(pcm.len(), offset * 8);
    assert!(
        pcm.chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-6)
    );
    result
}
#[test]
fn real_ac3_readers_negotiate_lr_and_restore_native_geometry() {
    for name in ["51", "single", "c-lfe"] {
        for kind in ["ac3", "mkv", "mp4"] {
            let mut source = reader(name, kind);
            let channels = source.channels();
            let mask = source.speaker_mask().unwrap();
            let duration = source.duration();
            source.set_ac3_stereo_output(true).unwrap();
            assert_eq!(source.channels(), 2);
            assert_eq!(source.speaker_mask().unwrap(), Some(3));
            assert_eq!(source.duration(), duration);
            assert_eq!(
                source.stereo_router().unwrap().unwrap().coefficients(),
                &[[1., 0.], [0., 1.]]
            );
            source.set_ac3_stereo_output(false).unwrap();
            assert_eq!(source.channels(), channels);
            assert_eq!(source.speaker_mask().unwrap(), mask);
        }
    }
}
#[test]
fn actual_ac3_worker_keeps_bsi_stereo_at_eof_rewind_and_partial_seek() {
    for name in ["51", "single", "c-lfe"] {
        for kind in ["ac3", "mkv", "mp4"] {
            for mode in [Mode::LoRo, Mode::LtRt, Mode::Automatic] {
                let policy = Policy {
                    mode,
                    headroom: true,
                };
                let full = expected(name, kind, policy);
                let mut offset = 0;
                let mut sought = Vec::new();
                let seek = (u64::from(rate(name)) * 40 / 1000) as usize;
                for (_, p) in &full {
                    let end = offset + p.len() / 8;
                    let first = offset.max(seek);
                    if first < end {
                        sought.push((
                            Duration::from_nanos(
                                first as u64 * 1_000_000_000 / u64::from(rate(name)),
                            ),
                            p[(first - offset) * 8..].to_vec(),
                        ));
                    }
                    offset = end;
                }
                capture::worker_check_with_dolby_policy(
                    reader(name, kind),
                    &full,
                    Duration::from_millis(40),
                    &sought,
                    Some(3),
                    fvid::audio_thread::DolbyPlaybackPolicy {
                        ac3_stereo_output: true,
                    eac3_stereo_output: true,
                        ac3_stereo_downmix: policy,
                        ..Default::default()
                    },
                );
            }
        }
    }
}

#[allow(dead_code)]
#[path = "support/audio_transitions.rs"]
mod transitions;
#[test]
fn ac3_rate_layout_reconfiguration_restores_stereo_and_matrix_policy() {
    use fvid::audio_thread::{AudioPlayback, DolbyPlaybackPolicy};
    use std::sync::{Arc, Mutex, atomic::AtomicUsize};
    let open = || {
        fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(std::fs::read(root().join("changing.mp4")).unwrap()),
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
        for (name, frame, start) in [("51", 0, 0), ("single", 1, 32000), ("51", 2, 80000)] {
            let raw = std::fs::read(root().join(format!("{name}.ac3"))).unwrap();
            let size = if name == "single" { 3840 } else { 2560 };
            let mut d = Ac3Decoder::for_container_stereo(&[], rate(name))
                .unwrap()
                .with_stereo_downmix(policy);
            let data = d
                .decode_complete(&raw[frame * size..(frame + 1) * size], 0, 1536)
                .unwrap()
                .unwrap()
                .data;
            assert_eq!(data.len(), 1536 * 8);
            wanted.push((rate(name), 2, Duration::from_micros(start), data));
        }
        let trace = Arc::new(Mutex::new(transitions::Trace::default()));
        let sink = trace.clone();
        let mut audio = AudioPlayback::start_with_dolby_policy(
            Box::new(open()),
            DolbyPlaybackPolicy {
                ac3_stereo_output: true,
                    eac3_stereo_output: true,
                ac3_stereo_downmix: policy,
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
        audio.seek(Duration::from_millis(40));
        transitions::ended(&audio, audio.generation());
        let mut suffix = wanted.clone();
        suffix.remove(0);
        suffix[0].2 = Duration::from_millis(40);
        suffix[0].3 = suffix[0].3[256 * 8..].to_vec();
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
