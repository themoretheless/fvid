use fvid::codec::aac_native::NativeAacDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-eld-esc")
}
#[test]
fn eld_esc_nonzero_core_matches_independent_windowed_cosine_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let golden: Vec<f32> = std::fs::read(root().join(format!("{name}.f32le")))
            .unwrap()
            .chunks_exact(4)
            .map(|p| f32::from_le_bytes(p.try_into().unwrap()))
            .collect();
        let mut d = NativeAacDecoder::new(&asc).unwrap_or_else(|e| panic!("{name}: {e}"));
        for _ in 0..2 {
            let mut actual = Vec::new();
            for row in c["frames"].as_array().unwrap() {
                let a = row["offset"].as_u64().unwrap() as usize;
                let b = a + row["bytes"].as_u64().unwrap() as usize;
                actual.extend(d.decode(&packets[a..b]).unwrap());
            }
            assert_eq!(actual.len(), golden.len());
            let worst = actual
                .iter()
                .zip(&golden)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(worst < 1e-7, "{name}: PCM error {worst}");
            assert!(actual.iter().any(|&v| v.abs() > 1e-5));
            d.reset();
        }
    }
}

#[test]
fn eld_esc_videos_decode_through_public_and_owned_mp4_audio_paths() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut public = Vec::new();
        let mut owned = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(public, owned);
        let golden = std::fs::read(root().join(format!("{name}.f32le"))).unwrap();
        assert_eq!(public.len(), golden.len());
        let worst = public
            .chunks_exact(4)
            .zip(golden.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(worst < 1e-7, "{name}: public PCM error {worst}");
    }
}

#[test]
fn eld_esc_history_checkpoint_and_late_channel_error_are_transactional_and_counted() {
    use fvid_media::owned_aac::NativeAacDecoder as Owned;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let mut d = Owned::new(&asc).unwrap();
        let config = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(config.sbr_present, Some(false));
        assert_eq!(config.ps_present, Some(false));
        assert!(
            config
                .resolve_output_rate(config.core.sample_rate * 2)
                .is_err()
        );
        let retained = d.retained_payload_bytes().unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let a = row["offset"].as_u64().unwrap() as usize;
            let raw = &packets[a..a + row["bytes"].as_u64().unwrap() as usize];
            let saved = d.checkpoint();
            let joint = d
                .retained_payload_bytes_with_checkpoint(Some(&saved))
                .unwrap();
            assert!(
                joint
                    >= retained
                        + 3 * c["n"].as_u64().unwrap() as usize
                            * c["channels"].as_u64().unwrap() as usize
                            * 8
            );
            if i == 3 {
                assert!(
                    d.decode(&raw[..raw.len() - 1]).is_err(),
                    "{name} truncated late channel"
                );
            }
            let actual = d
                .decode_timed(
                    raw,
                    (i as i64 - 1) * i64::from(config.core.frame_samples),
                    u64::from(config.core.frame_samples),
                )
                .unwrap()
                .unwrap();
            assert_eq!(
                actual.pts,
                (i as i64 - 1) * i64::from(config.core.frame_samples)
            );
            assert_eq!(actual.duration, u64::from(config.core.frame_samples));
            d.restore(&saved).unwrap();
            assert_eq!(actual.samples, d.decode(raw).unwrap(), "{name} rollback");
            assert_eq!(retained, d.retained_payload_bytes().unwrap());
        }
    }
}
#[test]
fn eld_esc_matroska_and_sample_ranges_preserve_complete_pcm() {
    use std::{io::Cursor, time::Duration};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video, &mut full).unwrap();
        let mut decoded = Vec::new();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut decoded, None)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(decoded, full, "{name} Matroska PCM");
        let mut owned = Vec::new();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full);
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(7u64, 35u64), (2, 19), (7, 35)] {
            let interval = Some((Duration::from_millis(from), Duration::from_millis(to)));
            let first = (from * rate).div_ceil(1000) as usize;
            let last = (to * rate).div_ceil(1000) as usize;
            let end = (last * width).min(full.len());
            let begin = (first * width).min(end);
            let mut out = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm_interval(&video, &mut out, interval).unwrap();
            assert_eq!(out, &full[begin..end], "{name} MP4 range");
            out.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, interval).unwrap();
            assert_eq!(out, &full[begin..end], "{name} Matroska range");
        }
    }
}

#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = vec![];
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}
#[cfg(feature = "player")]
#[test]
fn eld_esc_player_rewind_and_sample_seek_preserve_pcm_in_both_containers() {
    use fvid::audio::AudioStream;
    use std::io::Cursor;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        let mut readers: Vec<Box<dyn AudioStream>> = vec![
            Box::new(
                fvid::playback_mp4_audio::Mp4AudioReader::open(
                    Cursor::new(mp4),
                    Default::default(),
                )
                .unwrap(),
            ),
            Box::new(
                fvid::playback_webm_audio::WebmAudioReader::open(
                    Cursor::new(mkv),
                    Default::default(),
                )
                .unwrap(),
            ),
        ];
        for (container, reader) in readers.iter_mut().enumerate() {
            assert_eq!(play(reader.as_mut()), full, "{name} container {container}");
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full, "{name} rewind {container}");
            for target in [
                -1,
                0,
                13,
                c["n"].as_i64().unwrap() - 1,
                c["n"].as_i64().unwrap(),
                c["n"].as_i64().unwrap() + 1,
                512,
                2500,
                7,
                7,
                (full.len() / width) as i64,
                (full.len() / width) as i64 * 2,
                13,
                0,
            ] {
                let ticks = (i128::from(target) * i128::from(reader.timescale())
                    / i128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                assert!(landed >= 0);
                assert_eq!(
                    play(reader.as_mut()),
                    full[(((landed as u128 * u128::from(reader.sample_rate()))
                        .div_ceil(u128::from(reader.timescale()))
                        as usize)
                        * width)
                        .min(full.len())..],
                    "{name} container {container} seek {target} landed {landed}"
                );
            }
        }
    }
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
fn packet<'a>(blob: &'a [u8], row: &serde_json::Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn semantic_esc_mapping_recovers_exact_logical_aus_and_respects_each_bit_boundary() {
    use fvid_media::owned_aac::{aac_esc::reassemble_eld_core, config::AudioSpecificConfig};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let plain = std::fs::read(root().join("plain-packets.bin")).unwrap();
    let instances = std::fs::read(root().join("instances.bin")).unwrap();
    for n in [480, 512] {
        for tool in ["tns", "pns", "correlated-pns", "intensity"] {
            for flags in 0..8 {
                let suffix = format!("-{tool}-f{flags}");
                assert!(
                    m["cases"].as_array().unwrap().iter().any(|c| c["n"] == n
                        && c["flags"] == flags
                        && c["name"].as_str().unwrap().ends_with(&suffix)),
                    "missing {n} {suffix}"
                );
            }
        }
    }
    for c in m["cases"].as_array().unwrap() {
        let parsed = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap())).unwrap();
        assert!(parsed.direct_mapping);
        let flags = c["flags"].as_u64().unwrap();
        assert_eq!(parsed.core.section_data_resilience, flags & 4 != 0);
        assert_eq!(parsed.core.scalefactor_data_resilience, flags & 2 != 0);
        assert_eq!(parsed.core.spectral_data_resilience, flags & 1 != 0);
        assert_eq!(parsed.core.frame_samples as u64, c["n"].as_u64().unwrap());
        assert_eq!(parsed.core.sample_rate as u64, c["rate"].as_u64().unwrap());
        let ep = parsed.protection.as_ref().unwrap();
        for row in c["frames"].as_array().unwrap() {
            let frames = ep.decode_class_frames(packet(&blob, row)).unwrap();
            let frame = &frames[0];
            assert_eq!(frame.set_index, row["choice"].as_u64().unwrap() as usize);
            assert_eq!(
                frame.classes.len(),
                row["instances"].as_array().unwrap().len()
            );
            for (index, (class, source)) in frame
                .classes
                .iter()
                .zip(row["instances"].as_array().unwrap())
                .enumerate()
            {
                let a = source["offset"].as_u64().unwrap() as usize;
                let n = source["bits"].as_u64().unwrap() as usize;
                assert_eq!(class.class_index, index);
                assert_eq!(
                    class.bits,
                    (0..n)
                        .map(|i| (instances[a + i / 8] >> (7 - i % 8)) & 1)
                        .collect::<Vec<_>>()
                );
            }
            let a = row["plain_offset"].as_u64().unwrap() as usize;
            let b = a + row["plain_bytes"].as_u64().unwrap() as usize;
            assert_eq!(
                reassemble_eld_core(&parsed.core, frame).unwrap(),
                plain[a..b],
                "{}",
                c["name"]
            );
        }
        let row = &c["frames"][0];
        let frame = ep
            .decode_class_frames(packet(&blob, row))
            .unwrap()
            .remove(0);
        let side = row["instances"]
            .as_array()
            .unwrap()
            .iter()
            .position(|c| c["category"] == 1)
            .unwrap();
        let mut bad = frame.clone();
        bad.classes[side].bits.truncate(7);
        assert!(
            reassemble_eld_core(&parsed.core, &bad)
                .unwrap_err()
                .to_string()
                .contains("truncated or oversized bit field")
        );
        let mut bad = frame.clone();
        bad.classes.push(bad.classes.last().unwrap().clone());
        assert!(
            reassemble_eld_core(&parsed.core, &bad)
                .unwrap_err()
                .to_string()
                .contains("unconsumed AAC ESC instances")
        );
        let mut aligned = frame.clone();
        let count = frame.classes.iter().map(|c| c.bits.len()).sum::<usize>();
        let padding = (8 - count % 8) % 8;
        aligned
            .classes
            .last_mut()
            .unwrap()
            .bits
            .extend(vec![0; padding]);
        assert_eq!(
            reassemble_eld_core(&parsed.core, &aligned).unwrap(),
            reassemble_eld_core(&parsed.core, &frame).unwrap()
        );
        if padding > 0 {
            *aligned.classes.last_mut().unwrap().bits.last_mut().unwrap() = 1;
            assert!(
                reassemble_eld_core(&parsed.core, &aligned)
                    .unwrap_err()
                    .to_string()
                    .contains("nonzero AAC ESC alignment bits")
            );
        }
    }
}
#[test]
fn mapping_identity_is_checkpoint_metadata_and_reserved_direct_mapping_stays_invalid() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    for c in manifest()["cases"].as_array().unwrap() {
        let mut ep3 = Owned::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let ep2 = Owned::new(&hex(c["ep2_asc"].as_str().unwrap())).unwrap();
        assert!(
            ep3.restore(&ep2.checkpoint())
                .unwrap_err()
                .to_string()
                .contains("EP checkpoint configuration mismatch")
        );
        assert!(
            AudioSpecificConfig::parse(&hex(c["reserved_asc"].as_str().unwrap()))
                .unwrap_err()
                .to_string()
                .contains("reserved AAC EP3 directMapping")
        );
    }
}
#[test]
fn own_malformed_ep3_movies_fail_at_crc_or_the_bounded_side_instance_and_roll_back() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let m = manifest();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut d = Owned::new(&asc).unwrap();
        let mut prefix = Vec::new();
        for row in &c["frames"].as_array().unwrap()[..3] {
            prefix.extend(d.decode(packet(&blob, row)).unwrap());
        }
        let saved = d.checkpoint();
        let row = &c["frames"][3];
        let error = d.decode(packet(&blob, row)).unwrap_err().to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        if c["name"].as_str().unwrap().ends_with("short-side") {
            let ep = AudioSpecificConfig::parse(&asc)
                .unwrap()
                .protection
                .unwrap();
            assert!(ep.decode_class_frames(packet(&blob, row)).is_ok());
        }
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|good| good["name"] == c["good"])
            .unwrap();
        let raw = packet(&blob, &good["frames"][3]);
        let output = d.decode(raw).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(output, d.decode(raw).unwrap());
        prefix.extend(output);
        for row in &good["frames"].as_array().unwrap()[4..] {
            prefix.extend(d.decode(packet(&blob, row)).unwrap());
        }
        let golden =
            std::fs::read(root().join(format!("{}.f32le", good["name"].as_str().unwrap())))
                .unwrap();
        assert_eq!(prefix.len() * 4, golden.len());
        assert!(
            prefix
                .iter()
                .zip(golden.chunks_exact(4))
                .all(|(a, b)| (*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7)
        );
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        for error in [
            fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut vec![])
                .unwrap_err()
                .to_string(),
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut vec![], None)
                .unwrap_err()
                .to_string(),
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                std::io::Cursor::new(&mp4),
                &mut vec![],
                None,
                &Default::default(),
            )
            .unwrap_err()
            .to_string(),
            fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
                std::io::Cursor::new(&mkv),
                &mut vec![],
                None,
                &Default::default(),
            )
            .unwrap_err()
            .to_string(),
        ] {
            assert!(
                error.contains(c["error"].as_str().unwrap()),
                "{}: {error}",
                c["name"]
            );
        }
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut readers: Vec<Box<dyn AudioStream>> = vec![
                Box::new(
                    fvid::playback_mp4_audio::Mp4AudioReader::open(
                        std::io::Cursor::new(mp4),
                        Default::default(),
                    )
                    .unwrap(),
                ),
                Box::new(
                    fvid::playback_webm_audio::WebmAudioReader::open(
                        std::io::Cursor::new(mkv),
                        Default::default(),
                    )
                    .unwrap(),
                ),
            ];
            for reader in &mut readers {
                let mut decoder = reader.make_decoder().unwrap();
                let mut decoded = 0;
                let error = loop {
                    let packet = reader
                        .next_packet()
                        .unwrap()
                        .expect("malformed stream reached EOF without failure");
                    match decoder.decode_packet(&packet.data, packet.pts, packet.duration as u64) {
                        Err(error) => break error.to_string(),
                        Ok(Some(frame)) => {
                            if reader
                                .present_decoded(frame.packet, frame.source_pts)
                                .unwrap()
                                .is_some()
                            {
                                decoded += 1;
                            }
                        }
                        Ok(None) => {}
                    }
                };
                assert_eq!(decoded, 3);
                assert!(
                    error.contains(c["error"].as_str().unwrap()),
                    "{} player: {error}",
                    c["name"]
                );
            }
        }
    }
}

#[test]
fn missing_ld_sbr_esc_instances_remain_an_explicit_error() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let m = manifest();
    let case = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["ld_sbr_asc"].is_string())
        .unwrap();
    let asc = hex(case["ld_sbr_asc"].as_str().unwrap());
    let parsed = AudioSpecificConfig::parse(&asc).unwrap();
    assert!(parsed.direct_mapping && parsed.eld_sbr.is_some());
    let mut d = Owned::new(&asc).unwrap();
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    let error = d
        .decode(packet(&blob, &case["frames"][0]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("missing AAC ESC instance"));
}
