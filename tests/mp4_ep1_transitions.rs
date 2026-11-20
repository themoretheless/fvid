use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(
        &std::fs::read(root().join("mp4-ep1-transitions/manifest.json")).unwrap(),
    )
    .unwrap()
}
fn decode(data: &[u8]) -> Vec<u8> {
    let mut pcm = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(data),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    pcm
}
#[test]
fn ordinary_separate_ordinary_epochs_preserve_complete_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let reference = std::fs::read(
            root()
                .join(if c["local_control"] == true {
                    "mp4-ep1-transitions"
                } else {
                    "mp4-ep1-epochs"
                })
                .join(c["control"].as_str().unwrap()),
        )
        .unwrap();
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-transitions")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let expected = decode(&reference);
        assert_eq!(decode(&bytes), expected, "{}", c["name"]);
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut public).unwrap();
        assert_eq!(public, expected);
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        let a = c["edges"][0].as_u64().unwrap() / 48;
        let b = c["edges"][1].as_u64().unwrap() / 48;
        for (from, to) in [(a - 1, a + 1), (b - 1, b + 1), (7, 31), (a - 1, a + 1)] {
            let mut cropped = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut cropped,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                cropped,
                expected[from as usize * 48 * stride..to as usize * 48 * stride],
                "{} crop {from}..{to}",
                c["name"]
            );
        }
    }
}

#[test]
fn transport_selection_distinguishes_missing_inactive_and_missing_active_escs() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 64);
    for c in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-transitions")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let tracks = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(tracks.len(), 1);
        let group = Group::for_root(reader.tracks(), tracks[0])
            .unwrap()
            .unwrap();
        assert!(
            Group::admission_upper_bound(reader.tracks(), tracks[0]).unwrap()
                >= group.retained_payload_bytes().unwrap()
        );
        let auxiliary = if c["mode"] == "sparse-aux" {
            if c["reverse"] == true { 12 } else { 6 }
        } else {
            18
        };
        for (index, t) in reader
            .tracks()
            .iter()
            .enumerate()
            .filter(|(_, t)| t.handler == *b"soun")
        {
            assert_eq!(
                t.samples.len(),
                if index == tracks[0] { 18 } else { auxiliary }
            );
        }
        for i in 0..18 {
            let protected = if c["reverse"] == true {
                i / 6 != 1
            } else {
                i / 6 == 1
            };
            assert_eq!(group.requires_esc(reader.tracks(), i).unwrap(), protected);
            if protected {
                assert_eq!(
                    group.read_frame(&mut reader, i).unwrap().len(),
                    c["streams"].as_u64().unwrap() as usize
                );
            } else {
                let e = group.read_frame(&mut reader, i).err().unwrap().to_string();
                assert!(e.contains("ordinary AAC transport"), "{e}");
            }
        }
    }
    for c in m["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-transitions")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let result = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default());
        if c["parser"] == true {
            let error = result
                .err()
                .expect("unsupplied dependent track must refuse indexing")
                .to_string();
            assert!(error.contains(c["error"].as_str().unwrap()), "{error}");
            continue;
        }
        let mut reader = result.unwrap();
        let roots = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(
            roots.len(),
            1,
            "{} must retain dependent track identity",
            c["video"]
        );
        let selected = roots[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        let index = c["sample"].as_u64().unwrap() as usize;
        assert!(group.read_frame(&mut reader, index - 1).is_ok());
        let error = group
            .read_frame(&mut reader, index)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(c["error"].as_str().unwrap()), "{error}");
        assert!(group.read_frame(&mut reader, index - 1).is_ok());
    }
}

#[cfg(feature = "player")]
fn play(
    source: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    fn present(
        source: &mut dyn fvid::audio::AudioStream,
        frame: fvid::audio::DecodedAudio,
        out: &mut Vec<u8>,
    ) {
        if let Some(p) = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            assert_eq!(
                u128::from(p.pts) * u128::from(p.timebase_num) * u128::from(source.sample_rate())
                    % u128::from(p.timebase_den),
                0,
                "seek trimming left the exact sample grid"
            );
            out.extend(p.data);
        }
    }
    let mut out = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => {
                while let Some(f) = decoder.finish_packet().unwrap() {
                    present(source, f, &mut out);
                }
                source.advance_configuration().unwrap();
                *decoder = source.make_decoder().unwrap();
            }
            AudioStep::Pcm(p) => out.extend(p.data),
            AudioStep::Encoded(p) => {
                let stamp = source.decoder_stamp(&p);
                if let Some(f) = source
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    present(source, f, &mut out);
                }
            }
        }
    }
    if source.drain_decoded_at_eof() {
        while let Some(f) = decoder.finish_packet().unwrap() {
            present(source, f, &mut out);
        }
    }
    source.validate_eof().unwrap();
    out
}
#[cfg(feature = "player")]
#[test]
fn transport_switch_playback_seeks_and_rewind_preserve_complete_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-transitions")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let expected = decode(&bytes);
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(bytes), Default::default())
                .unwrap();
        assert_eq!(reader.audio_tracks().len(), 1);
        let mut decoder = reader.make_decoder().unwrap();
        let full = play(&mut reader, &mut decoder);
        assert_eq!(full, expected, "{} playback", c["name"]);
        let stride = usize::from(reader.channels()) * 4;
        let frames = full.len() / stride;
        let a = c["edges"][0].as_u64().unwrap() as usize;
        let b = c["edges"][1].as_u64().unwrap() as usize;
        for sample in [
            1,
            a - 1,
            a,
            a + 1,
            b - 1,
            b,
            b + 1,
            frames,
            frames + 1,
            7,
            7,
            0,
        ] {
            let ticks = (sample as u128 * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            reader.reset_decoder(&mut decoder).unwrap();
            let actual = play(&mut reader, &mut decoder);
            let begin = (u128::try_from(landed).unwrap() * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * stride;
            assert_eq!(
                actual,
                full[begin.min(full.len())..],
                "{} seek {sample}",
                c["name"]
            );
        }
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        assert_eq!(play(&mut reader, &mut decoder), full);
    }
}

#[test]
fn transformed_exports_switch_transport_before_normalization_and_resampling() {
    let dir = std::env::temp_dir().join(format!(
        "fvid-ep1-transport-transform-{}-{}",
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
    for (index, c) in manifest()["cases"].as_array().unwrap().iter().enumerate() {
        let source = root()
            .join("mp4-ep1-transitions")
            .join(c["video"].as_str().unwrap());
        let reference = root()
            .join(if c["local_control"] == true {
                "mp4-ep1-transitions"
            } else {
                "mp4-ep1-epochs"
            })
            .join(c["control"].as_str().unwrap());
        for rate in [48000, 32000] {
            let mut payloads = Vec::new();
            for (j, path) in [&source, &reference].iter().enumerate() {
                let output = dir.join(format!("{index}-{rate}-{j}.wav"));
                fvid_media::decode_audio_transformed(
                    path,
                    &output,
                    fvid_media::AudioDecodeTransform {
                        sample_rate: Some(rate),
                        channels: Some(2),
                        ..Default::default()
                    },
                    &fvid_control::CopyOptions {
                        max_controlled_bytes: Some(128 << 20),
                        ..Default::default()
                    },
                )
                .unwrap_or_else(|e| panic!("{} normalize {rate}: {e}", c["name"]));
                let wave = std::fs::read(&output).unwrap();
                let info =
                    fvid_media::owned_wave_inspect::inspect(&mut Cursor::new(&wave), None).unwrap();
                assert_eq!((info.sample_rate, info.channels), (rate as u32, 2));
                payloads.push(
                    wave[info.data_offset as usize
                        ..info.data_offset as usize + info.data_bytes as usize]
                        .to_vec(),
                );
            }
            assert!(!payloads[0].is_empty());
            assert_eq!(payloads[0], payloads[1], "{} normalized {rate}", c["name"]);
        }
    }
}
