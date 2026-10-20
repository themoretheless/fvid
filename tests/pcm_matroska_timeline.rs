#![cfg(feature = "player")]
use fvid::audio::AudioStream;
use std::{io::Cursor, path::Path};
fn value(n: usize, c: usize) -> f32 {
    ((n * (2 * c + 1)) % 127) as f32 / 128. - 63. / 128.
}
#[test]
fn pcm_seek_is_sample_exact_and_preserves_original_container_windows() {
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/pcm-timeline");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(base.join("manifest.json")).unwrap()).unwrap();
    let mut failures = Vec::new();
    for c in manifest["cases"].as_array().unwrap() {
        if c["invalid"].as_bool().unwrap_or(false) {
            continue;
        }
        let name = c["file"].as_str().unwrap();
        let bytes = std::fs::read(base.join(name)).unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let offset = c["offset"].as_u64().unwrap() as usize;
        let gap = c["gap"].as_u64().unwrap() as usize;
        let head_packet = c["head_packet"].as_u64().unwrap() as usize;
        let head = c["head"].as_u64().unwrap() as usize;
        let tail = c["tail"].as_u64().unwrap() as usize;
        let delay = c["delay"].as_u64().unwrap() as i128;
        let mut video =
            fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
                .unwrap();
        let mut frames = 0;
        while video.read_frame_raw().unwrap().is_some() {
            frames += 1;
        }
        assert_eq!(frames, 6);
        for target in [0, 1_000_000, 6_000_000, 51_000_000, 51_000_001] {
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(bytes.clone()),
                Default::default(),
            )
            .unwrap();
            let landed = reader.seek_to(target);
            if landed != target {
                failures.push(format!("{name}: requested {target}, landed {landed}"));
                continue;
            }
            let mut decoder = reader.make_decoder().unwrap();
            let requested = (target as u128 * 48000).div_ceil(1_000_000_000) as usize;
            let mut expected = Vec::new();
            let mut times = Vec::new();
            for i in 0..6 {
                let at = (i * 64 + offset + if i >= 3 { gap } else { 0 }) as i128 - delay;
                let first = ((requested as i128 - at).clamp(0, 64) as usize)
                    .max(if i == head_packet { head } else { 0 });
                let last = if i == 5 { 64 - tail } else { 64 };
                if first < last {
                    times.push((at + first as i128) as u64 * 1_000_000_000 / 48000);
                    for n in i * 64 + first..i * 64 + last {
                        for channel in 0..channels {
                            expected.push(value(n, channel));
                        }
                    }
                }
            }
            let mut actual = Vec::new();
            let mut actual_times = Vec::new();
            while let Some(p) = reader.next_packet().unwrap() {
                let f = decoder
                    .decode_packet(&p.data, p.pts, p.duration.max(0) as u64)
                    .unwrap()
                    .unwrap();
                if let Some(out) = reader.present_decoded(f.packet, f.source_pts).unwrap() {
                    actual_times.push(out.pts);
                    actual.extend(
                        out.data
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes(b.try_into().unwrap())),
                    );
                }
            }
            assert_eq!(actual_times, times, "{name}/{target}");
            assert_eq!(actual, expected, "{name}/{target}");
            assert_eq!(
                reader.duration(),
                Some(std::time::Duration::from_nanos(
                    c["samples"].as_u64().unwrap() * 1_000_000_000 / 48000
                )),
                "{name}"
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn indexed_pcm_seek_uses_packet_sizes_without_reading_preroll_payloads() {
    use std::io::{Read, Seek, SeekFrom};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    struct Meter {
        input: Cursor<Vec<u8>>,
        ranges: Vec<std::ops::Range<usize>>,
        reads: Arc<AtomicUsize>,
    }
    impl Read for Meter {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let begin = self.input.position() as usize;
            let n = self.input.read(buf)?;
            let range = begin..begin + n;
            for r in &self.ranges {
                let overlap = range
                    .end
                    .min(r.end)
                    .saturating_sub(range.start.max(r.start));
                self.reads.fetch_add(overlap, Ordering::Relaxed);
            }
            Ok(n)
        }
    }
    impl Seek for Meter {
        fn seek(&mut self, p: SeekFrom) -> std::io::Result<u64> {
            self.input.seek(p)
        }
    }
    let base =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/pcm-timeline");
    let bytes = std::fs::read(base.join("scaled-gap-f64-1.mkv")).unwrap();
    let mut metadata =
        fvid::container::webm::WebmReader::open(Cursor::new(bytes.clone()), Default::default())
            .unwrap();
    metadata.scan_all().unwrap();
    let ranges = metadata
        .packets
        .iter()
        .filter(|p| p.track == 2)
        .map(|p| p.offset as usize..p.offset as usize + p.size)
        .collect();
    let reads = Arc::new(AtomicUsize::new(0));
    let mut source = fvid::playback_webm_audio::WebmAudioReader::open(
        Meter {
            input: Cursor::new(bytes),
            ranges,
            reads: reads.clone(),
        },
        Default::default(),
    )
    .unwrap();
    reads.store(0, Ordering::Relaxed);
    assert_eq!(source.seek_to(6_000_000), 6_000_000);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        0,
        "seek decoded or read PCM preroll payload"
    );
    let packet = source.next_packet().unwrap().unwrap();
    assert_eq!(packet.pts, 8_000_000);
    assert!(reads.load(Ordering::Relaxed) > 0);
}
#[test]
fn malformed_pcm_stride_is_not_hidden_by_seeking_beyond_eof() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/pcm-timeline/bad-last-f32-1.mkv");
    let bytes = std::fs::read(path).unwrap();
    let mut video =
        fvid::playback_native::NativeReader::software(Cursor::new(bytes.clone()), 32 << 20)
            .unwrap();
    let mut frames = 0;
    while video.read_frame_raw().unwrap().is_some() {
        frames += 1;
    }
    assert_eq!(frames, 6);
    let mut reader =
        fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(bytes), Default::default())
            .unwrap();
    assert_eq!(reader.seek_to(1_000_000_000), 1_000_000_000);
    let error = match reader.next_packet() {
        Err(error) => error,
        _ => panic!("malformed PCM geometry was hidden by seek"),
    };
    assert!(error.to_string().contains("incomplete channel frame"));
}
