//! Native in-band PS negotiation preserves explicit false and real payload presence.
use fvid::codec::aac_ps_native::NativePsAacDecoder;
use fvid::container::mp4::{Limits, Mp4Reader};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn original_unhinted_ps_video_accepts_payload_discovery_and_independent_stereo_pcm() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-inband-oracles.json"
    ))
    .unwrap();
    let pcm = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    for c in m["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(c["video"]["file"].as_str().unwrap());
        let mut reader =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let packets: Vec<_> = (0..3)
            .map(|i| {
                let mut b = vec![];
                reader.read_packet(track, i, &mut b).unwrap();
                b
            })
            .collect();
        for (key, asc, rate) in [
            ("Double", c["video"]["asc"].as_str().unwrap(), 48000),
            ("Core", c["asc_core"].as_str().unwrap(), 24000),
        ] {
            assert!(NativePsAacDecoder::new(&hex(asc)).is_err());
            let mut decoder = NativePsAacDecoder::new_with_in_band_ps(&hex(asc), rate).unwrap();
            assert!(!decoder.ps_detected());
            let mut frames = vec![];
            for (i, packet) in packets.iter().enumerate() {
                let saved = decoder.checkpoint();
                if i == 1 {
                    let mut malformed = packet.clone();
                    malformed.push(0xa5);
                    assert!(
                        decoder
                            .decode(&malformed)
                            .unwrap_err()
                            .to_string()
                            .contains("trailing bytes after PS AAC END")
                    );
                    assert!(!decoder.ps_detected());
                    assert_eq!(decoder.pending_frame_index(), Some(0));
                }
                let frame = decoder.decode(packet).unwrap();
                assert_eq!(decoder.ps_detected(), c["detected"][i].as_bool().unwrap());
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.ps_detected(), i > 1);
                assert_eq!(frame, decoder.decode(packet).unwrap());
                if let Some(f) = frame {
                    frames.push(f);
                }
            }
            frames.push(decoder.finish().unwrap().unwrap());
            assert_eq!(frames.len(), 3);
            let actual: Vec<_> = frames.iter().flat_map(|f| f.pcm.iter().copied()).collect();
            for (n, a) in actual.iter().enumerate() {
                let descriptor = &c["pcm"][key][n % 2];
                let off = descriptor[0].as_u64().unwrap() as usize + (n / 2) * 8;
                let e = f64::from_le_bytes(pcm[off..off + 8].try_into().unwrap()) as f32;
                assert!((a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16);
            }
            assert_eq!(frames[0].sample_rate, rate);
            assert_eq!(
                actual.len(),
                c["pcm"][key][0][1].as_u64().unwrap() as usize * 2
            );
            assert!(decoder.finish().unwrap().is_none());
            decoder.reset();
            assert!(!decoder.ps_detected());
            for (i, packet) in packets.iter().enumerate() {
                let f = decoder.decode(packet).unwrap();
                if i > 0 {
                    assert_eq!(f, Some(frames[i - 1].clone()));
                } else {
                    assert!(f.is_none());
                }
            }
            assert_eq!(decoder.finish().unwrap(), frames.last().cloned());
        }
    }
}

#[test]
fn in_band_candidate_honors_disabled_tools_and_does_not_classify_plain_mono_as_ps() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-inband-oracles.json"
    ))
    .unwrap();
    for row in manifest["explicit_false"].as_array().unwrap() {
        for key in ["ps_false", "sbr_false"] {
            let error = match NativePsAacDecoder::new_with_in_band_ps(
                &hex(row[key].as_str().unwrap()),
                48000,
            ) {
                Err(e) => e,
                Ok(_) => panic!("explicit false must be honored"),
            };
            assert!(error.to_string().contains("explicitly disabled"));
        }
    }
    let asc = hex(manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["kind"] == "LC" && c["slots"] == 16)
        .unwrap()["video"]["asc"]
        .as_str()
        .unwrap());
    let absence: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-absence-oracles.json"
    ))
    .unwrap();
    let mono = absence["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "all-mono" && c["slots"] == 16)
        .unwrap();
    let payload = include_bytes!("fixtures/playback-errors/he-aac-ps-absence-packets.bin");
    let mut candidate = NativePsAacDecoder::new_with_in_band_ps(&asc, 48000).unwrap();
    let mut normal =
        fvid::codec::aac_native::NativeAacDecoder::new_with_output_rate(&asc, 48000).unwrap();
    assert_eq!(normal.channels(), 1);
    for row in mono["frames"].as_array().unwrap() {
        let start = row["offset"].as_u64().unwrap() as usize;
        let end = start + row["bytes"].as_u64().unwrap() as usize;
        candidate.decode(&payload[start..end]).unwrap();
        assert!(!candidate.ps_detected());
        assert_eq!(normal.decode(&payload[start..end]).unwrap().len(), 2048);
    }
    let saved = candidate.checkpoint();
    let pending = candidate.pending_frame_index();
    assert!(
        candidate
            .finish()
            .unwrap_err()
            .to_string()
            .contains("without a PS element")
    );
    assert_eq!(candidate.pending_frame_index(), pending);
    assert!(!candidate.ps_detected());
    candidate.restore(&saved).unwrap();
    assert_eq!(candidate.pending_frame_index(), pending);
}

#[cfg(feature = "player")]
#[test]
fn unhinted_ps_playback_bridge_retains_delayed_signed_source_windows() {
    use fvid::codec::aac_ps_playback::PsAacDecoder;
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-inband-oracles.json"
    ))
    .unwrap();
    let reference = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    for case in manifest["cases"].as_array().unwrap() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(case["video"]["file"].as_str().unwrap());
        let mut reader =
            Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let config = reader.tracks()[track].configuration.clone();
        assert!(PsAacDecoder::new(&config, 48000, 2).is_err());
        assert!(PsAacDecoder::new_with_in_band_ps(&config, 48000, 1).is_err());
        let mut decoder = PsAacDecoder::new_with_in_band_ps(&config, 48000, 2).unwrap();
        let pts = [-480, 1440, 3360];
        let duration = [960, 720, 480];
        let mut frames = vec![];
        for i in 0..3 {
            let mut packet = vec![];
            reader.read_packet(track, i, &mut packet).unwrap();
            let saved = decoder.checkpoint().unwrap();
            let output = decoder.decode(&packet, pts[i], duration[i]).unwrap();
            decoder.restore(&saved).unwrap();
            let replay = decoder.decode(&packet, pts[i], duration[i]).unwrap();
            assert_eq!(
                output.as_ref().map(|f| &f.packet.data),
                replay.as_ref().map(|f| &f.packet.data)
            );
            if let Some(frame) = output {
                frames.push(frame);
            }
        }
        frames.push(decoder.finish().unwrap().unwrap());
        assert!(decoder.finish().unwrap().is_none());
        let mut offset = 0;
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(frame.source_pts, pts[i]);
            assert_eq!(frame.source_duration, duration[i]);
            assert_eq!(frame.frame_index, i as u64);
            for bytes in frame.packet.data.chunks_exact(4) {
                let actual = f32::from_le_bytes(bytes.try_into().unwrap());
                let descriptor = &case["pcm"]["Double"][offset % 2];
                let start = descriptor[0].as_u64().unwrap() as usize + (offset / 2) * 8;
                let expected =
                    f64::from_le_bytes(reference[start..start + 8].try_into().unwrap()) as f32;
                assert!((actual - expected).abs() <= 2. * f32::EPSILON * expected.abs() + 2e-16);
                offset += 1;
            }
        }
        assert_eq!(
            offset,
            case["pcm"]["Double"][0][1].as_u64().unwrap() as usize * 2
        );
    }
}
