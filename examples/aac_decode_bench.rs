//! Full owned container-index + AAC-decode + PCM-output benchmark from memory.
//! File loading is outside timing; each iteration constructs fresh decoder state.
use fvid::{
    container::{adts, mp4, webm},
    native_media::{self, AudioDecodeStats},
};
use std::{
    hint::black_box,
    io::{Cursor, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
#[derive(Default)]
struct Sink {
    bytes: u64,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        black_box(bytes);
        self.bytes += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn decode(data: &[u8]) -> fvid::Result<AudioDecodeStats> {
    let mut sink = Sink::default();
    let source = Cursor::new(black_box(data));
    let stats = if data.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        native_media::decode_matroska_aac_reader(
            webm::WebmReader::open(source, Default::default())?,
            &mut sink,
            None,
        )?
    } else if data.get(4..8) == Some(b"ftyp".as_slice()) {
        native_media::decode_mp4_aac_reader(
            mp4::Mp4Reader::open(source, Default::default())?,
            &mut sink,
            None,
        )?
    } else {
        native_media::decode_adts_aac_reader(adts::StreamReader::open(source)?, &mut sink, None)?
    };
    assert_eq!(
        sink.bytes,
        stats.sample_frames * u64::from(stats.channels) * 4
    );
    black_box(sink.bytes);
    Ok(stats)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.is_empty() {
        paths = [
            "aac-mono-44k.aac",
            "aac-stereo.aac",
            "aac-tns.aac",
            "aac-51-active.aac",
            "aac-native-edit.m4a",
            "aac-stereo.mka",
            "aac-960-48000.mka",
            "aac-960-48000.m4a",
        ]
        .into_iter()
        .map(|name| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/audio")
                .join(name)
        })
        .collect();
    }
    for path in paths {
        let data = std::fs::read(&path)?;
        let expected = decode(&data)?;
        let mut samples = Vec::new();
        let mut iterations = 0;
        for _ in 0..5 {
            let begin = Instant::now();
            let mut count = 0;
            while count == 0 || (begin.elapsed() < Duration::from_millis(100) && count < 512) {
                assert_eq!(decode(&data)?, expected);
                count += 1;
            }
            samples.push(begin.elapsed().as_secs_f64() / f64::from(count));
            iterations += count;
        }
        samples.sort_by(f64::total_cmp);
        let elapsed = samples[2];
        let audio = expected.sample_frames as f64 / f64::from(expected.sample_rate);
        println!(
            "{}: {} Hz {} ch, {} decoded packets, {:.3}s PCM, median {:.3}ms, {:.1}x realtime, {iterations} iterations",
            path.file_name().unwrap_or_default().to_string_lossy(),
            expected.sample_rate,
            expected.channels,
            expected.decoded_frames,
            audio,
            elapsed * 1e3,
            audio / elapsed
        );
    }
    Ok(())
}
