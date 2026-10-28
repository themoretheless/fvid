use std::{io::Cursor, path::PathBuf, time::Duration};
#[path = "mp4_audio_playback.rs"]
mod playback;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors")
        .join(CORPUS)
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
fn downscaled_ld_sbr_matches_independent_cosine_overlap_and_rejects_cross_mode_checkpoints() {
    use fvid_media::owned_aac::NativeAacDecoder;
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), CASES);
    let blob = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut short = NativeAacDecoder::new_with_output_rate(&asc, rate).unwrap();
        let mut full = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(short.sample_rate(), rate);
        assert_eq!(
            full.sample_rate(),
            c["source_rate"].as_u64().unwrap() as u32
        );
        assert_eq!(short.core_frame_samples(), c["n"].as_u64().unwrap() as u16);
        assert!(short.retained_payload_bytes().unwrap() > 0);
        let full_checkpoint = full.checkpoint();
        let short_checkpoint = short.checkpoint();
        assert!(
            short
                .restore(&full_checkpoint)
                .unwrap_err()
                .to_string()
                .contains("output geometry mismatch")
        );
        assert!(
            full.restore(&short_checkpoint)
                .unwrap_err()
                .to_string()
                .contains("output geometry mismatch")
        );

        let mut pcm = Vec::new();
        for r in c["frames"].as_array().unwrap() {
            let start = r["offset"].as_u64().unwrap() as usize;
            let end = start + r["bytes"].as_u64().unwrap() as usize;
            let save = short.checkpoint();
            let invalid = blob[start..end - 1].to_vec();
            assert!(short.decode(&invalid).is_err());
            if let Some(bits) = r["end_bits"].as_u64() {
                if bits < ((end - start) * 8) as u64 {
                    let mut bad_alignment = blob[start..end].to_vec();
                    *bad_alignment.last_mut().unwrap() |= 1;
                    assert!(
                        short
                            .decode(&bad_alignment)
                            .unwrap_err()
                            .to_string()
                            .contains("alignment")
                    );
                }
            }
            let f = short.decode(&blob[start..end]).unwrap();
            assert_eq!(
                f.len(),
                c["out_n"].as_u64().unwrap() as usize * c["channels"].as_u64().unwrap() as usize
            );
            short.restore(&save).unwrap();
            assert_eq!(short.decode(&blob[start..end]).unwrap(), f);
            pcm.extend(f);
            assert!(short.retained_payload_bytes().unwrap() < 8 * 1024 * 1024);
        }
        let gold = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(pcm.len() * 4, gold.len());
        assert!(
            gold.chunks_exact(4)
                .any(|b| f32::from_le_bytes(b.try_into().unwrap()).abs() > 1e-10),
            "{} oracle must contain nonzero PCM",
            c["name"]
        );
        let error = pcm
            .iter()
            .zip(gold.chunks_exact(4))
            .map(|(a, b)| (*a - f32::from_le_bytes(b.try_into().unwrap())).abs())
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} independent PCM error {error}", c["name"]);
        short.reset();
        let r = &c["frames"][0];
        let start = r["offset"].as_u64().unwrap() as usize;
        let f = short
            .decode(&blob[start..start + r["bytes"].as_u64().unwrap() as usize])
            .unwrap();
        assert_eq!(f, &pcm[..f.len()]);
    }
}
#[test]
fn downscaled_ld_sbr_mp4_matroska_exports_keep_duration_and_exact_ranges() {
    for c in manifest()["cases"].as_array().unwrap() {
        let video = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mkv = std::fs::read(root().join(c["matroska"].as_str().unwrap())).unwrap();
        let mut full = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&video, &mut full).unwrap();
        let mut owned = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&video),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(owned == full);
        owned.clear();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut owned, None).unwrap();
        assert!(owned == full);
        owned.clear();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(owned == full);
        let gold = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(full.len(), gold.len());
        for (a, b) in full.chunks_exact(4).zip(gold.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7,
                "{} exported PCM",
                c["name"]
            );
        }
        let rate = c["rate"].as_u64().unwrap();
        let width = c["channels"].as_u64().unwrap() as usize * 4;
        for (a, b) in [(2, 19), (7, 35), (2, 19)] {
            let interval = Some((Duration::from_millis(a), Duration::from_millis(b)));
            let first = ((a * rate).div_ceil(1000) as usize * width).min(full.len());
            let last = ((b * rate).div_ceil(1000) as usize * width).min(full.len());
            owned.clear();
            fvid::native_media::decode_mp4_aac_pcm_interval(&video, &mut owned, interval).unwrap();
            assert!(owned == full[first..last]);
            owned.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut owned, interval)
                .unwrap();
            assert!(owned == full[first..last]);
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn downscaled_ld_sbr_playback_frame_edge_seek_eof_and_rewind_match_pcm() {
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
            let mut decoder = source.make_decoder().unwrap();
            assert!(playback::play(source.as_mut(), &mut decoder) == full);
            let width = usize::from(source.channels()) * 4;
            let frame = c["out_n"].as_u64().unwrap() as usize;
            let end = full.len() / width;
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
                let start = (u128::try_from(landed).unwrap() * u128::from(source.sample_rate()))
                    .div_ceil(u128::from(source.timescale())) as usize
                    * width;
                assert!(
                    actual == full[start.min(full.len())..],
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

#[cfg(feature = "player")]
#[test]
fn downscaled_ld_sbr_ep1_groups_keep_core_clock_and_short_output_through_exports_and_seek() {
    use fvid::audio::AudioStream;
    use fvid_media::{owned_aac::NativeAacDecoder, owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    let m = manifest();
    assert_eq!(m["exports"].as_array().unwrap().len(), EXPORTS);
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
        let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), at).unwrap();
        let mut native = NativeAacDecoder::new_with_output_rate(
            &hex(c["asc"].as_str().unwrap()),
            c["rate"].as_u64().unwrap() as u32,
        )
        .unwrap();
        let mut typed = Vec::new();
        for i in 0..12 {
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let checkpoint = native.checkpoint();
            let f = native.decode_esc(&input).unwrap().unwrap();
            assert_eq!(
                f.samples.len(),
                c["out_n"].as_u64().unwrap() as usize * c["channels"].as_u64().unwrap() as usize
            );
            native.restore(&checkpoint).unwrap();
            assert_eq!(native.decode_esc(&input).unwrap().as_ref(), Some(&f));
            typed.extend(f.samples.into_iter().flat_map(f32::to_le_bytes));
        }
        assert!(typed == full);
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        assert!(playback::play(&mut source, &mut decoder) == full);
        let width = usize::from(source.channels()) * 4;
        let n = c["out_n"].as_u64().unwrap() as usize;
        let end = full.len() / width;
        for sample in [
            1,
            n - 1,
            n,
            n + 1,
            8 * n - 1,
            8 * n,
            8 * n + 1,
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
                * width;
            assert!(
                actual == full[start.min(full.len())..],
                "{} EP1 seek {sample}",
                c["name"]
            );
        }
        source.rewind();
        source.reset_decoder(&mut decoder).unwrap();
        assert!(playback::play(&mut source, &mut decoder) == full);
    }
}
#[test]
fn downscaled_ld_sbr_wave_crop_gain_and_resampling_agree_between_transports() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-eld-short-wave-{}-{}",
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
        let mut outputs = Vec::new();
        for (j, key) in ["video", "matroska"].iter().enumerate() {
            let path = dir.join(format!("{i}-{j}.wav"));
            fvid_media::decode_audio_transformed(
                &root().join(c[key].as_str().unwrap()),
                &path,
                fvid_media::AudioDecodeTransform {
                    sample_rate: Some(32000),
                    channels: Some(2),
                    volume: Some(0.5),
                    interval: Some((7000, 31000)),
                    ..Default::default()
                },
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(256 << 20),
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{} {key}: {e}", c["name"]));
            let data = std::fs::read(path).unwrap();
            let info =
                fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&data), None).unwrap();
            assert_eq!((info.sample_rate, info.channels), (32000, 2));
            outputs.push(
                data[info.data_offset as usize
                    ..info.data_offset as usize + info.data_bytes as usize]
                    .to_vec(),
            );
        }
        assert!(!outputs[0].is_empty());
        assert!(
            outputs[0] == outputs[1],
            "{} transformed transport PCM",
            c["name"]
        );
    }
}
