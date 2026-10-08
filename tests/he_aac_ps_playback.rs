//! Delayed PS PCM retains the original MP4 packet window, EOF and seek checkpoints.
use fvid::{
    codec::aac_ps_playback::{DecodedFrame, PsAacDecoder},
    container::mp4::{Limits, Mp4Reader},
};
use serde_json::Value;
const PCM: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-playback-oracles.json"
    ))
    .unwrap()
}
fn reference() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap()
}
fn open(case: &Value) -> (Vec<u8>, Vec<Vec<u8>>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(case["video"]["file"].as_str().unwrap());
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let ai = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let t = &mp4.tracks()[ai];
    assert_eq!(t.timescale, 48000);
    assert_eq!(t.channels, 2);
    for i in 0..3 {
        let s = t.samples.get(i).unwrap();
        assert_eq!(s.pts, case["pts"][i].as_i64().unwrap());
        assert_eq!(
            u64::from(s.duration),
            case["durations"][i].as_u64().unwrap()
        );
    }
    let config = t.configuration.clone();
    let packets = (0..3)
        .map(|i| {
            let mut data = vec![];
            mp4.read_packet(ai, i, &mut data).unwrap();
            data
        })
        .collect();
    (config, packets)
}
fn scalars(v: &Value) -> Vec<f64> {
    let off = v[0].as_u64().unwrap() as usize;
    let len = v[1].as_u64().unwrap() as usize;
    PCM[off..off + len * 8]
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn compare(a: &DecodedFrame, b: &DecodedFrame) {
    assert_eq!(a.packet.data, b.packet.data);
    assert_eq!(a.packet.pts, b.packet.pts);
    assert_eq!(a.packet.timebase_num, b.packet.timebase_num);
    assert_eq!(a.packet.timebase_den, b.packet.timebase_den);
    assert_eq!(a.frame_index, b.frame_index);
    assert_eq!(a.source_pts, b.source_pts);
    assert_eq!(a.source_duration, b.source_duration);
}
#[test]
fn unequal_mp4_packet_windows_follow_the_decoded_source_frame_through_eof_and_replay() {
    let m = manifest();
    let refs = reference();
    for case in m["cases"].as_array().unwrap() {
        let (config, packets) = open(case);
        let expected = refs["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| {
                c["source"]["name"] == case["source_reference"]
                    && c["source"]["kind"]
                        .as_str()
                        .unwrap()
                        .starts_with("sbr-video")
                    && c["zero_eof"] == true
            })
            .unwrap();
        let mut decoder = PsAacDecoder::new(&config, 48000, 2).unwrap();
        assert_eq!(decoder.spec().channels, 2);
        assert_eq!(decoder.spec().sample_rate, 48000);
        let mut frames = vec![];
        for i in 0..3 {
            let saved = decoder.checkpoint().unwrap();
            let output = decoder
                .decode(
                    &packets[i],
                    case["pts"][i].as_i64().unwrap(),
                    case["durations"][i].as_u64().unwrap(),
                )
                .unwrap();
            assert_eq!(decoder.pending_frame_index(), Some(i as u64));
            decoder.restore(&saved).unwrap();
            let replay = decoder
                .decode(
                    &packets[i],
                    case["pts"][i].as_i64().unwrap(),
                    case["durations"][i].as_u64().unwrap(),
                )
                .unwrap();
            if i == 0 {
                assert!(output.is_none());
                assert!(replay.is_none());
            } else {
                compare(output.as_ref().unwrap(), replay.as_ref().unwrap());
                frames.push(output.unwrap());
            }
        }
        let saved = decoder.checkpoint().unwrap();
        let tail = decoder.finish().unwrap().unwrap();
        decoder.restore(&saved).unwrap();
        compare(&tail, &decoder.finish().unwrap().unwrap());
        frames.push(tail);
        assert!(decoder.finish().unwrap().is_none());
        assert_eq!(decoder.pending_frame_index(), None);
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.frame_index, i as u64);
            assert_eq!(frame.source_pts, case["pts"][i].as_i64().unwrap());
            assert_eq!(
                frame.source_duration,
                case["durations"][i].as_u64().unwrap()
            );
            assert_eq!(frame.packet.pts, frame.source_pts as u64);
            assert_eq!(frame.packet.timebase_num, 1);
            assert_eq!(frame.packet.timebase_den, 48000);
            let left = scalars(&expected["frames"][i]["Double"][0]);
            let right = scalars(&expected["frames"][i]["Double"][1]);
            assert_eq!(frame.packet.data.len(), left.len() * 8);
            for (j, sample) in frame.packet.data.chunks_exact(4).enumerate() {
                let a = f32::from_le_bytes(sample.try_into().unwrap());
                let e = ((if j % 2 == 0 {
                    left[j / 2]
                } else {
                    right[j / 2]
                }) / 32768.) as f32;
                assert!((a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16);
            }
            // The adapter preserves full codec PCM. The caller can now trim with this
            // frame's OWN presentation window, rather than the lookahead packet's one.
            assert!(frame.source_duration as usize * 8 <= frame.packet.data.len());
        }
        // Seek requires native preroll; replay after reset retains original packet
        // identity/window instead of assigning the lookahead packet's timestamp.
        decoder.reset();
        for i in 0..3 {
            let out = decoder
                .decode(
                    &packets[i],
                    case["pts"][i].as_i64().unwrap(),
                    case["durations"][i].as_u64().unwrap(),
                )
                .unwrap();
            if i > 0 {
                compare(&out.unwrap(), &frames[i - 1]);
            }
        }
        compare(&decoder.finish().unwrap().unwrap(), &frames[2]);
    }
}
#[test]
fn signed_preroll_timestamps_and_windows_are_not_replaced_by_later_input_metadata() {
    let m = manifest();
    let case = &m["cases"][0];
    let (config, packets) = open(case);
    let mut decoder = PsAacDecoder::new(&config, 48000, 2).unwrap();
    assert!(decoder.decode(&packets[0], -1024, 512).unwrap().is_none());
    let first = decoder.decode(&packets[1], 7000, 123).unwrap().unwrap();
    assert_eq!(first.source_pts, -1024);
    assert_eq!(first.source_duration, 512);
    assert_eq!(first.packet.pts, 0);
    let second = decoder.decode(&packets[2], 11000, 999).unwrap().unwrap();
    assert_eq!(second.source_pts, 7000);
    assert_eq!(second.source_duration, 123);
    assert_eq!(second.packet.pts, 7000);
    let last = decoder.finish().unwrap().unwrap();
    assert_eq!(last.source_pts, 11000);
    assert_eq!(last.source_duration, 999);
    assert_eq!(last.packet.pts, 11000);
}
#[test]
fn late_packet_failure_checkpoint_restore_and_incompatible_restore_preserve_pending_timing() {
    let m = manifest();
    let case = &m["cases"][0];
    let (config, packets) = open(case);
    let mut decoder = PsAacDecoder::new(&config, 48000, 2).unwrap();
    decoder.decode(&packets[0], 44, 555).unwrap();
    let saved = decoder.checkpoint().unwrap();
    let mut bad = packets[1].clone();
    bad.push(0xa5);
    let e = decoder.decode(&bad, 999, 1000).err().unwrap();
    assert!(e.to_string().contains("trailing bytes after PS AAC END"));
    assert_eq!(decoder.pending_frame_index(), Some(0));
    assert!(decoder.checkpoint().is_none());
    assert!(
        decoder
            .finish()
            .err()
            .unwrap()
            .to_string()
            .contains("requires reset")
    );
    decoder.restore(&saved).unwrap();
    let out = decoder.decode(&packets[1], 700, 321).unwrap().unwrap();
    assert_eq!(out.source_pts, 44);
    assert_eq!(out.source_duration, 555);
    let (other_config, _) = open(&m["cases"][1]);
    let other = PsAacDecoder::new(&other_config, 48000, 2)
        .unwrap()
        .checkpoint()
        .unwrap();
    assert!(
        decoder
            .restore(&other)
            .err()
            .unwrap()
            .to_string()
            .contains("configuration mismatch")
    );
    assert_eq!(decoder.pending_frame_index(), Some(1));
    let out = decoder.decode(&packets[2], 2000, 800).unwrap().unwrap();
    assert_eq!(out.source_pts, 700);
    assert_eq!(out.source_duration, 321);
    decoder.reset();
    assert_eq!(decoder.pending_frame_index(), None);
    assert!(decoder.checkpoint().is_some());
    for (rate, channels) in [(24000, 2), (48000, 1)] {
        assert!(
            PsAacDecoder::new(&config, rate, channels)
                .err()
                .unwrap()
                .to_string()
                .contains("disagrees with container")
        );
    }
}

#[test]
fn original_timing_videos_decode_and_checkpoint_on_a_two_mib_playback_thread_stack() {
    std::thread::Builder::new()
        .name("ps-playback-stack-regression".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            unequal_mp4_packet_windows_follow_the_decoded_source_frame_through_eof_and_replay();
            late_packet_failure_checkpoint_restore_and_incompatible_restore_preserve_pending_timing(
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
