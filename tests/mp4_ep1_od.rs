use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mp4-ep1-od")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn od_membership_changes_keep_the_same_asc_and_continuous_independent_pcm() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group,
    };
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 72);
    for c in m["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap();
        assert_eq!(selected.len(), 1);
        let group = Group::new(reader.tracks(), selected[0]).unwrap();
        let asc = aac_stream_descriptor(&reader.tracks()[selected[0]].configuration)
            .unwrap()
            .asc
            .to_vec();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut samples = Vec::new();
        for i in 0..12 {
            assert_eq!(
                aac_stream_descriptor(
                    &reader.tracks()[selected[0]]
                        .description_for_sample(i)
                        .unwrap()
                        .configuration
                )
                .unwrap()
                .asc,
                asc
            );
            let units = group
                .read_frame(&mut reader, i)
                .unwrap_or_else(|e| panic!("{} frame {i}: {e}", c["name"]));
            assert_eq!(units.len(), c["active"][i / 4].as_u64().unwrap() as usize);
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let checkpoint = decoder.checkpoint();
            let frame = decoder.decode_esc(&input).unwrap().unwrap();
            assert_eq!(frame.pts, i as i64 * c["n"].as_i64().unwrap());
            decoder.restore(&checkpoint).unwrap();
            assert_eq!(decoder.decode_esc(&input).unwrap().as_ref(), Some(&frame));
            samples.extend(frame.samples);
        }
        let golden = std::fs::read(root().join(c["golden"].as_str().unwrap())).unwrap();
        assert_eq!(samples.len() * 4, golden.len());
        for (a, b) in samples.iter().zip(golden.chunks_exact(4)) {
            assert!(
                (*a - f32::from_le_bytes(b.try_into().unwrap())).abs() < 1e-7,
                "{} independent PCM",
                c["name"]
            );
        }
    }
}

#[test]
fn owned_and_public_od_membership_exports_preserve_exact_pcm_and_ranges() {
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let control = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(control),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        let mut public = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut public).unwrap();
        assert!(public == expected, "{} public PCM", c["name"]);
        for range in [None, Some((7u64, 31u64)), Some((35, 67)), Some((7, 31))] {
            let mut actual = Vec::new();
            let stats = fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
                Cursor::new(&data),
                &mut actual,
                range.map(|(a, b)| {
                    (
                        std::time::Duration::from_millis(a),
                        std::time::Duration::from_millis(b),
                    )
                }),
                &fvid_control::CopyOptions {
                    max_controlled_bytes: Some(256 << 20),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate, stats.channels),
                (rate as u32, c["channels"].as_u64().unwrap() as u16)
            );
            let wanted = if let Some((a, b)) = range {
                &expected[(a * rate / 1000) as usize * stride
                    ..((b * rate / 1000) as usize * stride).min(expected.len())]
            } else {
                &expected
            };
            assert!(actual == wanted, "{} crop {range:?}", c["name"]);
        }
    }
}
#[path = "support/mp4_audio_playback.rs"]
mod playback;
#[cfg(feature = "player")]
use playback::play;
#[cfg(feature = "player")]
#[test]
fn od_membership_playback_frame_edge_seek_eof_and_rewind_match_pcm() {
    use fvid::audio::AudioStream;
    for c in manifest()["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut source =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(source.audio_tracks().len(), 1);
        assert_eq!(source.sample_rate(), c["rate"].as_u64().unwrap() as u32);
        assert_eq!(source.channels(), c["channels"].as_u64().unwrap() as u16);
        let mut decoder = source.make_decoder().unwrap();
        let full = play(&mut source, &mut decoder);
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&data),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        assert!(full == expected);
        let stride = usize::from(source.channels()) * 4;
        let frame = c["n"].as_u64().unwrap() as usize * source.sample_rate() as usize / 24000;
        let end = full.len() / stride;
        for sample in [
            1,
            4 * frame - 1,
            4 * frame,
            4 * frame + 1,
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
            let actual = play(&mut source, &mut decoder);
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
        assert!(play(&mut source, &mut decoder) == full);
    }
}

#[test]
fn od_membership_wave_gain_crop_and_resampling_match_separate_transport() {
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
                    interval: Some((7000, 31000)),
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

fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn explicit_od_errors_are_specific_atomic_and_do_not_relax_missing_aus() {
    use fvid_media::{
        owned_aac::NativeAacDecoder, owned_codec_config::aac_stream_descriptor,
        owned_mp4::Mp4Reader, owned_mp4_ep1::Group, owned_mp4_od::State,
    };
    let m = manifest();
    assert_eq!(m["malformed"].as_array().unwrap().len(), 130);
    for bad in m["malformed"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(bad["video"].as_str().unwrap())).unwrap();
        let parsed = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default());
        let expected = bad["error"].as_str().unwrap();
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["video"] == bad["good"])
            .unwrap();
        if bad["parser"] == true {
            let error = parsed
                .err()
                .expect("malformed OD must refuse indexing")
                .to_string();
            assert!(error.contains(expected), "{}: {error}", bad["video"]);
            if bad["command"] == true {
                let refs: Vec<_> = bad["od_refs"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u32)
                    .collect();
                let mut state = State::default();
                state
                    .apply(&hex(good["od_packets"][0].as_str().unwrap()), &refs)
                    .unwrap();
                let before = state.clone();
                assert!(state
                    .apply(&hex(bad["packet"].as_str().unwrap()), &refs)
                    .unwrap_err()
                    .contains(expected));
                assert_eq!(state, before);
                state
                    .apply(&hex(good["od_packets"][1].as_str().unwrap()), &refs)
                    .unwrap();
                let chain: Vec<_> = (2..bad["width"].as_u64().unwrap() as u32 + 2).collect();
                assert_eq!(state.prefix(&chain).unwrap(), chain.len());
                state
                    .apply(&hex(good["od_packets"][2].as_str().unwrap()), &refs)
                    .unwrap();
                assert_eq!(state.prefix(&chain).unwrap(), 1);
            }
        } else {
            let mut reader = parsed.unwrap();
            let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
            let group = Group::new(reader.tracks(), selected).unwrap();
            let asc = aac_stream_descriptor(&reader.tracks()[selected].configuration)
                .unwrap()
                .asc
                .to_vec();
            let mut actual = NativeAacDecoder::new(&asc).unwrap();
            let data = std::fs::read(root().join(good["video"].as_str().unwrap())).unwrap();
            let mut reference =
                Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
            let at = Group::automatic_audio_tracks(&reference).unwrap()[0];
            let control = Group::new(reference.tracks(), at).unwrap();
            let mut baseline = NativeAacDecoder::new(&asc).unwrap();
            let stop = bad["at"].as_u64().unwrap() as usize;
            for i in 0..stop {
                let units = group.read_frame(&mut reader, i).unwrap();
                let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                let units = control.read_frame(&mut reference, i).unwrap();
                let expected: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                assert_eq!(
                    actual.decode_esc(&input).unwrap(),
                    baseline.decode_esc(&expected).unwrap()
                );
            }
            let checkpoint = actual.checkpoint();
            let error = group
                .read_frame(&mut reader, stop)
                .err()
                .expect("invalid OD selection must refuse")
                .to_string();
            assert!(error.contains(expected), "{}: {error}", bad["video"]);
            if stop > 0 {
                assert_eq!(group.read_frame(&mut reader, 0).unwrap().len(), 1);
            }
            for i in stop..12 {
                let units = control.read_frame(&mut reference, i).unwrap();
                let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
                let expected = baseline.decode_esc(&input).unwrap();
                assert_eq!(actual.decode_esc(&input).unwrap(), expected);
                if i == stop {
                    actual.restore(&checkpoint).unwrap();
                    assert_eq!(actual.decode_esc(&input).unwrap(), expected);
                }
            }
        }
    }
}

#[test]
fn od_storage_bits_clock_mapping_and_retained_metadata_are_authored() {
    use fvid_media::{
        owned_aac::aac_ep1,
        owned_mp4::Mp4Reader,
        owned_mp4_ep1::Group,
        owned_mp4_od::{Event, Object, Timeline},
    };
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(&data), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), selected).unwrap();
        let width = c["active"][1].as_u64().unwrap() as usize;
        assert_eq!(group.stream_count(), width);
        assert!(
            Group::admission_upper_bound(reader.tracks(), selected).unwrap()
                >= group.retained_payload_bytes().unwrap()
        );
        let od = reader
            .tracks()
            .iter()
            .position(|t| t.handler == *b"odsm")
            .unwrap();
        assert_eq!(reader.tracks()[od].codec, *b"mp4s");
        assert_eq!(reader.tracks()[od].timescale, 72000);
        assert_eq!(reader.tracks()[od].samples.len(), 3);
        assert_eq!(
            reader.tracks()[od].od_references,
            c["od_refs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect::<Vec<_>>()
        );
        let mut moov_end = 0;
        let (mut cursor, mut moofs) = (0, 0);
        while cursor < data.len() {
            let size = u32::from_be_bytes(data[cursor..cursor + 4].try_into().unwrap()) as usize;
            if &data[cursor + 4..cursor + 8] == b"moov" {
                moov_end = cursor + size;
            }
            if &data[cursor + 4..cursor + 8] == b"moof" {
                moofs += 1;
            }
            cursor += size;
        }
        assert_eq!(
            moofs,
            if c["mode"] == "indexed" {
                0
            } else if c["mode"] == "hybrid" {
                2
            } else {
                3
            }
        );
        for i in 0..3 {
            let stamp = reader.tracks()[od].samples.get(i).unwrap();
            assert_eq!(stamp.dts, (i as u64) * 12 * c["n"].as_u64().unwrap());
            assert_eq!(stamp.pts, stamp.dts as i64);
            let mut packet = Vec::new();
            reader.read_packet(od, i, &mut packet).unwrap();
            assert_eq!(packet, hex(c["od_packets"][i].as_str().unwrap()));
            assert_eq!(
                stamp.offset < (moov_end as u64),
                c["mode"] == "indexed" || c["mode"] == "hybrid" && i == 0
            );
        }
        let bytes = std::fs::read(root().join(c["control"].as_str().unwrap())).unwrap();
        let mut original = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let at = Group::automatic_audio_tracks(&original).unwrap()[0];
        let control = Group::new(original.tracks(), at).unwrap();
        let chain: Vec<_> = (2..width as u32 + 2).collect();
        let n = c["n"].as_i64().unwrap();
        for (time, expected) in [
            (8 * n - 1, 1),
            (8 * n, width),
            (16 * n - 1, width),
            (16 * n, 1),
        ] {
            assert_eq!(
                reader
                    .object_descriptor_prefix(&chain, time, 48000)
                    .unwrap(),
                Some(expected)
            );
        }
        for i in 0..12 {
            let units = control.read_frame(&mut original, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let expected = aac_ep1::associate(&input).unwrap();
            let units = group.read_frame(&mut reader, i).unwrap();
            let input: Vec<_> = units.iter().map(|u| u.borrowed()).collect();
            let actual = aac_ep1::associate(&input).unwrap();
            assert_eq!(
                (actual.pts, actual.duration, actual.timescale),
                (expected.pts, expected.duration, expected.timescale)
            );
            if i / 4 == 1 {
                assert_eq!(actual.payload.classes, expected.payload.classes);
            } else {
                assert_eq!(actual.payload.classes.len(), 1);
                assert_eq!(
                    actual.payload.classes[0].bits,
                    expected
                        .payload
                        .classes
                        .iter()
                        .flat_map(|c| c.bits.iter().copied())
                        .collect::<Vec<_>>()
                );
            }
            let sample = reader.tracks()[selected].samples.get(i).unwrap();
            assert_eq!(
                sample.offset < (moov_end as u64),
                c["mode"] == "indexed" || c["mode"] == "hybrid" && i < 4
            );
        }
        if c["mode"] == "indexed" && c["unused"] == false {
            let omitted = m["malformed"]
                .as_array()
                .unwrap()
                .iter()
                .find(|b| b["good"] == c["video"] && b["variant"] == "no-iods")
                .unwrap();
            let no_od = Mp4Reader::open_indexed(
                Cursor::new(
                    std::fs::read(root().join(omitted["video"].as_str().unwrap())).unwrap(),
                ),
                Default::default(),
            )
            .unwrap();
            let minimum = std::mem::size_of::<Timeline>()
                + 3 * std::mem::size_of::<Event>()
                + 3 * std::mem::size_of::<Object>()
                + (2 * width + 3) * 4;
            assert!(
                reader.estimated_index_payload_bytes().unwrap()
                    >= no_od.estimated_index_payload_bytes().unwrap() + minimum
            );
        }
    }
}
