use std::path::Path;
fn manifest() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/aac-ssr-ps.json")).unwrap()
}
fn artifact(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|s| u8::from_str_radix(std::str::from_utf8(s).unwrap(), 16).unwrap())
        .collect()
}
fn video(c: &serde_json::Value) -> Vec<u8> {
    artifact(c["video"]["file"].as_str().unwrap())
}
#[test]
fn ssr_ps_fixture_core_controls_accept_all_authored_window_schedules() {
    let m = manifest();
    assert_eq!(m["controls"].as_array().unwrap().len(), 3);
    for case in m["controls"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        assert_eq!(pcm.len(), 3072 * 4);
        assert!(pcm.iter().all(|s| *s == 0));
    }
}
#[test]
fn ssr_ps_payloads_reach_nonzero_independent_stereo_pcm_with_real_eof() {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps::Decoder, bits::BitReader};
    let m = manifest();
    for (rate, mode) in [(24000, OutputRate::Core), (48000, OutputRate::Double)] {
        let mut decoder = Decoder::default();
        let zero = vec![0.0; 1024];
        let mut actual = Vec::new();
        for payload in m["payloads"].as_array().unwrap() {
            let raw = hex(payload.as_str().unwrap());
            let mut bits = BitReader::new(&raw);
            let kind = bits.read(4).unwrap();
            assert!(matches!(kind, 13 | 14));
            if let Some(frame) = decoder
                .read(&mut bits, raw.len() * 8, kind == 14, &zero, 48000, 16, mode)
                .unwrap()
            {
                actual.extend(
                    frame.pcm[0]
                        .iter()
                        .zip(&frame.pcm[1])
                        .flat_map(|(&l, &r)| [l, r]),
                );
            }
        }
        while let Some(frame) = decoder.finish().unwrap() {
            actual.extend(
                frame.pcm[0]
                    .iter()
                    .zip(&frame.pcm[1])
                    .flat_map(|(&l, &r)| [l, r]),
            );
        }
        assert!(decoder.finish().unwrap().is_none());
        let gold = artifact(&format!("aac-ssr-ps-{rate}-reference.f64le"));
        assert_eq!(actual.len() * 8, gold.len());
        assert!(actual.iter().any(|v| v.abs() > 1e-6));
        for (i, (a, b)) in actual.iter().zip(gold.chunks_exact(8)).enumerate() {
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!((a - b).abs() < 5e-12, "sample {i}: {a} vs {b}");
        }
    }
}
#[test]
fn combined_ssr_ps_native_waveform_acceptance() {
    let m = manifest();
    for case in m["cases"].as_array().unwrap() {
        let mut pcm = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut pcm).unwrap();
        let gold = artifact(case["reference"].as_str().unwrap());
        assert_eq!(pcm.len(), case["samples"].as_u64().unwrap() as usize * 8);
        assert_eq!(gold.len(), pcm.len() * 2);
        for (i, (a, b)) in pcm.chunks_exact(4).zip(gold.chunks_exact(8)).enumerate() {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = f64::from_le_bytes(b.try_into().unwrap());
            assert!(
                (a - b).abs() < 1e-9,
                "{} sample {i}: {a} vs {b}",
                case["name"]
            );
        }
    }
}

fn packets(case: &serde_json::Value) -> Vec<Vec<u8>> {
    let blob = artifact("aac-ssr-ps-packets.bin");
    case["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let at = r["offset"].as_u64().unwrap() as usize;
            blob[at..at + r["bytes"].as_u64().unwrap() as usize].to_vec()
        })
        .collect()
}
#[test]
fn ssr_ps_native_and_bridge_preserve_two_delays_signed_timing_and_replay() {
    use fvid::codec::aac_ps_playback::PsAacDecoder;
    use fvid_media::owned_aac::aac_ps_native::NativePsAacDecoder;
    let m = manifest();
    for case in m["cases"].as_array().unwrap() {
        let asc = hex(case["asc"].as_str().unwrap());
        let rate = case["container_rate"].as_u64().unwrap() as u32;
        let n = case["container_frame_samples"].as_u64().unwrap();
        let mut native = NativePsAacDecoder::new(&asc).unwrap();
        let mut bridge =
            PsAacDecoder::new(&fvid::playback_aac::esds_for(&asc).unwrap(), rate, 2).unwrap();
        let mut pcm = vec![];
        for (i, packet) in packets(case).iter().enumerate() {
            let saved = native.checkpoint();
            let mut bad = packet.clone();
            bad.push(0);
            assert!(
                native
                    .decode(&bad)
                    .unwrap_err()
                    .to_string()
                    .contains("trailing bytes")
            );
            let output = native.decode(packet).unwrap();
            native.restore(&saved).unwrap();
            assert_eq!(native.decode(packet).unwrap(), output);
            assert_eq!(native.pending_frame_index(), Some(i as u64));
            assert_eq!(output.is_some(), i >= 2);
            let saved = bridge.checkpoint().unwrap();
            let pts = (i as i64 - 1) * n as i64;
            let duration = if i == 1 { n / 2 } else { n };
            let frame = bridge.decode(packet, pts, duration).unwrap();
            bridge.restore(&saved).unwrap();
            let replay = bridge.decode(packet, pts, duration).unwrap();
            assert_eq!(
                frame.as_ref().map(|f| &f.packet.data),
                replay.as_ref().map(|f| &f.packet.data)
            );
            if let Some(frame) = frame {
                assert_eq!(frame.frame_index, (i - 2) as u64);
                assert_eq!(frame.source_pts, -(n as i64));
                pcm.extend(frame.packet.data);
            }
        }
        let mut tails = 0;
        loop {
            let saved = native.checkpoint();
            let frame = native.finish().unwrap();
            native.restore(&saved).unwrap();
            assert_eq!(native.finish().unwrap(), frame);
            let saved = bridge.checkpoint().unwrap();
            let output = bridge.finish().unwrap();
            bridge.restore(&saved).unwrap();
            let replay = bridge.finish().unwrap();
            assert_eq!(
                output.as_ref().map(|f| &f.packet.data),
                replay.as_ref().map(|f| &f.packet.data)
            );
            assert_eq!(frame.is_some(), output.is_some());
            let Some(output) = output else {
                break;
            };
            assert_eq!(output.frame_index, 1 + tails);
            assert_eq!(output.source_pts, tails as i64 * n as i64);
            assert_eq!(output.source_duration, if tails == 0 { n / 2 } else { n });
            pcm.extend(output.packet.data);
            tails += 1;
        }
        assert_eq!(tails, 2);
        assert_eq!(bridge.pending_frame_index(), None);
        assert_eq!(native.pending_frame_index(), None);
        let mut root = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(case), &mut root).unwrap();
        assert_eq!(pcm, root);
        native.reset();
        bridge.reset();
        assert_eq!(native.pending_frame_index(), None);
        assert!(bridge.finish().unwrap().is_none());
        let first = &packets(case)[0];
        if case["name"].as_str().unwrap().starts_with("long-") {
            assert!(native.decode(first).unwrap().is_none());
            assert_eq!(native.finish().unwrap().unwrap().frame_index, 0);
            assert!(native.finish().unwrap().is_none());
        }
    }
}

#[test]
fn ssr_ps_mp4_matroska_adts_ranges_rewind_seek_and_both_eof_frames_accept() {
    use fvid::audio::AudioStream;
    use std::{io::Cursor, time::Duration};
    fn play(stream: &mut dyn AudioStream) -> Vec<u8> {
        let mut decoder = stream.make_decoder().unwrap();
        let mut out = vec![];
        loop {
            use fvid::audio::AudioStep;
            let decoded = match stream.next_step().unwrap() {
                Some(AudioStep::Encoded(p)) => decoder
                    .decode_packet(&p.data, p.pts, p.duration as u64)
                    .unwrap(),
                Some(AudioStep::Pcm(p)) => {
                    out.extend(p.data);
                    continue;
                }
                Some(AudioStep::ResetDecoder) => {
                    decoder.reset();
                    continue;
                }
                None => {
                    if !stream.drain_decoded_at_eof() {
                        decoder.reset();
                        stream.validate_eof().unwrap();
                        break;
                    }
                    let frame = decoder.finish_packet().unwrap();
                    if frame.is_none() {
                        stream.validate_eof().unwrap();
                        break;
                    }
                    frame
                }
            };
            if let Some(frame) = decoded {
                if let Some(pcm) = stream
                    .present_decoded(frame.packet, frame.source_pts)
                    .unwrap()
                {
                    out.extend(pcm.data);
                }
            }
        }
        out
    }
    let m = manifest();
    for case in m["cases"].as_array().unwrap() {
        let bytes = video(case);
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut full).unwrap();
        let rate = case["container_rate"].as_u64().unwrap();
        let interval = Some((Duration::from_millis(10), Duration::from_millis(100)));
        let a = (rate / 100) as usize * 8;
        let b = (rate / 10) as usize * 8;
        for _ in 0..2 {
            let mut range = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(&bytes, &mut range, interval).unwrap();
            assert_eq!(range, full[a..b]);
        }
        let mut stream =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!(play(&mut stream), full);
        stream.rewind();
        assert_eq!(play(&mut stream), full);
        let landed = stream.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut stream), full[landed as usize * 8..]);
        let edited = artifact(case["edited"].as_str().unwrap());
        let silence = vec![0u8; rate as usize / 30 * 8];
        let from = rate as usize / 50 * 8;
        let to = from + rate as usize / 15 * 8;
        let expected: Vec<u8> = silence
            .iter()
            .chain(full[from..to].iter())
            .chain(full[from..to].iter())
            .chain(silence.iter())
            .copied()
            .collect();
        let mut decoded = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&edited, &mut decoded).unwrap();
        assert_eq!(decoded, expected);
        let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
            Cursor::new(&edited),
            Default::default(),
        )
        .unwrap();
        assert_eq!(play(&mut stream), expected);
        stream.rewind();
        assert_eq!(play(&mut stream), expected);
        let landed = stream.seek_to((rate / 10) as i64);
        assert_eq!(play(&mut stream), expected[landed as usize * 8..]);
        let mka = artifact(case["matroska"].as_str().unwrap());
        let mut decoded = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mka, &mut decoded, None).unwrap();
        assert_eq!(decoded, full);
        let mut range = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mka, &mut range, interval).unwrap();
        assert_eq!(range, full[a..b]);
    }
    for case in m["controls"].as_array().unwrap() {
        let bytes = artifact(case["adts"].as_str().unwrap());
        let expected = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == format!("{}-explicit-48000", case["name"].as_str().unwrap()))
            .unwrap();
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&video(expected), &mut full).unwrap();
        let mut decoded = vec![];
        fvid::native_media::decode_aac_pcm(&bytes, &mut decoded, &Default::default()).unwrap();
        assert_eq!(decoded, full);
        let mut stream =
            fvid::playback_aac::AacAudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
        assert_eq!(play(&mut stream), full);
        stream.rewind();
        assert_eq!(play(&mut stream), full);
    }
}
