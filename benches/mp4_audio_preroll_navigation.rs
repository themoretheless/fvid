//! Manual audio seek setup measurement; workload generation is separate.
use fvid::{
    audio::{AudioStep, AudioStream},
    playback_mp4_audio::Mp4AudioReader,
};
use std::{io::Cursor, time::Instant};

fn first_pcm(stream: &mut dyn AudioStream) -> Option<(std::time::Duration, Vec<u8>)> {
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
            Some(AudioStep::ReconfigureDecoder) => panic!("single-description workload"),
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
        eprintln!("manual benchmark: generated MACE MP4 path and optional query count");
        return;
    };
    let data = std::fs::read(path).unwrap();
    let queries: usize = args.get(2).map(|s| s.parse().unwrap()).unwrap_or(1000);
    assert!(queries > 0);
    let mut reader = Mp4AudioReader::open(Cursor::new(data.clone()), Default::default()).unwrap();
    let count = reader.track().samples.len();
    assert!(count >= 40);
    assert!(
        reader.track().samples.expanded().is_some(),
        "must exercise expanded index"
    );
    assert_eq!(reader.track().codec, *b"MAC3");
    let endpoint = count as i64 * 36;
    assert_eq!(reader.seek_to(endpoint), endpoint); // warm lazy endpoint, excluded
    let targets: Vec<_> = (0..queries)
        .map(|i| ((i * 7919) % count) as i64 * 36)
        .collect();
    let start = Instant::now();
    for &target in &targets {
        assert_eq!(reader.seek_to(std::hint::black_box(target)), target);
    }
    let navigation_ns = start.elapsed().as_nanos();
    // Real history restoration stays in scope. Compare repeated decoded output
    // at deterministic targets, and count its cost separately from setup.
    let actual_targets = [0, endpoint / 3 / 36 * 36, (count as i64 - 1) * 36];
    let mut references = Vec::new();
    for target in actual_targets {
        let mut fresh =
            Mp4AudioReader::open(Cursor::new(data.clone()), Default::default()).unwrap();
        assert_eq!(fresh.seek_to(target), target);
        references.push(first_pcm(&mut fresh).expect("positive seek PCM"));
    }
    let start = Instant::now();
    for (target, wanted) in actual_targets.into_iter().zip(references) {
        assert_eq!(reader.seek_to(target), target);
        let got = first_pcm(&mut reader).unwrap();
        assert_eq!(got, wanted);
        assert!(!got.1.is_empty());
    }
    let seek_pcm_ns = start.elapsed().as_nanos();
    println!(
        "{}",
        serde_json::json!({"packets":count,"queries":queries,"navigation_ns":navigation_ns,"seek_pcm_ns":seek_pcm_ns,"scope":"memory-resident MACE MP4; seek setup and real preroll to first PCM measured separately; no whole-player or hardware claim"})
    );
}
