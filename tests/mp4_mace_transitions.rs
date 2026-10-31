#![cfg(feature = "player")]
#[path = "support/audio_transitions.rs"]
mod transitions;
use fvid::playback_mp4_audio::Mp4AudioReader;
use std::{io::Cursor, time::Duration};
use transitions::{Frames, collect, compare};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/mp4-mace-transitions/{name}.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn open(name: &str) -> Mp4AudioReader<Cursor<Vec<u8>>> {
    Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default()).unwrap()
}
fn sequence(selection: &[usize]) -> Frames {
    let mut out = Vec::new();
    let mut at = Duration::ZERO;
    for &i in selection {
        let frames = collect(&mut open(&format!("control-{i}")));
        for (rate, ch, time, data) in frames {
            out.push((rate, ch, at + time, data));
        }
        at += Duration::from_micros(if i == 1 { 15000 } else { 30000 });
    }
    out
}
fn expected() -> Frames {
    sequence(&[0, 1, 2, 0])
}
#[test]
fn mace_description_changes_keep_block_geometry_and_predictor_state() {
    let wanted = expected();
    for name in [
        "changes",
        "start-third",
        "fragmented",
        "fragmented-third",
        "hybrid",
    ] {
        let mut stream = open(name);
        compare(&collect(&mut stream), &wanted);
        stream.rewind();
        compare(&collect(&mut stream), &wanted);
    }
}
fn suffix(frames: &Frames, target: Duration) -> Frames {
    frames
        .iter()
        .filter_map(|(rate, ch, at, data)| {
            let skip = target
                .saturating_sub(*at)
                .as_nanos()
                .saturating_mul(u128::from(*rate))
                .div_ceil(1_000_000_000) as usize;
            let stride = usize::from(*ch) * 4;
            if skip >= data.len() / stride {
                None
            } else {
                Some((
                    *rate,
                    *ch,
                    *at + Duration::from_nanos((skip as u64) * 1_000_000_000 / u64::from(*rate)),
                    data[skip * stride..].to_vec(),
                ))
            }
        })
        .collect()
}
fn edit_source(source: Frames) -> Frames {
    let mut out = Vec::new();
    for start in [0, 70] {
        out.push((8000, 1, Duration::from_millis(start), vec![0; 40 * 4]));
        for (rate, ch, at, data) in suffix(&source, Duration::from_millis(10)) {
            if at >= Duration::from_millis(75) {
                continue;
            }
            let frames = ((Duration::from_millis(75) - at).as_nanos() * u128::from(rate))
                .div_ceil(1_000_000_000) as usize;
            let end = data.len().min(frames * usize::from(ch) * 4);
            out.push((
                rate,
                ch,
                at - Duration::from_millis(10) + Duration::from_millis(start + 5),
                data[..end].to_vec(),
            ));
        }
    }
    out
}
fn edited() -> Frames {
    edit_source(expected())
}
#[test]
fn mace_workers_and_seek_preserve_predictors_across_changes_and_repeated_edits() {
    use fvid::audio_thread::AudioPlayback;
    use std::sync::{Arc, Mutex};
    use transitions::{Capture, Trace, ended};
    for (name, wanted) in [
        ("changes", expected()),
        ("mixed", sequence(&[0, 3, 1, 2, 0])),
        ("mixed-fragmented", sequence(&[0, 3, 1, 2, 0])),
        ("mixed-hybrid", sequence(&[0, 3, 1, 2, 0])),
        (
            "mixed-fragmented-edits",
            edit_source(sequence(&[0, 3, 1, 2, 0])),
        ),
        ("mixed-third", sequence(&[0, 3, 1, 2, 0])),
        ("mixed-edits", edit_source(sequence(&[0, 3, 1, 2, 0]))),
        ("start-third", expected()),
        ("edits", edited()),
        ("fragmented", expected()),
        ("fragmented-third", expected()),
        ("hybrid", expected()),
        ("fragmented-edits", edited()),
        ("hybrid-edits", edited()),
        (
            "negative",
            expected()
                .into_iter()
                .skip(1)
                .map(|(r, c, t, d)| (r, c, t - Duration::from_micros(750), d))
                .collect(),
        ),
    ] {
        let trace = Arc::new(Mutex::new(Trace::default()));
        let shared = trace.clone();
        let mut audio =
            AudioPlayback::start(Box::new(open(name)), move || Box::new(Capture::new(shared)));
        audio.play();
        ended(&audio, 0);
        compare(&trace.lock().unwrap().packets, &wanted);
        trace.lock().unwrap().packets.clear();
        audio.rewind();
        audio.play();
        ended(&audio, audio.generation());
        compare(&trace.lock().unwrap().packets, &wanted);
        for ms in [3, 10, 29, 30, 44, 45, 74, 75, 99, 105, 139, 200] {
            trace.lock().unwrap().packets.clear();
            audio.seek(Duration::from_millis(ms));
            ended(&audio, audio.generation());
            compare(
                &trace.lock().unwrap().packets,
                &suffix(&wanted, Duration::from_millis(ms)),
            );
        }
    }
}

#[test]
fn mace_index_uses_selected_width_and_coded_sample_timing() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    let movie = Mp4Reader::open_indexed(Cursor::new(bytes("changes")), Limits::default()).unwrap();
    let track = movie
        .tracks()
        .iter()
        .find(|t| t.handler == *b"soun")
        .unwrap();
    assert_eq!(track.samples.len(), 160);
    for (span, (size, delta, first)) in [(2, 36, 0), (2, 18, 1440), (4, 36, 2160), (2, 36, 3600)]
        .into_iter()
        .enumerate()
    {
        let start = track.samples.get(span * 40).unwrap().offset;
        for i in 0..40 {
            let sample = track.samples.get(span * 40 + i).unwrap();
            assert_eq!(
                (sample.size, sample.duration, sample.dts),
                (size, delta, first + i as u64 * u64::from(delta))
            );
            assert_eq!(sample.offset, start + i as u64 * u64::from(size));
            assert_eq!(sample.description_index, [1, 2, 3, 1][span]);
        }
    }
    let error = Mp4Reader::open_indexed(
        Cursor::new(bytes("changes")),
        Limits {
            samples: 159,
            ..Default::default()
        },
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("sample count exceeds budget"), "{error}");
}
#[test]
fn mace_invalid_chunk_tables_fail_for_the_specific_structural_reason() {
    for (name, expected) in [
        (
            "bad-composition-block",
            "MACE composition run must cover whole blocks",
        ),
        ("bad-partial-block", "invalid sample-to-chunk mapping"),
        ("bad-outside-mdat", "sample chunk outside mdat"),
    ] {
        let error = Mp4AudioReader::open(Cursor::new(bytes(name)), Default::default())
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
}
#[test]
fn mace_movies_retain_exact_owned_video_planes_and_clocks() {
    use fvid::playback_native::{NativeReader, RawFrame};
    for name in [
        "changes",
        "start-third",
        "edits",
        "control-0",
        "control-1",
        "control-2",
        "negative",
        "fragmented",
        "fragmented-third",
        "hybrid",
        "fragmented-edits",
        "hybrid-edits",
        "mixed",
        "mixed-third",
        "mixed-edits",
        "mixed-fragmented",
        "mixed-hybrid",
        "mixed-fragmented-edits",
        "control-3",
    ] {
        let mut video = NativeReader::software(Cursor::new(bytes(name)), 32 << 20).unwrap();
        for i in 0..6 {
            match video.read_frame_raw().unwrap().unwrap() {
                RawFrame::Planar(p) => {
                    assert_eq!((p.depth, p.frame.width, p.frame.height), (8, 16, 16));
                    assert_eq!(p.frame.data, vec![128; 384]);
                }
                _ => panic!("owned VP9 must retain exact planes"),
            }
            assert_eq!(video.frame_interval(), Some((i, i + 1, 50)));
        }
        assert!(video.read_frame_raw().unwrap().is_none());
    }
}

#[test]
fn mixed_mace_pcm_units_preserve_exact_samples_and_index_cost() {
    use fvid::container::mp4::{Limits, Mp4Reader};
    for name in ["mixed", "mixed-third", "mixed-fragmented", "mixed-hybrid"] {
        compare(&collect(&mut open(name)), &sequence(&[0, 3, 1, 2, 0]));
        let reader = Mp4Reader::open_indexed(Cursor::new(bytes(name)), Limits::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"soun")
            .unwrap();
        assert_eq!(track.samples.len(), 400);
        assert_eq!(track.duration, 6480);
        for (first, size, duration, dts) in [
            (0, 2, 36, 0),
            (40, 2, 6, 1440),
            (280, 2, 18, 2880),
            (320, 4, 36, 3600),
            (360, 2, 36, 5040),
        ] {
            let s = track.samples.get(first).unwrap();
            assert_eq!((s.size, s.duration, s.dts), (size, duration, dts));
        }
    }
    let err = Mp4Reader::open_indexed(
        Cursor::new(bytes("mixed")),
        Limits {
            samples: 399,
            ..Default::default()
        },
    )
    .err()
    .unwrap()
    .to_string();
    assert!(err.contains("sample count exceeds budget"), "{err}");
}
