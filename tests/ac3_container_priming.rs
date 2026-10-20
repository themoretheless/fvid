#![cfg(feature = "player")]
use fvid::audio::{AudioStep, AudioStream};
use std::{io::Cursor, path::Path, time::Duration};
#[path = "support/audio_capture.rs"]
mod capture;
fn open(ext: &str, bytes: Vec<u8>) -> Box<dyn AudioStream> {
    match ext {
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
#[test]
fn container_priming_and_tail_windows_do_not_double_trim_native_ac3_or_add_gaps() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-container-priming");
    let mut failures = Vec::new();
    for ext in ["mkv", "mp4"] {
        let bytes = std::fs::read(base.join(format!("tone.{ext}"))).unwrap();
        let reference = std::fs::read(base.join(format!("tone-{ext}.f32le"))).unwrap();
        assert_eq!(reference.len() / 4, 5760);
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut pictures = 0;
        while video.read_frame_raw().unwrap().is_some() {
            pictures += 1;
        }
        assert_eq!(pictures, 6);
        let mut source = open(ext, bytes.clone());
        assert_eq!((source.sample_rate(), source.channels()), (48000, 1));
        let mut engine = source.make_decoder().unwrap();
        let mut intervals = Vec::new();
        let mut frames = 0u64;
        let mut pcm = Vec::new();
        let mut clock_bad = false;
        while let Some(step) = source.next_step().unwrap() {
            let packet = match step {
                AudioStep::Encoded(p) => {
                    let f = engine
                        .decode_packet(&p.data, p.pts, p.duration as u64)
                        .unwrap()
                        .unwrap();
                    source.present_decoded(f.packet, f.source_pts).unwrap()
                }
                AudioStep::Pcm(p) => Some(p),
                AudioStep::ResetDecoder => {
                    source.reset_decoder(&mut engine).unwrap();
                    None
                }
            };
            if let Some(packet) = packet {
                let at = packet.presentation_time();
                let ideal = Duration::from_nanos(frames * 1_000_000_000 / 48000);
                if at.abs_diff(ideal) > Duration::from_nanos(1) {
                    clock_bad = true;
                }
                frames += packet.data.len() as u64 / 4;
                pcm.extend_from_slice(&packet.data);
                intervals.push((at, packet.data));
            }
        }
        source.validate_eof().unwrap();
        if frames != 5760 || clock_bad || source.duration() != Some(Duration::from_millis(120)) {
            failures.push(format!("{ext}: nativeframes{frames} versus reference5760, gapped={clock_bad}, duration{:?} instead120ms",source.duration()));
            continue;
        }
        let pairs = pcm
            .chunks_exact(4)
            .zip(reference.chunks_exact(4))
            .map(|(a, b)| {
                (
                    f32::from_le_bytes(a.try_into().unwrap()) as f64,
                    f32::from_le_bytes(b.try_into().unwrap()) as f64,
                )
            });
        let (mut error, mut power, mut dot, mut native) = (0., 0., 0., 0.);
        for (a, b) in pairs {
            error += (a - b) * (a - b);
            power += b * b;
            dot += a * b;
            native += a * a;
        }
        let relative = (error / power).sqrt();
        let correlation = dot / (power * native).sqrt();
        eprintln!("{ext}: relativeRMSerror={relative}, correlation={correlation}");
        assert!(
            relative < 0.01 && correlation > 0.9995,
            "{ext}: decoded/reference alignment"
        );
        capture::worker_check_with_seek(open(ext, bytes.clone()), &intervals, Some(Duration::ZERO));
        capture::worker_check_with_seek(open(ext, bytes.clone()), &intervals, Some(intervals[1].0));
        capture::worker_check_with_seek(
            open(ext, bytes),
            &intervals,
            Some(Duration::from_millis(120)),
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
}

fn windows(mut source: Box<dyn AudioStream>) -> Vec<(Duration, Vec<u8>)> {
    let mut engine = source.make_decoder().unwrap();
    let mut output = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        let packet = match step {
            AudioStep::Encoded(p) => {
                let f = engine
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap()
                    .unwrap();
                source.present_decoded(f.packet, f.source_pts).unwrap()
            }
            AudioStep::Pcm(p) => Some(p),
            AudioStep::ResetDecoder => {
                source.reset_decoder(&mut engine).unwrap();
                None
            }
        };
        if let Some(p) = packet {
            output.push((p.presentation_time(), p.data));
        }
    }
    source.validate_eof().unwrap();
    output
}
#[test]
fn ac3_mp4_silence_and_repeated_edits_keep_their_native_pcm_and_seek_windows() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/ac3-container-priming");
    let normal = windows(open("mp4", std::fs::read(base.join("tone.mp4")).unwrap()));
    let normal_pcm = normal
        .iter()
        .flat_map(|(_, data)| data.iter().copied())
        .collect::<Vec<_>>();
    let bytes = std::fs::read(base.join("gap-repeat.mp4")).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut pictures = 0;
    while video.read_frame_raw().unwrap().is_some() {
        pictures += 1;
    }
    assert_eq!(pictures, 6);
    let source = open("mp4", bytes.clone());
    assert_eq!(source.duration(), Some(Duration::from_millis(100)));
    let actual = windows(source);
    let mut expected = vec![0; 960 * 4];
    expected.extend_from_slice(&normal_pcm[..1920 * 4]);
    expected.extend_from_slice(&normal_pcm[..1920 * 4]);
    assert!(
        actual
            .iter()
            .flat_map(|(_, data)| data.iter().copied())
            .eq(expected),
        "gap/repeat PCM differs from exact native source ranges"
    );
    let mut count = 0u64;
    for (at, data) in &actual {
        assert!(
            at.abs_diff(Duration::from_nanos(count * 1_000_000_000 / 48000))
                <= Duration::from_nanos(1)
        );
        count += data.len() as u64 / 4;
    }
    assert_eq!(count, 4800);
    for target in [20, 60, 100] {
        capture::worker_check_with_seek(
            open("mp4", bytes.clone()),
            &actual,
            Some(Duration::from_millis(target)),
        );
    }
    let mut gap_suffix = vec![(Duration::from_millis(10), vec![0; 480 * 4])];
    gap_suffix.extend_from_slice(&actual[1..]);
    capture::worker_check_with_seek_expected(
        open("mp4", bytes),
        &actual,
        Duration::from_millis(10),
        &gap_suffix,
    );
}
