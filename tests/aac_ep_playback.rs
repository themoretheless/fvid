use fvid::codec::aac_native::NativeAacDecoder;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-ep-playback")
}
#[test]
fn ep_nonzero_core_and_high_bands_match_independent_complete_pcm() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let golden: Vec<f32> = std::fs::read(root().join(c["golden"].as_str().unwrap()))
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
fn ep_videos_decode_through_public_and_owned_mp4_audio_paths() {
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
        let golden = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
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
fn ep_matroska_and_sample_ranges_preserve_complete_pcm() {
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
    reader.validate_eof().unwrap();
    output
}
#[cfg(feature = "player")]
#[test]
fn ep_player_rewind_and_sample_seek_preserve_pcm_in_both_containers() {
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
            for target in [13, 512, 2500, 7, (full.len() / width) as i64] {
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

#[test]
fn ep_checkpoint_replay_and_declared_length_rollback() {
    use fvid_media::owned_aac::{NativeAacDecoder as Owned, config::AudioSpecificConfig};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(config.sbr_present, Some(name.contains("-ld1-")));
        assert!(config.protection.is_some());
        assert_eq!(config.ps_present, Some(false));
        assert_eq!(
            config
                .eld_sbr
                .as_ref()
                .map(|s| s.output_rate(config.core.sample_rate).unwrap())
                .unwrap_or(config.core.sample_rate),
            c["rate"].as_u64().unwrap() as u32
        );
        let mut d = Owned::new(&asc).unwrap();
        for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
            let a = row["offset"].as_u64().unwrap() as usize;
            let raw = &packets[a..a + row["bytes"].as_u64().unwrap() as usize];
            let checkpoint = d.checkpoint();
            let retained = d
                .retained_payload_bytes_with_checkpoint(Some(&checkpoint))
                .unwrap();
            assert!(retained > d.retained_payload_bytes().unwrap());
            let declared = matches!(
                config.protection.as_ref().unwrap().sets[0].classes[0].length,
                fvid_media::owned_aac::aac_ep::Length::Fixed(_)
                    | fvid_media::owned_aac::aac_ep::Length::InBand(1..=15)
            );
            // An until-end class has no externally declared payload length;
            // small CRCs do not guarantee detection of arbitrary truncation.
            if i == 3 && declared {
                assert!(
                    d.decode(&raw[..raw.len() - 1]).is_err(),
                    "{name} shorter than declared EP class length accepted"
                );
            }
            let actual = d.decode(raw).unwrap();
            d.restore(&checkpoint).unwrap();
            assert_eq!(actual, d.decode(raw).unwrap(), "{name} checkpoint");
        }
    }
}

#[test]
fn ep_own_malformed_videos_fail_at_the_specific_ep_block() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["malformed"].as_array().unwrap() {
        let asc: Vec<u8> = c["asc"]
            .as_str()
            .unwrap()
            .as_bytes()
            .chunks_exact(2)
            .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
            .collect();
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        let rows = c["frames"].as_array().unwrap();
        for row in &rows[..3] {
            let a = row["offset"].as_u64().unwrap() as usize;
            d.decode(&packets[a..a + row["bytes"].as_u64().unwrap() as usize])
                .unwrap();
        }
        let row = &rows[3];
        let a = row["offset"].as_u64().unwrap() as usize;
        let error = d
            .decode(&packets[a..a + row["bytes"].as_u64().unwrap() as usize])
            .unwrap_err()
            .to_string();
        assert!(
            error
                .to_lowercase()
                .contains(&c["error"].as_str().unwrap().to_lowercase()),
            "{}: {error}",
            c["name"]
        );
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let error = fvid::native_media::decode_mp4_aac_pcm(&video, &mut Vec::new())
            .unwrap_err()
            .to_string();
        assert!(
            error
                .to_lowercase()
                .contains(&c["error"].as_str().unwrap().to_lowercase()),
            "container: {error}"
        );
    }
}

#[test]
fn ep_integrity_errors_preserve_decoder_history_and_configuration() {
    use fvid_media::owned_aac::NativeAacDecoder as Owned;
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    let bytes = std::fs::read(root().join("packets.bin")).unwrap();
    let packet = |row: &serde_json::Value| {
        let a = row["offset"].as_u64().unwrap() as usize;
        &bytes[a..a + row["bytes"].as_u64().unwrap() as usize]
    };
    for bad in m["malformed"].as_array().unwrap() {
        let name = bad["name"].as_str().unwrap();
        let good_name = name
            .replace("bad-crc", "stuffing")
            .replace("truncated", "length");
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == good_name)
            .unwrap();
        let asc = std::fs::read(root().join(format!("{name}.asc"))).unwrap();
        let mut decoder = Owned::new(&asc).unwrap();
        for row in &bad["frames"].as_array().unwrap()[..3] {
            decoder.decode(packet(row)).unwrap();
        }
        let saved = decoder.checkpoint();
        let error = decoder
            .decode(packet(&bad["frames"][3]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{name}: {error}"
        );
        let actual = decoder.decode(packet(&good["frames"][3])).unwrap();
        decoder.restore(&saved).unwrap();
        assert_eq!(actual, decoder.decode(packet(&good["frames"][3])).unwrap());
        let plain = std::fs::read(root().parent().unwrap().join("aac-eld/n512-c1.asc")).unwrap();
        let mut unprotected = Owned::new(&plain).unwrap();
        assert!(
            unprotected
                .restore(&saved)
                .unwrap_err()
                .to_string()
                .contains("EP checkpoint")
        );
    }
}
