//! Scalar Main/LC source clock and QMF references for dynamic PCE rosters.
use fvid_media::owned_aac::NativeAacDecoder;
use serde_json::Value;
use std::path::Path;
fn manifest() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-pce-roster-sbr.json"
    ))
    .unwrap()
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
fn video(c: &Value) -> Vec<u8> {
    file(c["video"]["file"].as_str().unwrap())
}
fn packets(c: &Value) -> Vec<Vec<u8>> {
    let raw = file("aac-pce-roster-sbr-packets.bin");
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
#[test]
fn main_lc_source_sbr_rosters_match_scalar_pcm_and_paused_qmf_clock() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 36);
    for c in m["cases"].as_array().unwrap() {
        let mut actual = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(c), &mut actual).unwrap();
        let expected = file(c["reference"].as_str().unwrap());
        assert_eq!(expected.len(), actual.len() * 2);
        assert!(
            expected
                .chunks_exact(8)
                .any(|s| f64::from_le_bytes(s.try_into().unwrap()).abs() > 1e-5)
        );
        for (i, (a, b)) in actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(8))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 1e-9, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        let ratio = c["container_rate"].as_u64().unwrap() as usize / 24000;
        for (i, on) in c["present"].as_array().unwrap().iter().enumerate() {
            if on == 0 {
                assert!(
                    actual[i * 1024 * ratio * 4..(i + 1) * 1024 * ratio * 4]
                        .chunks_exact(4)
                        .all(|s| f32::from_le_bytes(s.try_into().unwrap()) == 0.)
                );
            }
        }
        if c["dynamic"] == true {
            let name = c["name"].as_str().unwrap().replace("dynamic", "static");
            let pair = m["cases"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["name"] == name)
                .unwrap();
            let mut control = vec![];
            fvid::native_media::decode_mp4_aac_pcm(&video(pair), &mut control).unwrap();
            assert_eq!(actual, control);
        }
    }
}
#[test]
fn pce_source_sbr_histories_survive_bad_packets_checkpoints_reset_and_eof() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["dynamic"] == true)
    {
        let mut d = NativeAacDecoder::new_with_output_rate(
            &asc(c),
            c["container_rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let encoded = packets(c);
        let ticks = c["container_frame_samples"].as_u64().unwrap();
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
                    d.decode_timed(&bad, i as i64 * ticks as i64, ticks)
                        .unwrap_err()
                        .to_string()
                        .contains("trailing bytes")
                );
                let output = d.decode_timed(p, i as i64 * ticks as i64, ticks).unwrap();
                d.restore(&saved).unwrap();
                assert_eq!(
                    d.decode_timed(p, i as i64 * ticks as i64, ticks).unwrap(),
                    output
                );
                let f = output.unwrap();
                assert_eq!(f.pts, i as i64 * ticks as i64);
                assert_eq!(f.duration, ticks);
                pcm.extend(f.samples.into_iter().flat_map(|s| s.to_le_bytes()));
            }
            assert!(d.finish().unwrap().is_none());
            assert_eq!(pcm, baseline);
        }
    }
}
#[test]
fn source_sbr_roster_playback_ranges_rewind_seek_keep_exact_pcm() {
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
        let rate = c["container_rate"].as_u64().unwrap() as usize;
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let mut range = vec![];
        fvid::native_media::decode_mp4_aac_pcm_interval(
            &data,
            &mut range,
            Some((Duration::from_millis(20), Duration::from_millis(200))),
        )
        .unwrap();
        assert_eq!(range, full[rate / 50 * 4..rate / 5 * 4]);
        let mut s =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap();
        assert_eq!(play(&mut s), full);
        s.rewind();
        assert_eq!(play(&mut s), full);
        let landed = s.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut s), full[landed as usize * 4..]);
    }
}

#[test]
fn scalar_source_extension_oracle_detects_discarding_dsp_on_roster_removal() {
    let m = manifest();
    let mut count = 0;
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["discarded_sbr_reference"].is_string())
    {
        let good = file(c["reference"].as_str().unwrap());
        let wrong = file(c["discarded_sbr_reference"].as_str().unwrap());
        let error = good
            .chunks_exact(8)
            .zip(wrong.chunks_exact(8))
            .map(|(a, b)| {
                (f64::from_le_bytes(a.try_into().unwrap())
                    - f64::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f64, f64::max);
        assert!(
            error > 1e-7,
            "discarded source SBR DSP is invisible: {}",
            c["name"]
        );
        count += 1;
    }
    assert_eq!(count, 8);
}
