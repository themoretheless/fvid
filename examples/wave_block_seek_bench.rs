//! Explicit seek-to-first-PCM CPU benchmark; inputs generated separately.
use fvid::audio::AudioStream;
use std::{
    io::Cursor,
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert_eq!(args.len(), 3, "MANIFEST OUTPUT ROUNDS");
    let manifest = PathBuf::from(&args[0]);
    let root = manifest.parent().unwrap();
    let rounds: usize = args[2].parse().unwrap();
    assert!((3..=100).contains(&rounds));
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    let mut rows = Vec::new();
    for c in data["cases"].as_array().unwrap() {
        let rate = c["rate"].as_u64().unwrap() as u32;
        let channels = c["channels"].as_u64().unwrap() as usize;
        let frames = c["frames"].as_u64().unwrap();
        let target = c["target_frame"].as_u64().unwrap();
        let at = Duration::from_nanos(c["target_ns"].as_u64().unwrap());
        let block = c["block_bytes"].as_u64().unwrap() as usize;
        let spb = c["block_frames"].as_u64().unwrap();
        let input = std::fs::read(root.join(c["file"].as_str().unwrap())).unwrap();
        let expected = std::fs::read(root.join(c["expected"].as_str().unwrap())).unwrap();
        let mut stream =
            fvid::playback_wav::WavAudioReader::open(Cursor::new(input), Default::default())
                .unwrap();
        assert_eq!(
            stream.duration(),
            Some(Duration::new(
                frames / u64::from(rate),
                ((frames % u64::from(rate)) * 1_000_000_000 / u64::from(rate)) as u32
            ))
        );
        let mut decoder = stream.make_decoder().unwrap();
        for i in 0..=rounds {
            let started = Instant::now();
            let landed = stream.seek_time(at);
            decoder.reset();
            let mut packets = 0u64;
            let mut bytes = 0u64;
            let mut decoded = 0u64;
            let pcm = loop {
                let p = stream
                    .next_packet()
                    .unwrap()
                    .expect("EOF before target PCM");
                packets += 1;
                bytes += p.data.len() as u64;
                decoded += (p.data.len() / block) as u64 * spb;
                if let Some(f) = decoder
                    .decode_packet(&p.data, stream.decoder_stamp(&p), p.duration as u64)
                    .unwrap()
                {
                    if let Some(p) = stream.present_decoded(f.packet, f.source_pts).unwrap() {
                        break p;
                    }
                }
            };
            let ns = started.elapsed().as_nanos() as u64;
            assert_eq!(landed, target as i64);
            assert_eq!(pcm.pts, target);
            assert_eq!(pcm.timebase_num, 1);
            assert_eq!(pcm.timebase_den, rate);
            assert_eq!(pcm.data, expected);
            assert_eq!(pcm.data.len() % (channels * 4), 0);
            if c["end_after_first"] == true {
                assert!(stream.next_packet().unwrap().is_none());
                stream.validate_eof().unwrap();
            }
            std::hint::black_box(&pcm);
            if i > 0 {
                rows.push(serde_json::json!({"name":c["name"],"position":c["position"],"round":i,"ns":ns,"packets":packets,"encoded_bytes":bytes,"decoded_frame_capacity":decoded,"pcm_bytes":pcm.data.len(),"verified":true}));
            }
        }
    }
    std::fs::write(&args[1],serde_json::to_vec_pretty(&serde_json::json!({"debug_assertions":cfg!(debug_assertions),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"rounds":rounds,"warmup_rounds_excluded":1,"rows":rows})).unwrap()).unwrap();
}
