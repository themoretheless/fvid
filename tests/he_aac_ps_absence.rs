//! Native declared-PS streams accept absent/late PS as normative dual mono.
use fvid::{
    codec::aac_ps_native::NativePsAacDecoder,
    container::mp4::{Limits, Mp4Reader},
};
use serde_json::Value;
const PCM: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
fn m() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-absence-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn values(v: &Value) -> Vec<f64> {
    let off = v[0].as_u64().unwrap() as usize;
    let len = v[1].as_u64().unwrap() as usize;
    PCM[off..off + len * 8]
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
#[test]
fn original_absent_late_and_returning_ps_videos_accept_both_independent_stereo_pcm_channels() {
    let manifest = m();
    let authored = include_bytes!("fixtures/playback-errors/he-aac-ps-absence-packets.bin");
    let mut count = 0;
    for c in manifest["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let mut mp4 =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
        let ai = mp4
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        assert_eq!(mp4.tracks()[ai].samples.len(), 3);
        let mut packets = vec![];
        for i in 0..3 {
            let mut data = vec![];
            mp4.read_packet(ai, i, &mut data).unwrap();
            let row = &c["frames"][i];
            let off = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            assert_eq!(data, &authored[off..off + len]);
            packets.push(data);
        }
        for (key, asc, rate, width) in [
            ("Double", c["video"]["asc"].as_str().unwrap(), 48000, 64),
            ("Core", c["asc_core"].as_str().unwrap(), 24000, 32),
        ] {
            count += 1;
            let mut d = NativePsAacDecoder::new(&hex(asc)).unwrap();
            let mut frames = vec![];
            for p in &packets {
                let saved = d.checkpoint();
                let frame = d.decode(p).unwrap();
                d.restore(&saved).unwrap();
                assert_eq!(frame, d.decode(p).unwrap());
                if let Some(f) = frame {
                    frames.push(f);
                }
            }
            let saved = d.checkpoint();
            let tail = d.finish().unwrap().unwrap();
            d.restore(&saved).unwrap();
            assert_eq!(Some(tail.clone()), d.finish().unwrap());
            frames.push(tail);
            assert!(d.finish().unwrap().is_none());
            assert_eq!(frames.len(), 3);
            let left = values(&c["pcm"][key][0]);
            let right = values(&c["pcm"][key][1]);
            let actual: Vec<f32> = frames.iter().flat_map(|f| f.pcm.iter().copied()).collect();
            assert_eq!(actual.len(), left.len() * 2);
            for (i, a) in actual.iter().enumerate() {
                let e = (if i % 2 == 0 {
                    left[i / 2]
                } else {
                    right[i / 2]
                }) as f32;
                assert!(
                    (*a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                    "{} {key}: {a} != {e}",
                    c["name"]
                );
            }
            for (i, f) in frames.iter().enumerate() {
                assert_eq!(f.frame_index, i as u64);
                assert_eq!(f.sample_rate, rate);
                assert_eq!(
                    f.pcm.len(),
                    c["slots"].as_u64().unwrap() as usize * 2 * width * 2
                );
                if c["stereo_active"][i] == false {
                    let tail = &f.pcm[10 * width * 2..];
                    assert!(
                        tail.chunks_exact(2).all(|r| r[0] == r[1]),
                        "dual-mono filter histories did not settle"
                    );
                    assert!(tail.iter().any(|&v| v != 0.));
                } else {
                    assert!(
                        f.pcm.chunks_exact(2).any(|r| r[0] != r[1]),
                        "independent PS header did not start stereo"
                    );
                }
            }
            d.reset();
            for (i, p) in packets.iter().enumerate() {
                let f = d.decode(p).unwrap();
                if i == 0 {
                    assert!(f.is_none());
                } else {
                    assert_eq!(f, Some(frames[i - 1].clone()));
                }
            }
            assert_eq!(d.finish().unwrap(), frames.last().cloned());
        }
    }
    assert_eq!(count, 28);
}
