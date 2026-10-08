//! Mono PCE programs preserve configured tags and independent PS stereo PCM.
use fvid::codec::aac_ps_native::{InBandPsProbe, NativePsAacDecoder};
use fvid::container::mp4::{Limits, Mp4Reader};
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packets(name: &str) -> Vec<Vec<u8>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(name);
    let mut r = Mp4Reader::open(std::fs::File::open(path).unwrap(), Limits::default()).unwrap();
    let t = r
        .tracks()
        .iter()
        .position(|t| t.handler == *b"soun")
        .unwrap();
    (0..3)
        .map(|i| {
            let mut b = vec![];
            r.read_packet(t, i, &mut b).unwrap();
            b
        })
        .collect()
}
#[test]
fn configured_mono_pce_accepts_tagged_ps_packets_both_output_clocks_and_atomic_checkpoints() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-pce-oracles.json"
    ))
    .unwrap();
    let reference = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    for c in m["cases"].as_array().unwrap() {
        let packets = packets(c["video"]["file"].as_str().unwrap());
        for (mode, asc, rate) in [
            ("Double", c["video"]["asc"].as_str().unwrap(), 48000),
            ("Core", c["asc_core"].as_str().unwrap(), 24000),
        ] {
            let mut d = NativePsAacDecoder::new_with_in_band_ps(&hex(asc), rate).unwrap();
            let mut p = InBandPsProbe::new(&hex(asc), rate).unwrap();
            let mut frames = vec![];
            for (i, packet) in packets.iter().enumerate() {
                let saved = d.checkpoint();
                let f = d.decode(packet).unwrap();
                d.restore(&saved).unwrap();
                assert_eq!(f, d.decode(packet).unwrap());
                assert_eq!(p.read(packet).unwrap(), i > 0);
                assert_eq!(d.ps_detected(), i > 0);
                if let Some(f) = f {
                    frames.push(f);
                }
            }
            frames.push(d.finish().unwrap().unwrap());
            assert!(d.finish().unwrap().is_none());
            let actual: Vec<_> = frames.iter().flat_map(|f| f.pcm.iter().copied()).collect();
            assert_eq!(
                actual.len(),
                c["pcm"][mode][0][1].as_u64().unwrap() as usize * 2
            );
            for (n, a) in actual.iter().enumerate() {
                let descriptor = &c["pcm"][mode][n % 2];
                let off = descriptor[0].as_u64().unwrap() as usize + n / 2 * 8;
                let e = f64::from_le_bytes(reference[off..off + 8].try_into().unwrap()) as f32;
                assert!((a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16);
            }
            d.reset();
            for (i, packet) in packets.iter().enumerate() {
                assert_eq!(
                    d.decode(packet).unwrap(),
                    if i == 0 {
                        None
                    } else {
                        Some(frames[i - 1].clone())
                    }
                );
            }
            assert_eq!(d.finish().unwrap(), frames.last().cloned());
        }
        let mut d = NativePsAacDecoder::new_with_in_band_ps(
            &hex(c["video"]["asc"].as_str().unwrap()),
            48000,
        )
        .unwrap();
        d.decode(&packets[0]).unwrap();
        let state = d.checkpoint();
        let mut different = NativePsAacDecoder::new_with_in_band_ps(
            &hex(c["asc_wrong_tag"].as_str().unwrap()),
            48000,
        )
        .unwrap();
        assert!(
            different
                .restore(&state)
                .unwrap_err()
                .to_string()
                .contains("checkpoint configuration mismatch")
        );
        assert_eq!(different.pending_frame_index(), None);
    }
}
#[test]
fn invalid_pce_mapping_refuses_without_committing_pending_pcm_or_presence() {
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-pce-oracles.json"
    ))
    .unwrap();
    for c in m["invalid"].as_array().unwrap() {
        let packets = packets(c["video"]["file"].as_str().unwrap());
        let asc = hex(c["video"]["asc"].as_str().unwrap());
        let mut d = NativePsAacDecoder::new(&asc).unwrap();
        let mut p = InBandPsProbe::new(&asc, 48000).unwrap();
        d.decode(&packets[0]).unwrap();
        assert!(!p.read(&packets[0]).unwrap());
        let state = d.checkpoint();
        assert!(
            d.decode(&packets[1])
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        assert!(
            p.read(&packets[1])
                .unwrap_err()
                .to_string()
                .contains(c["error"].as_str().unwrap())
        );
        assert_eq!(d.pending_frame_index(), Some(0));
        assert!(!d.ps_detected());
        assert!(!p.ps_detected());
        d.restore(&state).unwrap();
        assert_eq!(d.pending_frame_index(), Some(0));
    }
}

#[cfg(feature = "player")]
#[test]
fn mono_pce_ps_videos_accept_owned_and_root_pcm_wav_and_reader_factories() {
    use fvid::audio::AudioStream;
    use fvid_control::CopyOptions;
    use std::io::Cursor;
    let m: serde_json::Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-pce-oracles.json"
    ))
    .unwrap();
    let reference = include_bytes!("fixtures/playback-errors/aac-ps-absence-pcm.bin");
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in m["cases"].as_array().unwrap() {
        let frames = c["slots"].as_u64().unwrap() as usize * 128;
        let mut pcm = vec![];
        for n in 0..frames * 3 {
            for ch in 0..2 {
                let off = c["pcm"]["Double"][ch][0].as_u64().unwrap() as usize + n * 8;
                pcm.push(f64::from_le_bytes(reference[off..off + 8].try_into().unwrap()) as f32);
            }
        }
        for matroska in [false, true] {
            let name = if matroska {
                c["matroska_file"].as_str().unwrap()
            } else {
                c["video"]["file"].as_str().unwrap()
            };
            let path = root.join(name);
            let bytes = std::fs::read(&path).unwrap();
            let expected = if matroska {
                let mut e = vec![0.; (4800 + frames) * 2];
                for i in 0..3 {
                    e[i * 2400 * 2..(i * 2400 + frames) * 2]
                        .copy_from_slice(&pcm[i * frames * 2..(i + 1) * frames * 2]);
                }
                e
            } else {
                pcm.clone()
            };
            let mut owned = vec![];
            let mut native = vec![];
            let stats = if matroska {
                let r = fvid::container::webm::WebmReader::open(
                    Cursor::new(&bytes),
                    Default::default(),
                )
                .unwrap();
                fvid::native_media::decode_matroska_aac_reader(r, &mut native, None).unwrap();
                let stream = fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(&bytes),
                    Default::default(),
                )
                .unwrap();
                assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
                stream.make_decoder().unwrap();
                fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                    Cursor::new(&bytes),
                    &mut owned,
                    None,
                    &CopyOptions::default(),
                )
                .unwrap()
            } else {
                fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut native).unwrap();
                let stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(&bytes),
                    Default::default(),
                )
                .unwrap();
                assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
                stream.make_decoder().unwrap();
                fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                    Cursor::new(&bytes),
                    &mut owned,
                    None,
                    &CopyOptions::default(),
                )
                .unwrap()
            };
            assert_eq!(
                (stats.sample_rate, stats.channels, stats.sample_frames),
                (48000, 2, (expected.len() / 2) as u64)
            );
            assert_eq!(native, owned);
            assert_eq!(owned.len(), expected.len() * 4);
            for (a, e) in owned.chunks_exact(4).zip(expected) {
                let a = f32::from_le_bytes(a.try_into().unwrap());
                assert!((a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16);
            }
            let destination =
                std::env::temp_dir().join(format!("fvid-ps-pce-{}-{name}.wav", std::process::id()));
            let _ = std::fs::remove_file(&destination);
            fvid_media::decode_audio(&path, &destination, &CopyOptions::default()).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(
                &mut std::fs::File::open(&destination).unwrap(),
                None,
            )
            .unwrap();
            assert_eq!(
                (info.sample_rate, info.channels, info.sample_frames),
                (48000, 2, stats.sample_frames)
            );
            std::fs::remove_file(destination).unwrap();
        }
    }
}
