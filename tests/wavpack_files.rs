#![cfg(feature = "player")]
use fvid::{
    audio::{AudioDecode, AudioStream},
    playback_wavpack::{Limits, WavpackAudioReader},
};
use std::{
    io::{Cursor, Read, Seek, SeekFrom},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
#[path = "support/audio_capture.rs"]
mod audio_capture;
fn read(n: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors/wavpack-files")
            .join(n),
    )
    .unwrap()
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn open(c: &serde_json::Value, paired: bool) -> WavpackAudioReader {
    let main = Cursor::new(read(c["file"].as_str().unwrap()));
    if paired && let Some(correction) = c["correction"].as_str() {
        WavpackAudioReader::with_correction(main, Cursor::new(read(correction)), Default::default())
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
    } else {
        WavpackAudioReader::open(main, Default::default())
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
    }
}
fn drain(stream: &mut WavpackAudioReader, decoder: &mut dyn AudioDecode) -> Vec<u8> {
    let mut pcm = Vec::new();
    while let Some(p) = stream.next_packet().unwrap() {
        let frame = decoder
            .decode_packet(&p.data, p.pts, p.duration as u64)
            .unwrap()
            .unwrap();
        if let Some(p) = stream
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            pcm.extend(p.data);
        }
    }
    stream.validate_eof().unwrap();
    pcm
}
#[test]
fn complete_native_files_decode_exactly_and_preserve_rewind_seek_and_duration() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 1317);
    for c in m["cases"].as_array().unwrap() {
        let count = c["samples"].as_u64().unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let channels = c["channels"].as_u64().unwrap() as u16;
        for paired in [false, true] {
            if paired && c["correction"].is_null() {
                continue;
            }
            let expected = read(c[if paired { "oracle" } else { "lossy" }].as_str().unwrap());
            let mut stream = open(c, paired);
            assert_eq!(
                (stream.sample_rate(), stream.timescale(), stream.channels()),
                (rate, rate, channels),
                "{}",
                c["name"]
            );
            assert_eq!(stream.indexed_frames(), 1);
            assert!(!stream.fully_indexed());
            if c["unknown"].as_bool() == Some(true) {
                assert_eq!(stream.duration(), None);
            } else {
                assert_eq!(
                    stream.duration(),
                    Some(Duration::new(
                        count / u64::from(rate),
                        ((count % u64::from(rate)) * 1_000_000_000 / u64::from(rate)) as u32
                    ))
                );
            }
            let mut decoder = stream.make_decoder().unwrap();
            assert!(
                drain(&mut stream, &mut *decoder) == expected,
                "{} paired={paired} initial",
                c["name"]
            );
            assert!(stream.fully_indexed());
            let duration = Duration::new(
                count / u64::from(rate),
                ((count % u64::from(rate)) * 1_000_000_000 / u64::from(rate)) as u32,
            );
            assert_eq!(stream.duration(), Some(duration));
            stream.rewind();
            decoder.reset();
            assert!(
                drain(&mut stream, &mut *decoder) == expected,
                "{} rewind",
                c["name"]
            );
            let target = c["frames"][0]["samples"].as_u64().unwrap()
                + c["frames"][1]["samples"].as_u64().unwrap() / 2;
            assert_eq!(stream.seek_to(target as i64), target as i64);
            decoder.reset();
            assert!(
                drain(&mut stream, &mut *decoder)
                    == expected[target as usize * usize::from(channels) * 4..],
                "{} seek paired={paired}",
                c["name"]
            );
            assert_eq!(stream.seek_to(i64::MAX), count as i64);
            decoder.reset();
            assert!(stream.next_packet().unwrap().is_none());
        }
    }
}
struct Observed {
    inner: Cursor<Vec<u8>>,
    reads: Arc<AtomicUsize>,
}
impl Read for Observed {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(out)?;
        self.reads.fetch_add(n, Ordering::Relaxed);
        Ok(n)
    }
}
impl Seek for Observed {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}
#[test]
fn open_and_seek_only_materialize_requested_sample_frames() {
    let m = manifest();
    let c = &m["cases"][0];
    let count = Arc::new(AtomicUsize::new(0));
    let source = Observed {
        inner: Cursor::new(read(c["file"].as_str().unwrap())),
        reads: count.clone(),
    };
    let mut stream = WavpackAudioReader::open(source, Default::default()).unwrap();
    let first = c["frames"][0]["length"].as_u64().unwrap() as usize;
    assert_eq!(count.load(Ordering::Relaxed), first + 42);
    stream.seek_to(c["samples"].as_i64().unwrap());
    assert_eq!(count.load(Ordering::Relaxed), first + 42 + 64);
    assert!(stream.next_packet().unwrap().is_none());
}
#[test]
fn malformed_native_files_and_correction_pairs_never_fall_back_or_skip_errors() {
    let m = manifest();
    for c in m["invalid"].as_array().unwrap() {
        let message = c["message"].as_str().unwrap();
        let error = match WavpackAudioReader::open(
            Cursor::new(read(c["file"].as_str().unwrap())),
            Default::default(),
        ) {
            Err(e) => e.to_string(),
            Ok(mut s) => loop {
                match s.next_packet() {
                    Err(e) => break e.to_string(),
                    Ok(Some(_)) => {}
                    Ok(None) => panic!("{} accepted", c["file"]),
                }
            },
        };
        assert!(error.contains(message), "{}: {error}", c["file"]);
    }
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == m["pair_case"])
        .unwrap();
    let error = WavpackAudioReader::with_correction(
        Cursor::new(read(c["file"].as_str().unwrap())),
        Cursor::new(read("wrong-pair.wvc")),
        Default::default(),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("correction headers disagree"), "{error}");
    for (file, message) in [
        ("short-pair.wvc", "correction ends before"),
        ("extra-pair.wvc", "correction contains extra"),
    ] {
        let mut stream = WavpackAudioReader::with_correction(
            Cursor::new(read(c["file"].as_str().unwrap())),
            Cursor::new(read(file)),
            Default::default(),
        )
        .unwrap();
        let error = loop {
            match stream.next_packet() {
                Err(e) => break e.to_string(),
                Ok(Some(_)) => {}
                Ok(None) => panic!("{file} accepted"),
            }
        };
        assert!(error.contains(message), "{file}: {error}");
        assert!(stream.next_packet().is_err());
        stream.rewind();
        assert!(stream.validate_eof().is_err());
        assert!(stream.next_packet().is_err());
    }
}
#[test]
fn explicit_input_packet_and_index_limits_are_enforced() {
    let m = manifest();
    let c = &m["cases"][0];
    let bytes = read(c["file"].as_str().unwrap());
    let first = c["frames"][0]["length"].as_u64().unwrap() as usize;
    for (limits, message) in [
        (
            Limits {
                file_bytes: bytes.len() as u64 - 1,
                ..Default::default()
            },
            "file exceeds",
        ),
        (
            Limits {
                packet_bytes: first - 1,
                ..Default::default()
            },
            "frame exceeds packet",
        ),
        (
            Limits {
                index_entries: 0,
                ..Default::default()
            },
            "index exceeds",
        ),
    ] {
        let error = WavpackAudioReader::open(Cursor::new(bytes.clone()), limits)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(message), "{error}");
    }
    let mut stream = WavpackAudioReader::open(
        Cursor::new(bytes),
        Limits {
            index_entries: 1,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(stream.next_packet().unwrap().is_some());
    let error = stream.next_packet().err().unwrap().to_string();
    assert!(error.contains("index exceeds"), "{error}");
    let c = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == m["pair_case"])
        .unwrap();
    let main = read(c["file"].as_str().unwrap());
    let extra = read(c["correction"].as_str().unwrap());
    let a = u32::from_le_bytes(main[4..8].try_into().unwrap()) as usize + 8;
    let b = u32::from_le_bytes(extra[4..8].try_into().unwrap()) as usize + 8;
    let error = WavpackAudioReader::with_correction(
        Cursor::new(main),
        Cursor::new(extra),
        Limits {
            packet_bytes: a + b - 1,
            ..Default::default()
        },
    )
    .err()
    .unwrap()
    .to_string();
    assert!(error.contains("paired frame exceeds"), "{error}");
}

#[test]
fn native_audio_worker_preserves_pcm_rewind_and_partial_seek() {
    let m = manifest();
    for name in [
        "false-d8-t1-neg0-none",
        "correction-multichannel-bitrate-delta-new-cross1",
        "float-hybrid-stereo-bitrate-delta-optimized-e127",
        "dsd-high-six-p3-noise",
    ] {
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == name)
            .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let pcm = read(c["oracle"].as_str().unwrap());
        let stride = channels * 4;
        let n = c["frames"][0]["samples"].as_u64().unwrap() as usize;
        let expected: Vec<_> = (0..3)
            .map(|i| {
                (
                    Duration::from_nanos(i as u64 * n as u64 * 1_000_000_000 / rate),
                    pcm[i * n * stride..(i + 1) * n * stride].to_vec(),
                )
            })
            .collect();
        let target_samples = n + n / 2;
        let target = (target_samples as u64 * 1_000_000_000).div_ceil(rate);
        let after = vec![
            (
                Duration::from_nanos(target_samples as u64 * 1_000_000_000 / rate),
                pcm[target_samples * stride..2 * n * stride].to_vec(),
            ),
            expected[2].clone(),
        ];
        audio_capture::worker_check_with_seek_expected(
            Box::new(open(c, true)),
            &expected,
            Duration::from_nanos(target),
            &after,
        );
    }
}
