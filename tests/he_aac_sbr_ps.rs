//! Actual authored MP4/SCE/SBR/PS packets to stereo PCM; native combined API remains pending.
use fvid::{
    codec::aac_native::NativeAacDecoder,
    container::mp4::{Limits, Mp4Reader},
};
use fvid_media::owned_aac::{
    aac_sbr_dsp::OutputRate,
    aac_sbr_history,
    aac_sbr_ps::{Decoder, Frame},
    aac_sbr_qmf_dsp,
    bits::BitReader,
};
use serde_json::Value;
const DATA: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-dsp-reference.bin");
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-dsp-oracles.json"
    ))
    .unwrap()
}
fn matrices() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-matrix-controller-oracles.json"
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
    let count = v[1].as_u64().unwrap() as usize;
    DATA[off..off + count * 8]
        .chunks_exact(8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn compare(frame: &Frame, expected: &Value, mode: OutputRate, index: usize) {
    assert_eq!(frame.frame_index, index as u64);
    assert_eq!(u64::from(frame.slots), expected["slots"].as_u64().unwrap());
    assert_eq!(frame.output_rate, mode);
    let key = if mode == OutputRate::Double {
        "Double"
    } else {
        "Core"
    };
    for c in 0..2 {
        let refs = values(&expected[key][c]);
        assert_eq!(frame.pcm[c].len(), refs.len());
        for (&a, e) in frame.pcm[c].iter().zip(refs) {
            let e = e / 32768.;
            assert!(
                (a - e).abs() < 4e-14 * (1. + e.abs()),
                "channel{c} {a} != {e}"
            );
        }
    }
    assert_ne!(frame.pcm[0], frame.pcm[1]);
}
fn packets(video: &Value) -> (Vec<Vec<u8>>, Vec<Vec<f32>>) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(video["video"]["file"].as_str().unwrap());
    let mut mp4 = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let track = mp4
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    let slots = video["slots"].as_u64().unwrap_or(16) as u8;
    let frame_samples = u32::from(slots) * 128;
    let t = &mp4.tracks()[track];
    assert_eq!(t.timescale, 48000);
    assert_eq!(t.duration, u64::from(frame_samples) * 3);
    for i in 0..t.samples.len() {
        let s = t.samples.get(i).unwrap();
        assert_eq!(s.duration, frame_samples);
        assert_eq!(s.dts, i as u64 * u64::from(frame_samples));
        assert_eq!(s.pts, i as i64 * i64::from(frame_samples));
    }
    let authored: &[u8] = if slots == 15 {
        include_bytes!("fixtures/playback-errors/he-aac-sbr-ps-960-packets.bin")
    } else {
        include_bytes!("fixtures/playback-errors/he-aac-ps-matrix-controller-packets.bin")
    };
    let mut core = NativeAacDecoder::new(&[0x13, if slots == 15 { 0x0c } else { 0x08 }]).unwrap();
    let mut pcm = vec![];
    for (i, row) in video["packet_frames"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let off = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let mut packet = vec![];
        mp4.read_packet(track, i, &mut packet).unwrap();
        assert_eq!(packet, &authored[off..off + len]);
        // Authored SCE occupies exactly 29 bits. Terminate that original SCE with
        // ID_END instead of the following FIL; decode it with the native LC core.
        let mut sce = packet[..4].to_vec();
        sce[3] = (sce[3] & 0xf8) | 7;
        let samples = core.decode(&sce).unwrap();
        assert_eq!(samples.len(), usize::from(slots) * 64);
        assert!(samples.iter().all(|&x| x == 0.));
        pcm.push(samples);
    }
    (
        video["sbr_payloads"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| hex(r.as_str().unwrap()))
            .collect(),
        pcm,
    )
}
fn open(raw: &[u8]) -> (BitReader<'_>, bool) {
    let mut bits = BitReader::new(raw);
    let kind = bits.read(4).unwrap();
    assert!(matches!(kind, 13 | 14));
    (bits, kind == 14)
}
#[test]
fn original_video_packets_reach_both_stereo_pcm_oracles_with_real_lookahead_and_eof() {
    let mut m = matrices();
    let thirty: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-sbr-ps-30-oracles.json"
    ))
    .unwrap();
    m["videos"]
        .as_array_mut()
        .unwrap()
        .extend(thirty["videos"].as_array().unwrap().iter().cloned());
    let refs = manifest();
    let mut count = 0;
    for case in refs["cases"].as_array().unwrap().iter().filter(|c| {
        c["source"]["kind"]
            .as_str()
            .unwrap()
            .starts_with("sbr-video")
            && c["zero_eof"] == true
    }) {
        count += 1;
        let video = m["videos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["video"]["file"] == case["source"]["name"])
            .unwrap();
        let slots = video["slots"].as_u64().unwrap_or(16) as u8;
        let width = usize::from(slots) * 2;
        let (payloads, pcm) = packets(video);
        let mut stream = aac_sbr_history::Stream::default();
        let mut qmf = aac_sbr_qmf_dsp::Dsp::default();
        let input = values(&case["input"]);
        for (i, raw) in payloads.iter().enumerate() {
            let (mut bits, crc) = open(raw);
            let f = stream
                .read(&mut bits, raw.len() * 8, crc, 48000, slots, 1)
                .unwrap();
            let rows = qmf.process(&f, &[&pcm[i]], 48000, slots).unwrap();
            assert_eq!(rows.qmf_limit, 27);
            assert_eq!(rows.rows[0].len(), width);
            for (j, value) in rows.rows[0]
                .iter()
                .flatten()
                .flat_map(|c| [c.re, c.im])
                .enumerate()
            {
                let e = input[i * width * 128 + j];
                assert!(
                    (value - e).abs() < 4e-12 * (1. + e.abs()),
                    "QMF {value} vs {e}"
                );
            }
        }
        for mode in [OutputRate::Double, OutputRate::Core] {
            let mut state = Decoder::default();
            let mut frames = vec![];
            for (i, raw) in payloads.iter().enumerate() {
                let saved = state.clone();
                let (mut bits, crc) = open(raw);
                let result = state
                    .read(&mut bits, raw.len() * 8, crc, &pcm[i], 48000, slots, mode)
                    .unwrap();
                assert_eq!(bits.remaining(), 0);
                let committed = state.clone();
                let (mut replay, crc) = open(raw);
                state = saved;
                assert_eq!(
                    result,
                    state
                        .read(&mut replay, raw.len() * 8, crc, &pcm[i], 48000, slots, mode)
                        .unwrap()
                );
                assert_eq!(state, committed);
                assert_eq!(state.pending_frame_index(), Some(i as u64));
                if i == 0 {
                    assert!(result.is_none());
                } else {
                    frames.push(result.unwrap());
                }
            }
            let saved = state.clone();
            let last = state.finish().unwrap().unwrap();
            let committed = state.clone();
            state = saved;
            assert_eq!(Some(last.clone()), state.finish().unwrap());
            assert_eq!(state, committed);
            frames.push(last);
            assert!(state.finish().unwrap().is_none());
            assert_eq!(state, committed);
            assert_eq!(state.pending_frame_index(), None);
            for (i, f) in frames.iter().enumerate() {
                compare(f, &case["frames"][i], mode, i);
            }
            let (mut bits, crc) = open(&payloads[0]);
            assert!(
                state
                    .read(
                        &mut bits,
                        payloads[0].len() * 8,
                        crc,
                        &pcm[0],
                        48000,
                        slots,
                        mode
                    )
                    .is_err()
            );
            assert_eq!(state, committed);
            state.reset();
            assert_eq!(state, Decoder::default());
            for (i, raw) in payloads.iter().enumerate() {
                let (mut bits, crc) = open(raw);
                let result = state
                    .read(&mut bits, raw.len() * 8, crc, &pcm[i], 48000, slots, mode)
                    .unwrap();
                if i > 0 {
                    assert_eq!(result, Some(frames[i - 1].clone()));
                }
            }
            assert_eq!(state.finish().unwrap(), frames.last().cloned());
        }
    }
    assert_eq!(count, 4);
}
#[test]
fn reader_queue_and_all_histories_roll_back_on_crc_pcm_ps_and_epoch_failures() {
    let m = matrices();
    let video = &m["videos"][0];
    let (payloads, pcm) = packets(video);
    let mut state = Decoder::default();
    let (mut bits, crc) = open(&payloads[0]);
    assert!(
        state
            .read(
                &mut bits,
                payloads[0].len() * 8,
                crc,
                &pcm[0],
                48000,
                16,
                OutputRate::Double
            )
            .unwrap()
            .is_none()
    );
    let saved = state.clone();
    let check =
        |state: &mut Decoder, raw: &[u8], samples: &[f32], rate, slots, mode, expected: &str| {
            let (mut bits, crc) = open(raw);
            let start = bits.position();
            let error = state
                .read(&mut bits, raw.len() * 8, crc, samples, rate, slots, mode)
                .unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "{error} did not reproduce {expected}"
            );
            assert_eq!(bits.position(), start);
            assert_eq!(*state, saved);
        };
    let mut broken = payloads[1].clone();
    broken[0] ^= 1;
    check(
        &mut state,
        &broken,
        &pcm[1],
        48000,
        16,
        OutputRate::Double,
        "SBR CRC mismatch",
    );
    let mut bad = pcm[1].clone();
    bad[1023] = f32::NAN;
    check(
        &mut state,
        &payloads[1],
        &bad,
        48000,
        16,
        OutputRate::Double,
        "invalid SBR preparation frame",
    );
    check(
        &mut state,
        &payloads[1],
        &pcm[1][..1000],
        48000,
        16,
        OutputRate::Double,
        "invalid SBR preparation frame",
    );
    check(
        &mut state,
        &payloads[1],
        &pcm[1],
        24000,
        16,
        OutputRate::Double,
        "SBR PS format changed without reset",
    );
    check(
        &mut state,
        &payloads[1],
        &pcm[1],
        48000,
        15,
        OutputRate::Double,
        "SBR PS format changed without reset",
    );
    check(
        &mut state,
        &payloads[1],
        &pcm[1],
        48000,
        16,
        OutputRate::Core,
        "SBR PS format changed without reset",
    );
    // Existing original no-PS SBR syntax is syntactically valid, but this owner
    // explicitly refuses missing PS; it must retain the queued prior output.
    let raw = include_bytes!("fixtures/playback-errors/aac-sbr-dsp-syntax.bin");
    let manifest: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-sbr-dsp-oracles.json"
    ))
    .unwrap();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["slots"] == 16 && v["smoothing"] == true && v["limiter"] == 2)
        .unwrap();
    let f = &c["frames"][1];
    let off = f["offset"].as_u64().unwrap() as usize;
    let len = f["byte_length"].as_u64().unwrap() as usize;
    check(
        &mut state,
        &raw[off..off + len],
        &pcm[1],
        48000,
        16,
        OutputRate::Double,
        "requires exactly one PS element",
    );
    let (mut bits, crc) = open(&payloads[1]);
    assert_eq!(
        state
            .read(
                &mut bits,
                payloads[1].len() * 8,
                crc,
                &pcm[1],
                48000,
                16,
                OutputRate::Double
            )
            .unwrap()
            .unwrap()
            .frame_index,
        0
    );
}
