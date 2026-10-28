use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ep1-empty")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn explicit_empty_esc_mp4_access_units_are_not_missing_packets() {
    for case in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid_media::owned_mp4::Mp4Reader::open_indexed(Cursor::new(&bytes), Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let mut empties = 0;
        for track in 0..reader.tracks().len() {
            for sample in 0..reader.tracks()[track].samples.len() {
                let entry = reader.tracks()[track].samples.get(sample).unwrap();
                if entry.size != 0 {
                    continue;
                }
                empties += 1;
                assert!(entry.duration > 0);
                assert_eq!(reader.packet_bit_len(track, sample).unwrap(), 0);
                let mut raw = vec![0xaa];
                reader.read_packet(track, sample, &mut raw).unwrap();
                assert!(raw.is_empty());
            }
        }
        assert!(empties > 0, "{} has no empty AU", case["name"]);
        let mut actual = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut actual,
            None,
            &Default::default(),
        )
        .unwrap();
        let expected = std::fs::read(root().join(case["golden"].as_str().unwrap())).unwrap();
        assert_eq!(actual.len(), expected.len());
        let error = actual
            .chunks_exact(4)
            .zip(expected.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} PCM {error}", case["name"]);
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&bytes, &mut public).unwrap();
        assert_eq!(actual, public);
    }
}
#[test]
fn empty_esc_is_distinct_from_omission_and_from_impossible_padding() {
    for case in manifest()["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let expected = case["error"].as_str().unwrap();
        if case["kind"] == "group" {
            let mut reader = fvid_media::owned_mp4::Mp4Reader::open_indexed(
                Cursor::new(bytes),
                Default::default(),
            )
            .unwrap();
            let root = reader
                .tracks()
                .iter()
                .position(|t| {
                    t.handler == *b"soun"
                        && fvid_media::owned_codec_config::aac_stream_descriptor(&t.configuration)
                            .unwrap()
                            .depends_on
                            .is_none()
                })
                .unwrap();
            let group = fvid_media::owned_mp4_ep1::Group::new(reader.tracks(), root).unwrap();
            let error = match group.read_frame(&mut reader, 0) {
                Ok(_) => panic!("missing empty AU was invented"),
                Err(e) => e,
            };
            assert!(error.to_string().contains(expected), "{error}");
        } else {
            match fvid_media::owned_mp4::Mp4Reader::open_indexed(
                Cursor::new(&bytes),
                Default::default(),
            ) {
                Ok(_) => panic!("{} accepted", case["video"]),
                Err(e) => assert!(e.to_string().contains(expected), "{}: {e}", case["video"]),
            }
            match fvid::container::mp4::Mp4Reader::open_indexed(
                Cursor::new(&bytes),
                Default::default(),
            ) {
                Ok(_) => panic!("{} accepted by public parser", case["video"]),
                Err(e) => assert!(e.to_string().contains(expected), "{}: {e}", case["video"]),
            }
        }
    }
}
#[cfg(feature = "player")]
fn play(
    reader: &mut dyn fvid::audio::AudioStream,
    decoder: &mut Box<dyn fvid::audio::AudioDecode>,
) -> Vec<u8> {
    use fvid::audio::AudioStep;
    let mut pcm = Vec::new();
    while let Some(step) = reader.next_step().unwrap() {
        match step {
            AudioStep::ResetDecoder => reader.reset_decoder(decoder).unwrap(),
            AudioStep::ReconfigureDecoder => panic!("unexpected changing empty-ESC setup"),
            AudioStep::Pcm(p) => pcm.extend(p.data),
            AudioStep::Encoded(p) => {
                let stamp = reader.decoder_stamp(&p);
                if let Some(frame) = reader
                    .decode_source_packet(decoder.as_mut(), &p, stamp)
                    .unwrap()
                {
                    if let Some(p) = reader
                        .present_decoded(frame.packet, frame.source_pts)
                        .unwrap()
                    {
                        pcm.extend(p.data);
                    }
                }
            }
        }
    }
    if reader.drain_decoded_at_eof() {
        while let Some(frame) = decoder.finish_packet().unwrap() {
            if let Some(p) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                pcm.extend(p.data);
            }
        }
    }
    reader.validate_eof().unwrap();
    pcm
}
#[cfg(feature = "player")]
#[test]
fn empty_esc_playback_seeks_rewind_and_checkpoints_preserve_history() {
    use fvid::audio::AudioStream;
    for case in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(case["video"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&bytes), Default::default())
                .unwrap();
        assert_eq!(reader.audio_tracks().len(), 1);
        let mut decoder = reader.make_decoder().unwrap();
        let full = play(&mut reader, &mut decoder);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&bytes),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(full, expected);
        let stride = usize::from(reader.channels()) * 4;
        let frame = full.len() / stride / 3;
        for sample in [
            1,
            frame - 1,
            frame,
            frame + 1,
            0,
            frame,
            full.len() / stride,
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
                case["name"]
            );
        }
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        let p = reader.next_packet().unwrap().unwrap();
        let stamp = reader.decoder_stamp(&p);
        let saved = decoder.checkpoint().unwrap();
        let first = reader
            .decode_source_packet(decoder.as_mut(), &p, stamp)
            .unwrap()
            .unwrap();
        decoder.restore(&saved).unwrap();
        let replay = reader
            .decode_source_packet(decoder.as_mut(), &p, stamp)
            .unwrap()
            .unwrap();
        assert_eq!(first.packet.data, replay.packet.data);
        assert_eq!(first.source_pts, replay.source_pts);
        reader.rewind();
        reader.reset_decoder(&mut decoder).unwrap();
        assert_eq!(play(&mut reader, &mut decoder), full);
    }
}
