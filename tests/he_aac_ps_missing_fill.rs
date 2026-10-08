//! Native declared-PS streams accept absent/late PS as normative dual mono.
use fvid::{
    codec::aac_ps_native::NativePsAacDecoder,
    container::mp4::{Limits, Mp4Reader},
};
use serde_json::Value;
const PCM: &[u8] = include_bytes!("fixtures/playback-errors/aac-ps-missing-fill-pcm.bin");
fn m() -> Value {
    serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-missing-fill-oracles.json"
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
fn missing_sbr_fill_transitions_accept_independent_pcm_checkpoints_reset_and_eof() {
    let manifest = m();
    let authored = include_bytes!("fixtures/playback-errors/he-aac-ps-missing-fill-packets.bin");
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
                let mut malformed = p.clone();
                malformed.push(0xa5);
                assert!(
                    d.decode(&malformed)
                        .unwrap_err()
                        .to_string()
                        .contains("trailing bytes")
                );
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
    assert_eq!(count, 16);
}

#[test]
fn missing_fill_bridge_preserves_nonzero_core_pcm_against_direct_convolution() {
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps};
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    let manifest: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-sbr-upsampling-oracles.json"
    ))
    .unwrap();
    for c in manifest.as_array().unwrap() {
        let name = c["prefix"].as_str().unwrap();
        let input = std::fs::read(root.join(format!("{name}.f32le"))).unwrap();
        let input: Vec<f32> = input
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        let reference = std::fs::read(root.join(format!("{name}.f64le"))).unwrap();
        let slots = c["slots"].as_u64().unwrap() as u8;
        let mode = if c["bands"] == 64 {
            OutputRate::Double
        } else {
            OutputRate::Core
        };
        let mut d = aac_sbr_ps::Decoder::default();
        let mut frames = vec![];
        for p in input.chunks_exact(slots as usize * 64) {
            let before = d.clone();
            assert!(
                d.process_upsampling(&p[..p.len() - 1], 48000, slots, mode)
                    .is_err()
            );
            assert_eq!(d, before);
            if let Some(f) = d.process_upsampling(p, 48000, slots, mode).unwrap() {
                frames.push(f)
            }
        }
        frames.push(d.finish().unwrap().unwrap());
        assert!(d.finish().unwrap().is_none());
        assert!(!d.ps_seen());
        assert!(
            d.process_upsampling(&input[..slots as usize * 64], 48000, slots, mode)
                .is_err()
        );
        for channel in 0..2 {
            let values: Vec<f64> = frames
                .iter()
                .flat_map(|f| f.pcm[channel].iter().copied())
                .collect();
            assert_eq!(values.len() * 8, reference.len());
            assert!(values.iter().any(|v| v.abs() > 0.01));
            for (a, b) in values.iter().zip(reference.chunks_exact(8)) {
                let e = f64::from_le_bytes(b.try_into().unwrap());
                assert!((a - e).abs() < 2e-12, "{name}: {a} != {e}");
            }
        }
    }
}

#[test]
fn missing_fill_videos_accept_explicit_and_discovered_ps_in_pcm_and_wav_export() {
    use std::io::Cursor;
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in m()["cases"].as_array().unwrap() {
        for video in [&c["video"], &c["implicit_video"]] {
            if video.is_null() {
                continue;
            }
            let source = root.join(video["file"].as_str().unwrap());
            let bytes = std::fs::read(&source).unwrap();
            let mut owned = vec![];
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut owned,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!((stats.sample_rate, stats.channels), (48000, 2));
            let reader = Mp4Reader::open(Cursor::new(&bytes), Limits::default()).unwrap();
            let mut core = vec![];
            fvid::native_media::decode_mp4_aac_reader(reader, &mut core, None).unwrap();
            assert_eq!(owned, core);
            let l = values(&c["pcm"]["Double"][0]);
            let r = values(&c["pcm"]["Double"][1]);
            assert_eq!(owned.len(), l.len() * 8);
            for (i, b) in owned.chunks_exact(4).enumerate() {
                let a = f32::from_le_bytes(b.try_into().unwrap());
                let e = if i % 2 == 0 { l[i / 2] } else { r[i / 2] } as f32;
                assert!(
                    (a - e).abs() <= 2. * f32::EPSILON * e.abs() + 2e-16,
                    "{}",
                    source.display()
                );
            }
            #[cfg(feature = "player")]
            {
                use fvid::audio::AudioStream;
                let mut stream = fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(&bytes),
                    Limits::default(),
                )
                .unwrap();
                assert_eq!((stream.sample_rate(), stream.channels()), (48000, 2));
                let mut decoder = stream.make_decoder().unwrap();
                let mut rendered = vec![];
                let mut source_times = vec![];
                let mut returned = 0;
                while let Some(packet) = stream.next_packet().unwrap() {
                    let duration = u64::try_from(packet.duration).unwrap();
                    source_times.push((packet.pts, duration));
                    if let Some(frame) = decoder
                        .decode_packet(&packet.data, packet.pts, duration)
                        .unwrap()
                    {
                        assert_eq!(
                            (frame.source_pts, frame.source_duration),
                            source_times[returned]
                        );
                        returned += 1;
                        rendered.extend(frame.packet.data);
                    }
                }
                let tail = decoder.finish_packet().unwrap().unwrap();
                assert_eq!(
                    (tail.source_pts, tail.source_duration),
                    source_times[returned]
                );
                rendered.extend(tail.packet.data);
                assert!(decoder.finish_packet().unwrap().is_none());
                assert_eq!(rendered, owned);
            }
            let dest = std::env::temp_dir().join(format!(
                "fvid-ps-missing-{}-{}.wav",
                std::process::id(),
                video["file"].as_str().unwrap()
            ));
            fvid_media::decode_audio(&source, &dest, &Default::default()).unwrap();
            let info = fvid_media::owned_wave_inspect::inspect(
                &mut std::fs::File::open(&dest).unwrap(),
                None,
            )
            .unwrap();
            assert_eq!(
                (info.sample_rate, info.channels, info.sample_frames),
                (48000, 2, l.len() as u64)
            );
            std::fs::remove_file(dest).unwrap();
        }
    }
}

#[test]
fn missing_fill_does_not_fabricate_ps_detection_or_allow_candidate_eof_without_ps() {
    let manifest = m();
    let c = manifest["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "all-missing" && c["slots"] == 16)
        .unwrap();
    let mut asc = hex(c["video"]["asc"].as_str().unwrap());
    // Identical explicit SBR core; AOT5 does not signal PS whereas AOT29 does.
    asc[0] = (asc[0] & 7) | (5 << 3);
    let mut decoder = NativePsAacDecoder::new_with_in_band_ps(&asc, 48000).unwrap();
    let raw = include_bytes!("fixtures/playback-errors/he-aac-ps-missing-fill-packets.bin");
    for row in c["frames"].as_array().unwrap() {
        let start = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        decoder.decode(&raw[start..start + len]).unwrap();
        assert!(!decoder.ps_detected());
    }
    assert_eq!(decoder.pending_frame_index(), Some(2));
    assert!(
        decoder
            .finish()
            .unwrap_err()
            .to_string()
            .contains("without a PS element")
    );
    assert_eq!(decoder.pending_frame_index(), Some(2));
}

#[test]
fn former_missing_fill_refusal_video_now_accepts_and_preserves_original_frame_indices() {
    let manifest: Value = serde_json::from_str(include_str!(
        "fixtures/playback-errors/aac-ps-native-oracles.json"
    ))
    .unwrap();
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors");
    for c in manifest["accepted_missing_fill"].as_array().unwrap() {
        let mut reader = Mp4Reader::open(
            std::fs::File::open(root.join(c["video"]["file"].as_str().unwrap())).unwrap(),
            Limits::default(),
        )
        .unwrap();
        let track = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"soun")
            .unwrap();
        let mut decoder =
            NativePsAacDecoder::new(&hex(c["video"]["asc"].as_str().unwrap())).unwrap();
        for n in 0..3 {
            let mut packet = vec![];
            reader.read_packet(track, n, &mut packet).unwrap();
            let frame = decoder.decode(&packet).unwrap();
            if n == 0 {
                assert!(frame.is_none());
            } else {
                assert_eq!(frame.unwrap().frame_index, (n - 1) as u64);
            }
        }
        assert_eq!(decoder.finish().unwrap().unwrap().frame_index, 2);
        assert!(decoder.finish().unwrap().is_none());
    }
}
