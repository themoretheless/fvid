//! Dynamic Main/LC CCE rosters with independent scalar predictor/IMDCT gold.
use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::path::Path;
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-pce-roster.json")).unwrap()
}
fn file(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn asc(c: &Value) -> Vec<u8> {
    c["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
fn packets(c: &Value) -> Vec<Vec<u8>> {
    let raw = file("aac-pce-roster-packets.bin");
    c["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let at = r["offset"].as_u64().unwrap() as usize;
            raw[at..at + r["bytes"].as_u64().unwrap() as usize].to_vec()
        })
        .collect()
}
fn video(c: &Value) -> Vec<u8> {
    file(c["video"]["file"].as_str().unwrap())
}
#[test]
fn main_lc_dynamic_rosters_match_scalar_prediction_and_independent_source_overlap() {
    let m = manifest();
    let gold = file("aac-pce-roster-pcm.f32le");
    assert_eq!(m["cases"].as_array().unwrap().len(), 44);
    for c in m["cases"].as_array().unwrap() {
        let mut actual = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(c), &mut actual).unwrap();
        let at = c["pcm_offset"].as_u64().unwrap() as usize;
        let expected = &gold[at..at + c["pcm_bytes"].as_u64().unwrap() as usize];
        assert_eq!(actual.len(), expected.len());
        assert!(
            expected
                .chunks_exact(4)
                .any(|s| f32::from_le_bytes(s.try_into().unwrap()).abs() > 1e-5)
        );
        for (i, (a, b)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap());
            let b = f32::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 1e-8, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        if c["point"] == 3 && c["schedule"].as_str().unwrap().starts_with("return") {
            let wrong_at = c["discarded_history_pcm_offset"].as_u64().unwrap() as usize;
            let wrong = &gold[wrong_at..wrong_at + expected.len()];
            let error = expected
                .chunks_exact(4)
                .zip(wrong.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(
                error > 1e-6,
                "discarding source history is invisible: {}",
                c["name"]
            );
        }
        if c["schedule"] == "return-long" && c["object_type"] == 1 {
            let wrong_at = c["discarded_predictor_pcm_offset"].as_u64().unwrap() as usize;
            let wrong = &gold[wrong_at..wrong_at + expected.len()];
            let error = expected
                .chunks_exact(4)
                .zip(wrong.chunks_exact(4))
                .map(|(a, b)| {
                    (f32::from_le_bytes(a.try_into().unwrap())
                        - f32::from_le_bytes(b.try_into().unwrap()))
                    .abs()
                })
                .fold(0f32, f32::max);
            assert!(
                error > 1e-7,
                "discarding Main predictor is invisible: {}",
                c["name"]
            );
        }
        let pair = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| {
                p["object_type"] == c["object_type"]
                    && p["point"] == c["point"]
                    && p["tns"] == c["tns"]
                    && p["schedule"] == c["schedule"]
                    && p["dynamic"] != c["dynamic"]
            })
            .unwrap();
        let mut control = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(pair), &mut control).unwrap();
        assert_eq!(actual, control);
    }
}
#[test]
fn dynamic_pce_is_transactional_checkpointed_and_reset_to_initial_roster() {
    let m = manifest();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["dynamic"] == true)
    {
        let mut d = NativeAacDecoder::new(&asc(c)).unwrap();
        let encoded = packets(c);
        let mut baseline = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(c), &mut baseline).unwrap();
        for replay in 0..2 {
            if replay != 0 {
                d.reset();
            }
            let mut pcm = vec![];
            for (i, p) in encoded.iter().enumerate() {
                let saved = d.checkpoint();
                let mut bad = p.clone();
                bad.push(0);
                assert!(
                    d.decode_timed(&bad, i as i64 * 1024, 1024)
                        .unwrap_err()
                        .to_string()
                        .contains("trailing bytes")
                );
                let output = d.decode_timed(p, i as i64 * 1024, 1024).unwrap();
                d.restore(&saved).unwrap();
                assert_eq!(d.decode_timed(p, i as i64 * 1024, 1024).unwrap(), output);
                let frame = output.unwrap();
                assert_eq!(frame.pts, i as i64 * 1024);
                assert_eq!(frame.duration, 1024);
                pcm.extend(frame.samples.into_iter().flat_map(|s| s.to_le_bytes()));
            }
            assert!(d.finish().unwrap().is_none());
            assert_eq!(pcm, baseline);
        }
        if c["schedule"] == "arrival" {
            let control = m["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| {
                    p["object_type"] == c["object_type"]
                        && p["point"] == c["point"]
                        && p["tns"] == c["tns"]
                        && p["schedule"] == "arrival"
                        && p["dynamic"] == false
                })
                .unwrap();
            let foreign = NativeAacDecoder::new(&asc(control)).unwrap();
            assert!(
                d.restore(&foreign.checkpoint())
                    .unwrap_err()
                    .to_string()
                    .contains("checkpoint configuration mismatch")
            );
            d.reset();
            assert!(
                d.decode(&packets(control)[2])
                    .unwrap_err()
                    .to_string()
                    .contains("AAC coupling is absent from configured PCE")
            );
            assert_eq!(d.decode(&encoded[0]).unwrap().len(), 1024);
        }
    }
}
#[test]
fn dynamic_roster_playback_ranges_rewind_seek_preserve_exact_pcm() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn play(s: &mut dyn AudioStream) -> Vec<u8> {
        let mut d = s.make_decoder().unwrap();
        let mut pcm = vec![];
        while let Some(p) = s.next_packet().unwrap() {
            if let Some(f) = d.decode_packet(&p.data, p.pts, p.duration as u64).unwrap() {
                if let Some(out) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                    pcm.extend(out.data);
                }
            }
        }
        while let Some(f) = d.finish_packet().unwrap() {
            if let Some(out) = s.present_decoded(f.packet, f.source_pts).unwrap() {
                pcm.extend(out.data);
            }
        }
        pcm
    }
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["dynamic"] == true)
    {
        let data = video(c);
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let mut range = vec![];
        fvid::native_media::decode_mp4_aac_pcm_interval(
            &data,
            &mut range,
            Some((Duration::from_millis(100), Duration::from_millis(400))),
        )
        .unwrap();
        assert_eq!(range, full[2400 * 4..9600 * 4]);
        let mut s =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap();
        assert_eq!(play(&mut s), full);
        s.rewind();
        assert_eq!(play(&mut s), full);
        let landed = s.seek_to(4800);
        assert_eq!(play(&mut s), full[landed as usize * 4..]);
    }
}

#[test]
fn dependent_roster_oracle_distinguishes_coupling_before_and_after_target_tns() {
    let m = manifest();
    let gold = file("aac-pce-roster-pcm.f32le");
    let samples = |c: &Value| {
        let at = c["pcm_offset"].as_u64().unwrap() as usize;
        gold[at..at + c["pcm_bytes"].as_u64().unwrap() as usize]
            .chunks_exact(4)
            .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
            .collect::<Vec<_>>()
    };
    let mut count = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["point"] == 0 && c["tns"] == true)
    {
        let paired = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| {
                p["point"] == 1
                    && p["tns"] == true
                    && p["object_type"] == c["object_type"]
                    && p["schedule"] == c["schedule"]
                    && p["dynamic"] == c["dynamic"]
            })
            .unwrap();
        let error = samples(c)
            .iter()
            .zip(samples(paired))
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(
            error > 1e-6,
            "coupling point is invisible to TNS oracle: {}",
            c["name"]
        );
        count += 1;
    }
    assert_eq!(count, 8);
}
