use std::{io::Cursor, path::PathBuf};
#[path = "support/mp4_audio_playback.rs"]
mod playback;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-od-context")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn od_context_export_independent_pcm_silence_repeated_windows_and_exact_ranges() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 144);
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let golden = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        let mut pcm = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut pcm,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(pcm.len(), golden.len(), "{}", c["name"]);
        for (a, b) in pcm.chunks_exact(4).zip(golden.chunks_exact(4)) {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7,
                "{} independent PCM",
                c["name"]
            );
        }
        let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut reference = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&control),
            &mut reference,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(
            pcm == reference,
            "{} separate transport: lengths {} {}, first {:?}",
            c["name"],
            pcm.len(),
            reference.len(),
            pcm.iter().zip(&reference).position(|(a, b)| a != b)
        );
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        assert!(public == pcm);
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (a, b) in [(7, 31), (80, 190), (7, 31)] {
            let mut crop = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut crop,
                Some((
                    std::time::Duration::from_millis(a),
                    std::time::Duration::from_millis(b),
                )),
                &Default::default(),
            )
            .unwrap();
            assert!(
                crop == pcm[a as usize * c["rate"].as_u64().unwrap() as usize / 1000 * stride
                    ..(b as usize * c["rate"].as_u64().unwrap() as usize / 1000 * stride)
                        .min(pcm.len())],
                "{} crop",
                c["name"]
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn od_context_playback_seek_across_silence_crops_repetitions_eof_and_rewind() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        let mut decoder = source.make_decoder().unwrap();
        let full = playback::play(&mut source, &mut decoder);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(full == expected, "{} playback", c["name"]);
        let stride = usize::from(source.channels()) * 4;
        let n = (c["n"].as_u64().unwrap() * c["rate"].as_u64().unwrap() / 48000) as usize;
        let end = full.len() / stride;
        for sample in [
            0,
            n - 1,
            n,
            n + 1,
            8 * n - 1,
            8 * n,
            8 * n + 1,
            25 * n - 1,
            25 * n,
            25 * n + 1,
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
fn repeated_source_uses_each_od_domain_and_retains_strict_missing_au_errors() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    let m = manifest();
    assert_eq!(m["malformed"].as_array().unwrap().len(), 24);
    for c in m["malformed"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
        let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), at).unwrap();
        let chain: Vec<_> = (2..c["width"].as_u64().unwrap() as u32 + 2).collect();
        assert_eq!(
            reader
                .object_descriptor_prefix_in_edit(&chain, 0, 24000, Some(1))
                .unwrap(),
            Some(1)
        );
        assert_eq!(
            reader
                .object_descriptor_prefix_in_edit(&chain, 0, 24000, Some(2))
                .unwrap(),
            Some(c["width"].as_u64().unwrap() as usize)
        );
        assert!(reader
            .object_descriptor_prefix(&chain, 0, 24000)
            .unwrap_err()
            .to_string()
            .contains("presentation-dependent membership"));
        let asc = aac_stream_descriptor(&reader.tracks()[at].configuration)
            .unwrap()
            .asc
            .to_vec();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let before = decoder.checkpoint();
        let units = group.read_frame_in_edit(&mut reader, 0, Some(1)).unwrap();
        let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
        let first = decoder.decode_esc(&input).unwrap();
        let e = group
            .read_frame_in_edit(&mut reader, 0, Some(2))
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains(c["error"].as_str().unwrap()), "{e}");
        decoder.restore(&before).unwrap();
        let units = group.read_frame_in_edit(&mut reader, 0, Some(1)).unwrap();
        let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
        assert_eq!(decoder.decode_esc(&input).unwrap(), first);
        let mut output = Vec::new();
        let e = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains(c["error"].as_str().unwrap()),
            "{}: {e}",
            c["video"]
        );
        let samples =
            (25 * c["n"].as_u64().unwrap() * c["rate"].as_u64().unwrap() / 48000) as usize;
        let channels = if c["width"] == 4 { 1 } else { 8 };
        assert_eq!(
            output.len(),
            samples * channels * 4,
            "{} changed first showing",
            c["video"]
        );
    }
}
#[test]
fn authored_od_domains_cover_both_presentations_and_exact_fractional_edges() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    for c in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
        assert_eq!(reader.tracks()[at].edits.len(), 3);
        let od = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"odsm")
            .unwrap();
        assert_eq!(reader.tracks()[od].samples.len(), 6);
        let n = c["n"].as_i64().unwrap();
        for i in 0..6 {
            assert_eq!(
                reader.tracks()[od].samples.get(i).unwrap().pts,
                1 + i as i64 * 12 * n
            );
        }
        let refs = reader.tracks()[od].od_references.clone();
        let mut state = fvid_media::owned_mp4_od::State::default();
        for i in 0..6 {
            let mut packet = Vec::new();
            reader.read_packet(od, i, &mut packet).unwrap();
            state.apply(&packet, &refs).unwrap();
            assert_eq!(state.objects.len(), 1);
            assert_eq!(state.objects[0].id, if i < 3 { 2 } else { 3 });
        }
        let width = c["width"].as_u64().unwrap() as usize;
        let chain: Vec<_> = (2..width as u32 + 2).collect();
        for edit in [1, 2] {
            for (t, w) in [
                (8 * n - 1, 1),
                (8 * n, width),
                (16 * n - 1, width),
                (16 * n, 1),
            ] {
                assert_eq!(
                    reader
                        .object_descriptor_prefix_in_edit(&chain, t, 48000, Some(edit))
                        .unwrap(),
                    Some(w)
                );
            }
        }
        assert!(reader
            .object_descriptor_prefix_in_edit(&chain, 0, 24000, Some(0))
            .unwrap_err()
            .to_string()
            .contains("root edit is empty"));
        assert!(reader
            .object_descriptor_prefix_in_edit(&chain, 0, 24000, Some(3))
            .unwrap_err()
            .to_string()
            .contains("context is unavailable"));
        let group = Group::new(reader.tracks(), at).unwrap();
        for i in 0..12 {
            let a = group.read_frame_in_edit(&mut reader, i, Some(1)).unwrap();
            let b = group.read_frame_in_edit(&mut reader, i, Some(2)).unwrap();
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.data, b.data);
            }
        }
    }
}

#[test]
fn od_context_wave_gain_crop_and_resampling_match_separate_transport() {
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
    for (i, c) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
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
                    interval: Some((
                        25 * c["n"].as_i64().unwrap() * 1_000_000 / 48000 + 7000,
                        25 * c["n"].as_i64().unwrap() * 1_000_000 / 48000 + 31000,
                    )),
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
