//! Full raw_data_block acceptance, delayed frame identity, EOF and atomic native histories.
use fvid::codec::aac_ps_native::{Frame, NativePsAacDecoder};
use fvid::container::mp4::{Limits, Mp4Reader};
use serde_json::Value;
const DATA: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
fn native() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-native-oracles.json"
    ))
    .unwrap()
}
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn packets(name: &str) -> Vec<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name);
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let ai = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    (0..mp4.tracks()[ai].samples.len())
        .map(|n| {
            let mut data = vec![];
            mp4.read_packet(ai, n, &mut data).unwrap();
            data
        })
        .collect()
}
fn values(v: &Value) -> Vec<f64> {
    let off = v[0].as_u64().unwrap() as usize;
    let count = v[1].as_u64().unwrap() as usize;
    DATA[off..off + count * 8]
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn compare(actual: &Frame, row: &Value, rate: u32, index: usize) {
    assert_eq!(actual.frame_index, index as u64);
    assert_eq!(actual.sample_rate, rate);
    let key = if rate == 48000 { "Double" } else { "Core" };
    let left = values(&row[key][0]);
    let right = values(&row[key][1]);
    assert_eq!(actual.pcm.len(), left.len() * 2);
    for (i, (&l, &r)) in left.iter().zip(&right).enumerate() {
        for (c, v) in [l, r].into_iter().enumerate() {
            let e = (v / 32768.) as f32;
            let a = actual.pcm[2 * i + c];
            assert!(a.is_finite());
            assert!(
                (a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                "{a} != {e}"
            );
        }
    }
}
#[test]
fn complete_native_packets_match_stereo_pcm_for_960_1024_single_double_and_both_signallings() {
    let n = native();
    let refs = reference();
    let mut count = 0;
    for case in n["cases"].as_array().unwrap() {
        let file = case["file"].as_str().unwrap();
        let data = packets(file);
        assert_eq!(data.len(), 3);
        let expected = refs["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| {
                v["source"]["name"] == file
                    && v["source"]["kind"]
                        .as_str()
                        .unwrap()
                        .starts_with("sbr-video")
                    && v["zero_eof"] == true
            })
            .unwrap();
        for config in case["configurations"].as_array().unwrap() {
            count += 1;
            let asc = hex(config["asc"].as_str().unwrap());
            let rate = config["output_rate"].as_u64().unwrap() as u32;
            let mut decoder = NativePsAacDecoder::new(&asc).unwrap();
            assert_eq!(decoder.sample_rate(), rate);
            assert_eq!(decoder.channels(), 2);
            assert_eq!(decoder.channel_mask(), 3);
            let mut frames = vec![];
            for (i, p) in data.iter().enumerate() {
                let saved = decoder.checkpoint();
                let rendered = decoder.decode(p).unwrap();
                assert_eq!(decoder.pending_frame_index(), Some(i as u64));
                decoder.restore(&saved).unwrap();
                assert_eq!(rendered, decoder.decode(p).unwrap());
                if i == 0 {
                    assert!(rendered.is_none());
                } else {
                    frames.push(rendered.unwrap());
                }
            }
            let saved = decoder.checkpoint();
            let last = decoder.finish().unwrap().unwrap();
            decoder.restore(&saved).unwrap();
            assert_eq!(Some(last.clone()), decoder.finish().unwrap());
            frames.push(last);
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(decoder.pending_frame_index(), None);
            let eof = decoder.checkpoint();
            let error = decoder.decode(&data[0]).unwrap_err();
            assert!(error.to_string().contains("input after EOF requires reset"));
            decoder.restore(&eof).unwrap();
            assert!(decoder.finish().unwrap().is_none());
            for (i, f) in frames.iter().enumerate() {
                compare(f, &expected["frames"][i], rate, i);
            }
            decoder.reset();
            for (i, p) in data.iter().enumerate() {
                let f = decoder.decode(p).unwrap();
                if i == 0 {
                    assert!(f.is_none());
                } else {
                    assert_eq!(f, Some(frames[i - 1].clone()));
                }
            }
            assert_eq!(decoder.finish().unwrap(), frames.last().cloned());
        }
    }
    assert_eq!(count, 16);
}
#[test]
fn malformed_synthetic_videos_reproduce_exact_errors_and_preserve_core_extension_and_pending_pcm() {
    let m = native();
    let valid = packets(m["cases"][0]["file"].as_str().unwrap());
    let authored =
        include_bytes!("fixtures/playback-errors/he-aac-ps-native-malformed-packets.bin");
    for bad in m["malformed"].as_array().unwrap() {
        let asc = hex(bad["video"]["asc"].as_str().unwrap());
        let data = packets(bad["video"]["file"].as_str().unwrap());
        for (p, row) in data.iter().zip(bad["frames"].as_array().unwrap()) {
            let off = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            assert_eq!(p, &authored[off..off + len]);
        }
        let mut decoder = NativePsAacDecoder::new(&asc).unwrap();
        assert!(decoder.decode(&data[0]).unwrap().is_none());
        let saved = decoder.checkpoint();
        let error = decoder.decode(&data[1]).unwrap_err();
        assert!(
            error.to_string().contains(bad["error"].as_str().unwrap()),
            "{}: {error}",
            bad["name"]
        );
        assert_eq!(decoder.pending_frame_index(), Some(0));
        let mut baseline = NativePsAacDecoder::new(&asc).unwrap();
        baseline.restore(&saved).unwrap();
        for p in &valid[1..] {
            assert_eq!(decoder.decode(p).unwrap(), baseline.decode(p).unwrap());
        }
        assert_eq!(decoder.finish().unwrap(), baseline.finish().unwrap());
    }
}
#[test]
fn checkpoints_reject_other_format_without_changing_queued_audio_and_truncations_are_transactional()
{
    let m = native();
    let case = &m["cases"][0];
    let asc = hex(case["configurations"][2]["asc"].as_str().unwrap());
    let data = packets(case["file"].as_str().unwrap());
    let mut decoder = NativePsAacDecoder::new(&asc).unwrap();
    assert!(decoder.decode(&data[0]).unwrap().is_none());
    let saved = decoder.checkpoint();
    for end in 0..data[1].len() - 1 {
        assert!(
            decoder.decode(&data[1][..end]).is_err(),
            "accepted truncated packet {end}"
        );
        assert_eq!(decoder.pending_frame_index(), Some(0));
        let after_failure = decoder.decode(&data[1]).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(after_failure, decoder.decode(&data[1]).unwrap());
        decoder.restore(&saved).unwrap();
    }
    let other_rate =
        NativePsAacDecoder::new(&hex(case["configurations"][0]["asc"].as_str().unwrap()))
            .unwrap()
            .checkpoint();
    assert!(
        decoder
            .restore(&other_rate)
            .unwrap_err()
            .to_string()
            .contains("configuration mismatch")
    );
    assert_eq!(decoder.pending_frame_index(), Some(0));
    let mut baseline = NativePsAacDecoder::new(&asc).unwrap();
    baseline.restore(&saved).unwrap();
    assert_eq!(
        decoder.decode(&data[1]).unwrap(),
        baseline.decode(&data[1]).unwrap()
    );
}
