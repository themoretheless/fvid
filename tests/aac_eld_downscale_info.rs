use std::{io::Cursor, path::PathBuf, time::Duration};
#[path = "support/mp4_audio_playback.rs"]
mod playback;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-eld-downscale-info")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn eld_downscale_info_preserves_full_rate_pcm_and_checkpoint_history() {
    use fvid_media::owned_aac::{config::AudioSpecificConfig, NativeAacDecoder};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 320);
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(
            config.eld_downscale_sample_rate,
            Some(c["hint"].as_u64().unwrap() as u32)
        );
        assert_eq!(
            config.output_sample_rate(),
            c["rate"].as_u64().unwrap() as u32
        );
        assert_eq!(
            config.core.sample_rate,
            c["core_rate"].as_u64().unwrap() as u32
        );
        let public = fvid::codec::config::AudioSpecificConfig::parse(&asc).unwrap();
        assert_eq!(
            public.eld_downscale_sample_rate,
            config.eld_downscale_sample_rate
        );
        let mut actual = NativeAacDecoder::new(&asc).unwrap();
        let mut plain = NativeAacDecoder::new(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        assert_eq!(actual.sample_rate(), c["rate"].as_u64().unwrap() as u32);
        let packets =
            std::fs::read(root().join(format!("../{}/packets.bin", c["source"].as_str().unwrap())))
                .unwrap();
        let mut pcm = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let start = row["offset"].as_u64().unwrap() as usize;
            let end = start + row["bytes"].as_u64().unwrap() as usize;
            let checkpoint = actual.checkpoint();
            let frame = actual.decode(&packets[start..end]).unwrap();
            assert_eq!(frame, plain.decode(&packets[start..end]).unwrap());
            actual.restore(&checkpoint).unwrap();
            assert_eq!(actual.decode(&packets[start..end]).unwrap(), frame);
            pcm.extend(frame);
        }
        let gold = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(pcm.len() * 4, gold.len());
        for (a, b) in pcm.iter().zip(gold.chunks_exact(4)) {
            assert!(
                (*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7,
                "{} independent PCM",
                c["name"]
            );
        }
    }
}
#[test]
fn eld_downscale_info_mp4_matroska_exports_and_repeated_ranges_keep_source_clock() {
    for c in manifest()["cases"].as_array().unwrap() {
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let mut decoded = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut decoded,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(decoded == full);
        decoded.clear();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut decoded, None).unwrap();
        assert!(decoded == full);
        decoded.clear();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut decoded,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(decoded == full);
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (a, b) in [(2, 19), (7, 35), (2, 19)] {
            let begin = ((a * rate).div_ceil(1000) as usize * stride).min(full.len());
            let end = ((b * rate).div_ceil(1000) as usize * stride).min(full.len());
            let interval = Some((Duration::from_millis(a), Duration::from_millis(b)));
            decoded.clear();
            fvid::native_media::decode_mp4_aac_pcm_interval(&mp4, &mut decoded, interval).unwrap();
            assert!(decoded == full[begin..end]);
            decoded.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut decoded, interval)
                .unwrap();
            assert!(decoded == full[begin..end]);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn eld_downscale_info_playback_frame_edge_seek_eof_and_rewind_keep_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let mp4 = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let mut sources: Vec<Box<dyn AudioStream>> = vec![
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
        for source in &mut sources {
            assert_eq!(source.sample_rate(), c["rate"].as_u64().unwrap() as u32);
            let stride = usize::from(source.channels()) * 4;
            let frame = c["container_frame_samples"].as_u64().unwrap() as usize;
            let mut decoder = source.make_decoder().unwrap();
            assert!(playback::play(source.as_mut(), &mut decoder) == full);
            let end = full.len() / stride;
            for sample in [
                0,
                1,
                frame - 1,
                frame,
                frame + 1,
                4 * frame - 1,
                4 * frame,
                4 * frame + 1,
                end,
                end + 1,
                7,
                7,
                0,
            ] {
                let ticks = (sample as u128 * u128::from(source.timescale())
                    / u128::from(source.sample_rate())) as i64;
                let landed = source.seek_to(ticks);
                source.reset_decoder(&mut decoder).unwrap();
                let actual = playback::play(source.as_mut(), &mut decoder);
                let begin = (u128::try_from(landed).unwrap() * u128::from(source.sample_rate()))
                    .div_ceil(u128::from(source.timescale())) as usize
                    * stride;
                assert!(
                    actual == full[begin.min(full.len())..],
                    "{} seek {sample}",
                    c["name"]
                );
            }
            source.rewind();
            source.reset_decoder(&mut decoder).unwrap();
            assert!(playback::play(source.as_mut(), &mut decoder) == full);
        }
    }
}
#[test]
fn eld_downscale_info_bounded_payloads_keep_specific_errors() {
    let m = manifest();
    assert_eq!(m["malformed"].as_array().unwrap().len(), 12);
    for c in m["malformed"].as_array().unwrap() {
        let e = fvid_media::owned_aac::config::AudioSpecificConfig::parse(&hex(c["asc"]
            .as_str()
            .unwrap()))
        .unwrap_err()
        .to_string();
        assert!(
            e.contains(c["error"].as_str().unwrap()),
            "{}: {e}",
            c["name"]
        );
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut out = Vec::new();
        let e = fvid::native_media::decode_mp4_aac_pcm(&video, &mut out)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains(c["error"].as_str().unwrap()),
            "{}: {e}",
            c["name"]
        );
        assert!(out.is_empty());
    }
}
#[cfg(feature = "player")]
#[test]
fn eld_downscale_info_ep1_typed_groups_exports_and_navigation_keep_pcm() {
    use fvid::audio::AudioStream;
    use fvid_media::{
        owned_aac::{config::AudioSpecificConfig, NativeAacDecoder},
        owned_mp4::Mp4Reader,
        owned_mp4_ep1::Group,
    };
    let m = manifest();
    assert_eq!(m["exports"].as_array().unwrap().len(), 16);
    for c in m["exports"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        let mut plain = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        fvid::native_media::decode_mp4_aac_pcm(&control, &mut plain).unwrap();
        assert!(full == plain);
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(owned == full);
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        assert!(config.separate_esc);
        assert_eq!(config.eld_downscale_sample_rate, Some(12000));
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut decoded = Vec::new();
        for i in 0..12 {
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let checkpoint = decoder.checkpoint();
            let f = decoder.decode_esc(&input).unwrap();
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(decoder.decode_esc(&input).unwrap(), f);
            decoded.extend(f.unwrap().samples.into_iter().flat_map(f32::to_le_bytes));
        }
        assert!(decoded == full);
        let golden = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(full.len(), golden.len());
        for (a, b) in full.chunks_exact(4).zip(golden.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        assert!(playback::play(&mut source, &mut decoder) == full);
        let stride = usize::from(source.channels()) * 4;
        let end = full.len() / stride;
        let frame = c["n"].as_u64().unwrap() as usize * source.sample_rate() as usize / 24000;
        for sample in [
            1,
            frame - 1,
            frame,
            frame + 1,
            8 * frame - 1,
            8 * frame,
            8 * frame + 1,
            end,
            end + 1,
            7,
            7,
            0,
        ] {
            let ticks = (sample as u128 * u128::from(source.timescale())
                / u128::from(source.sample_rate())) as i64;
            let landed = source.seek_to(ticks);
            source.reset_decoder(&mut decoder).unwrap();
            let actual = playback::play(&mut source, &mut decoder);
            let start = (u128::try_from(landed).unwrap() * u128::from(source.sample_rate()))
                .div_ceil(u128::from(source.timescale())) as usize
                * stride;
            assert!(
                actual == full[start.min(full.len())..],
                "{} seek {sample}",
                c["name"]
            );
        }
        source.rewind();
        source.reset_decoder(&mut decoder).unwrap();
        assert!(playback::play(&mut source, &mut decoder) == full);
    }
}

#[test]
fn eld_downscale_info_wave_gain_crop_and_resampling_match_separate_transport() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-od-membership-wave-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&dir).unwrap();
    struct Clean(PathBuf);
    impl Drop for Clean {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _clean = Clean(dir.clone());
    let m = manifest();
    for (i, c) in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .chain(m["exports"].as_array().unwrap())
        .enumerate()
    {
        let mut output = Vec::new();
        for (j, key) in ["video", "control"].iter().enumerate() {
            let path = dir.join(format!("{i}-{j}.wav"));
            fvid_media::decode_audio_transformed(
                &root().join(c[key].as_str().unwrap()),
                &path,
                fvid_media::AudioDecodeTransform {
                    sample_rate: Some(32000),
                    channels: Some(2),
                    volume: Some(0.5),
                    interval: Some((7000, 35000)),
                    ..Default::default()
                },
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(256 << 20),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
            let bytes = std::fs::read(path).unwrap();
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&bytes), None).unwrap();
            assert_eq!((info.sample_rate, info.channels), (32000, 2));
            output.push(
                bytes[info.data_offset as usize
                    ..info.data_offset as usize + info.data_bytes as usize]
                    .to_vec(),
            );
        }
        assert!(!output[0].is_empty());
        assert!(output[0] == output[1], "{} transformed PCM", c["name"]);
    }
}
