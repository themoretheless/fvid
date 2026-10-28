use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("mp4-ep1-epochs/manifest.json")).unwrap())
        .unwrap()
}
fn decode(bytes: &[u8]) -> Vec<u8> {
    let mut pcm = Vec::new();
    fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
        Cursor::new(bytes),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    pcm
}
#[test]
fn ep1_description_epochs_export_complete_ps_and_independent_ld_sbr_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let controls: Vec<_> = c["controls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| {
                let directory = if c["local_controls"] == true {
                    "mp4-ep1-epochs"
                } else {
                    "mp4-ep1-ps"
                };
                let path = root().join(directory).join(n.as_str().unwrap());
                let pcm = decode(&std::fs::read(&path).unwrap());
                if c["local_controls"] == true {
                    let gold = std::fs::read(path.with_extension("mp4.f32le")).unwrap();
                    assert_eq!(pcm.len(), gold.len());
                    for (a, b) in pcm.chunks_exact(4).zip(gold.chunks_exact(4)) {
                        assert!(
                            (f32::from_le_bytes(a.try_into().unwrap())
                                - f32::from_le_bytes(b.try_into().unwrap()))
                            .abs()
                                < 1e-7
                        );
                    }
                }
                pcm
            })
            .collect();
        let expected = [
            controls[0].clone(),
            controls[1].clone(),
            controls[0].clone(),
        ]
        .concat();
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        assert_eq!(decode(&bytes), expected, "{} complete epochs", c["name"]);
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut public).unwrap();
        assert_eq!(public, expected);
        let edge_ms = c["edges"][0].as_u64().unwrap() / 48;
        for (from, to) in [
            (edge_ms - 1, edge_ms + 1),
            (7, 31),
            (edge_ms - 1, edge_ms + 1),
        ] {
            let mut window = Vec::new();
            fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&bytes),
                &mut window,
                Some((
                    std::time::Duration::from_millis(from),
                    std::time::Duration::from_millis(to),
                )),
                &Default::default(),
            )
            .unwrap();
            let stride = c["channels"].as_u64().unwrap() as usize * 4;
            assert_eq!(
                window,
                expected[from as usize * 48 * stride..to as usize * 48 * stride],
                "{} interval {from}..{to}",
                c["name"]
            );
        }
    }
}

#[test]
fn dependent_description_mismatch_refuses_at_the_exact_epoch_boundary() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
                .join(c["malformed"].as_str().unwrap()),
        )
        .unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        assert!(
            Group::admission_upper_bound(reader.tracks(), selected).unwrap()
                >= group.retained_payload_bytes().unwrap()
        );
        for i in 0..6 {
            assert_eq!(
                group.read_frame(&mut reader, i).unwrap().len(),
                c["streams"].as_u64().unwrap() as usize
            );
        }
        let error = group.read_frame(&mut reader, 6).err().unwrap().to_string();
        assert!(
            error.contains("simultaneous descriptions disagree with root ASC"),
            "{error}"
        );
        assert_eq!(
            group.read_frame(&mut reader, 0).unwrap().len(),
            c["streams"].as_u64().unwrap() as usize
        );
        // Same initial header and track IDs cannot conceal a changed later stsd.
        let good = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let mut good_reader =
            Mp4Reader::open_indexed(Cursor::new(good), Default::default()).unwrap();
        let saved = Group::new(good_reader.tracks(), selected).unwrap();
        let error = saved.read_frame(&mut reader, 6).err().unwrap().to_string();
        assert!(
            error.contains("group belongs to different track configuration"),
            "{error}"
        );
        assert_eq!(
            saved.read_frame(&mut good_reader, 6).unwrap().len(),
            c["streams"].as_u64().unwrap() as usize
        );
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
        pcm: &mut Vec<u8>,
    ) {
        if let Some(p) = source
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            pcm.extend(p.data);
        }
    }
    let mut pcm = Vec::new();
    while let Some(step) = source.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => source.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => {
                while let Some(f) = decoder.finish_packet().unwrap() {
                    present(source, f, &mut pcm);
                }
                source.advance_configuration().unwrap();
                *decoder = source.make_decoder().unwrap();
            }
            AudioStep::Pcm(p) => pcm.extend(p.data),
            AudioStep::Encoded(p) => {
                let stamp = source.decoder_stamp(&p);
                if let Some(f) = source
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    present(source, f, &mut pcm);
                }
            }
        }
    }
    if source.drain_decoded_at_eof() {
        while let Some(f) = decoder.finish_packet().unwrap() {
            present(source, f, &mut pcm);
        }
    }
    source.validate_eof().unwrap();
    pcm
}
#[cfg(feature = "player")]
#[test]
fn changing_ep1_playback_drains_each_epoch_and_preserves_seek_rewind_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
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
        assert_eq!(full, expected);
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
fn fragmented_and_hybrid_epochs_retain_packet_identity_padding_and_descriptions() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 24);
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c.get("mode").is_some())
    {
        let data = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
                .join(c["video"].as_str().unwrap()),
        )
        .unwrap();
        let mut at = 0;
        let mut fragments = 0;
        while at < data.len() {
            let size = u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as usize;
            assert!(size >= 8);
            if &data[at + 4..at + 8] == b"moof" {
                fragments += 1;
            }
            at += size;
        }
        assert_eq!(at, data.len());
        assert_eq!(fragments, if c["mode"] == "hybrid" { 2 } else { 3 });
        let reference = std::fs::read(
            root()
                .join("mp4-ep1-epochs")
                .join(c["indexed"].as_str().unwrap()),
        )
        .unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let mut original =
            Mp4Reader::open_indexed(Cursor::new(reference), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(selected.len(), 1);
        let other = Group::automatic_audio_tracks(&original).unwrap();
        assert_eq!(other.len(), 1);
        let group = Group::for_root(reader.tracks(), selected[0])
            .unwrap()
            .unwrap();
        let baseline = Group::for_root(original.tracks(), other[0])
            .unwrap()
            .unwrap();
        for t in reader.tracks().iter().filter(|t| t.handler == *b"soun") {
            assert_eq!(t.samples.len(), 18);
            for i in 0..18 {
                assert_eq!(
                    t.samples.get(i).unwrap().description_index,
                    [1, 2, 1][i / 6] + u32::from(c["unused_first"] == true)
                );
            }
        }
        for i in 0..18 {
            let actual = group.read_frame(&mut reader, i).unwrap();
            let expected = baseline.read_frame(&mut original, i).unwrap();
            assert_eq!(actual.len(), expected.len());
            for (a, b) in actual.iter().zip(&expected) {
                assert_eq!(
                    (
                        a.stream_id,
                        a.depends_on,
                        a.bit_len,
                        a.pts,
                        a.duration,
                        a.timescale
                    ),
                    (
                        b.stream_id,
                        b.depends_on,
                        b.bit_len,
                        b.pts,
                        b.duration,
                        b.timescale
                    )
                );
                assert_eq!(a.data, b.data);
            }
        }
    }
}
