use serde_json::Value;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&bytes("aac-ltp-late-sbr.json")).unwrap()
}
fn asc(case: &Value) -> Vec<u8> {
    case["asc"]
        .as_str()
        .unwrap()
        .as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
fn pcm(case: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    fvid::native_media::decode_mp4_aac_pcm(
        &bytes(case["video"]["file"].as_str().unwrap()),
        &mut out,
    )
    .unwrap();
    out
}
#[test]
fn late_ltp_sbr_direct_and_independent_coupling_match_scalar_history() {
    for c in manifest()["cases"].as_array().unwrap() {
        let output = pcm(c);
        let gold = bytes(c["reference"].as_str().unwrap());
        assert_eq!(output.len() * 2, gold.len());
        for (i, (a, b)) in output.chunks_exact(4).zip(gold.chunks_exact(8)).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 1e-9, "{} sample {i}: {a} vs {b}", c["name"]);
        }
        let rate = c["rate"].as_u64().unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, output);
        if c["late"] == true {
            let begin = 4 * 1024 * (rate / 24000) as usize;
            for kind in ["cold-qmf-control", "inactive-ltp-control"] {
                let control = bytes(
                    c[if kind == "cold-qmf-control" {
                        "cold_control"
                    } else {
                        "inactive_control"
                    }]
                    .as_str()
                    .unwrap(),
                );
                let peak = output
                    .chunks_exact(4)
                    .zip(control.chunks_exact(8))
                    .skip(begin)
                    .map(|(a, b)| {
                        (f32::from_le_bytes(a.try_into().unwrap()) as f64
                            - f64::from_le_bytes(b.try_into().unwrap()))
                        .abs()
                    })
                    .fold(0f64, f64::max);
                assert!(
                    peak > 1e-7,
                    "{} {kind} must alter transition PCM: {peak}",
                    c["name"]
                );
            }
        }
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut selected = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
                &mut selected,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                selected,
                output[(from * rate / 1000) as usize * 4..(to * rate / 1000) as usize * 4]
            );
        }
    }
}
#[test]
fn late_ltp_sbr_checkpoint_reset_and_failure_keep_core_and_source_histories() {
    let blob = bytes("aac-ltp-late-sbr-packets.bin");
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["late"] == true)
    {
        let mut decoder = fvid_media::owned_aac::NativeAacDecoder::new_with_output_rate(
            &asc(c),
            c["rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let expected = pcm(c);
        for pass in 0..2 {
            if pass > 0 {
                decoder.reset();
            }
            let mut output = Vec::new();
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let checkpoint = decoder.checkpoint();
                assert!(decoder.decode_timed(&[], i as i64 * 1024, 1024).is_err());
                if i == 5 {
                    let bad = bytes(c["bad_packet"].as_str().unwrap());
                    let error = decoder
                        .decode_timed(&bad, i as i64 * 1024, 1024)
                        .unwrap_err();
                    assert!(error.to_string().contains("SBR CRC mismatch"), "{error}");
                }
                let first = decoder
                    .decode_timed(&blob[at..at + len], i as i64 * 1024, 1024)
                    .unwrap();
                decoder.restore(&checkpoint).unwrap();
                let again = decoder
                    .decode_timed(&blob[at..at + len], i as i64 * 1024, 1024)
                    .unwrap();
                assert_eq!(first, again);
                let frame = again.unwrap();
                assert_eq!(frame.pts, i as i64 * 1024);
                assert_eq!(frame.duration, 1024);
                assert_eq!(decoder.sample_rate(), c["rate"].as_u64().unwrap() as u32);
                output.extend(frame.samples.iter().flat_map(|v| v.to_le_bytes()));
            }
            assert!(decoder.finish().unwrap().is_none());
            assert!(decoder.finish().unwrap().is_none());
            assert_eq!(output, expected);
        }
    }
}
#[test]
fn late_ltp_adts_negotiation_replays_active_prefix_at_selected_clock() {
    let m = manifest();
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["adts"].is_string())
    {
        let data = bytes(c["adts"].as_str().unwrap());
        let expected = pcm(c);
        let mut output = Vec::new();
        let stats = fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!((stats.sample_rate, stats.sample_frames), (48000, 12288));
        assert_eq!(output, expected);
        let mut root = Vec::new();
        fvid::native_media::decode_aac_pcm_interval(&data, &mut root, &Default::default(), None)
            .unwrap();
        assert_eq!(root, expected);
        for (from, to, rate) in [
            (130, 230, 48000),
            (130, 160, 24000),
            (10, 60, 24000),
            (130, 230, 48000),
        ] {
            let mut selected = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut selected,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(stats.sample_rate, rate);
            let control = if rate == 48000 {
                expected.clone()
            } else {
                pcm(m["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|other| {
                        other["rate"] == 24000
                            && other["program"] == c["program"]
                            && other["late"] == false
                    })
                    .unwrap())
            };
            assert_eq!(
                selected,
                control[(from * rate as u64 / 1000) as usize * 4
                    ..(to * rate as u64 / 1000) as usize * 4]
            );
        }
    }
}

#[cfg(feature = "player")]
#[test]
fn late_ltp_sbr_player_rewind_and_seeks_on_each_side_keep_pcm() {
    use fvid::audio::AudioStream;
    fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = reader.make_decoder().unwrap();
        let mut output = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .unwrap()
            {
                if let Some(frame) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    output.extend(frame.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(frame) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(frame.data);
            }
        }
        output
    }
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["late"] == true)
    {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let expected = pcm(c);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(reader.sample_rate(), c["rate"].as_u64().unwrap() as u32);
        assert_eq!(play(&mut reader), expected);
        reader.rewind();
        assert_eq!(play(&mut reader), expected);
        let ratio = c["rate"].as_u64().unwrap() / 24000;
        for target in [
            1100,
            3900 * ratio,
            4300 * ratio,
            c["samples"].as_u64().unwrap(),
        ] {
            let landed = reader.seek_to(target as i64);
            assert_eq!(play(&mut reader), expected[landed as usize * 4..]);
        }
    }
}

#[test]
fn late_ltp_sbr_bad_crc_videos_report_exact_failure_after_active_history() {
    for c in manifest()["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["late"] == true)
    {
        let mut output = Vec::new();
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(c["bad_video"]["file"].as_str().unwrap()),
            &mut output,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("SBR CRC mismatch"),
            "{}: {error}",
            c["name"]
        );
    }
}
