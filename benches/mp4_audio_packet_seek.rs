//! Manual ordered IMA4 MP4 navigation and actual seek-to-PCM comparison.
use fvid::{
    audio::{AudioStep, AudioStream},
    playback_mp4_audio::Mp4AudioReader,
};
use std::{
    io::Cursor,
    time::{Duration, Instant},
};
fn ramp(predictor: i32) -> Vec<u8> {
    (1..=64)
        .flat_map(|n| ((predictor + n) as f32 / 32768.0).to_le_bytes())
        .collect()
}
fn first_pcm(stream: &mut dyn AudioStream) -> Option<(Duration, Vec<u8>)> {
    let mut decoder = stream.make_decoder().unwrap();
    loop {
        match stream.next_step().unwrap() {
            Some(AudioStep::Encoded(p)) => {
                if let Some(mut frame) = decoder
                    .decode_packet(&p.data, stream.decoder_stamp(&p), p.duration as u64)
                    .unwrap()
                {
                    if let Some(frames) = stream
                        .decoded_packet_sample_limit(frame.source_pts, frame.source_duration)
                        .unwrap()
                    {
                        let bytes = frames * usize::from(stream.channels()) * 4;
                        assert!(bytes <= frame.packet.data.len());
                        frame.packet.data.truncate(bytes);
                    }
                    if let Some(p) = stream
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        return Some((p.presentation_time(), p.data));
                    }
                }
            }
            Some(AudioStep::ResetDecoder) => stream.reset_decoder(&mut decoder).unwrap(),
            Some(AudioStep::ReconfigureDecoder) => panic!("single-description input"),
            Some(AudioStep::Pcm(p)) => return Some((p.presentation_time(), p.data)),
            None => {
                stream.validate_eof().unwrap();
                return None;
            }
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let Some(path) = args.get(1).filter(|p| !p.starts_with("--")) else {
        eprintln!("manual benchmark: generated IMA4 MP4 path and optional query count");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let queries: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(1000);
    assert!(queries > 0);
    let input = Cursor::new(data.clone());
    let start = Instant::now();
    let mut reader = Mp4AudioReader::open(input, Default::default()).unwrap();
    let open_ns = start.elapsed().as_nanos();
    let count = reader.track().samples.len();
    assert!(count >= 6 && reader.track().samples.expanded().is_some());
    assert_eq!(
        (
            reader.track().codec,
            reader.sample_rate(),
            reader.channels(),
            reader.track().timescale
        ),
        (*b"ima4", 8000, 1, 8000)
    );
    let start = Instant::now();
    assert_eq!(reader.seek_to(0), 0);
    let first_seek_ns = start.elapsed().as_nanos();
    let mut cold = Mp4AudioReader::open(Cursor::new(data.clone()), Default::default()).unwrap();
    let start = Instant::now();
    assert_eq!(cold.seek(0), 0);
    let cold_direct_seek_ns = start.elapsed().as_nanos();
    drop(cold);
    let targets: Vec<_> = (0..queries)
        .map(|i| (i.wrapping_mul(7919) % count) as i64 * 64)
        .collect();
    let start = Instant::now();
    for &target in &targets {
        assert_eq!(reader.seek_to(std::hint::black_box(target)), target);
    }
    let navigation_ns = start.elapsed().as_nanos();
    let reference = [ramp(128), ramp(1152), ramp(2176)];
    let start = Instant::now();
    for &target in &targets {
        assert_eq!(reader.seek_to(target), target);
        let (at, pcm) = first_pcm(&mut reader).unwrap();
        assert_eq!(at, Duration::from_millis(target as u64 / 8));
        assert_eq!(pcm, reference[(target as usize / 64) % 3]);
    }
    let seek_pcm_ns = start.elapsed().as_nanos();
    assert_eq!(reader.seek_to(count as i64 * 64), (count as i64 - 1) * 64);
    assert!(first_pcm(&mut reader).is_none());
    reader.rewind();
    assert_eq!(
        first_pcm(&mut reader).unwrap(),
        (Duration::ZERO, reference[0].clone())
    );
    println!(
        "{}",
        serde_json::json!({"packets":count,"queries":queries,"open_ns":open_ns,"first_seek_ns":first_seek_ns,"cold_direct_seek_ns":cold_direct_seek_ns,"navigation_ns":navigation_ns,"seek_pcm_ns":seek_pcm_ns,"scope":"Memory-resident IMA4 index; exact analytic first PCM/timestamps,rewind/EOF. Setup and actual software seek measured separately. No GPU or whole-player throughput claim."})
    );
}
