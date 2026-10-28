use std::{io::Cursor, path::PathBuf};
#[path = "support/mp4_audio_playback.rs"]
mod playback;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/mp4-ep1-od-edits")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn od_edits_export_independent_pcm_silence_repeated_windows_and_exact_ranges() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 76);
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
                crop == pcm[a as usize * 48 * stride..(b as usize * 48 * stride).min(pcm.len())],
                "{} crop",
                c["name"]
            );
        }
    }
}
#[cfg(feature = "player")]
#[test]
fn od_edits_playback_seek_across_silence_crops_repetitions_eof_and_rewind() {
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
        let n = c["n"].as_u64().unwrap() as usize;
        let end = full.len() / stride;
        for sample in [
            0,
            n - 1,
            n,
            n + 1,
            8 * n - 1,
            8 * n,
            8 * n + 1,
            13 * n - 1,
            13 * n,
            13 * n + 1,
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
fn inconsistent_od_edits_refuse_without_guessing_a_source_occurrence() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    let m = manifest();
    assert_eq!(m["malformed"].as_array().unwrap().len(), 16);
    for c in m["malformed"].as_array().unwrap() {
        let data = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(data), Default::default()).unwrap();
        let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let group = Group::new(reader.tracks(), at).unwrap();
        for _ in 0..2 {
            let e = group.read_frame(&mut reader, 0).err().unwrap().to_string();
            assert!(
                e.contains(c["error"].as_str().unwrap()),
                "{}: {e}",
                c["video"]
            );
        }
    }
}

#[test]
fn od_edit_mapping_preserves_authored_fractional_boundaries_and_source_aus() {
    use fvid_media::{owned_mp4::Mp4Reader, owned_mp4_ep1::Group};
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["video"].as_str().unwrap())).unwrap();
        let mut reader = Mp4Reader::open_indexed(Cursor::new(bytes), Default::default()).unwrap();
        let at = Group::automatic_audio_tracks(&reader).unwrap()[0];
        let root_track = &reader.tracks()[at];
        assert_eq!(root_track.timescale, 24000);
        assert_eq!(
            root_track.edits.len(),
            if c["repeat"] == true { 3 } else { 0 }
        );
        let od = reader
            .tracks()
            .iter()
            .find(|t| t.handler == *b"odsm")
            .unwrap();
        assert_eq!(od.timescale, 72000);
        assert_eq!(od.edits.len(), if c["repeat"] == true { 4 } else { 1 });
        let n = c["n"].as_i64().unwrap();
        for i in 0..3 {
            assert_eq!(od.samples.get(i).unwrap().pts, 1 + i as i64 * 12 * n);
        }
        let width = if c["channels"] == 1 { 4 } else { 27 };
        let chain: Vec<_> = (2..width + 2).collect();
        for (ticks, expected) in [
            (8 * n - 1, 1),
            (8 * n, width as usize),
            (16 * n - 1, width as usize),
            (16 * n, 1),
        ] {
            assert_eq!(
                reader
                    .object_descriptor_prefix(&chain, ticks, 48000)
                    .unwrap(),
                Some(expected)
            );
        }
        // Each native source frame is still addressed in its own clock;
        // presentation silence/repeats do not reset or rewrite source AU bits.
        let group = Group::new(reader.tracks(), at).unwrap();
        let original = std::fs::read(root().join(format!(
            "../mp4-ep1-od/n{}-c{}-snapshots-indexed.mp4",
            n,
            if c["channels"] == 1 { 1 } else { 7 }
        )))
        .unwrap();
        let mut original =
            Mp4Reader::open_indexed(Cursor::new(original), Default::default()).unwrap();
        let selected = Group::automatic_audio_tracks(&original).unwrap()[0];
        let control = Group::new(original.tracks(), selected).unwrap();
        for i in 0..12 {
            let a = group.read_frame(&mut reader, i).unwrap();
            let b = control.read_frame(&mut original, i).unwrap();
            assert_eq!(a.len(), b.len());
            for (a, b) in a.iter().zip(&b) {
                assert_eq!(a.data, b.data);
            }
        }
    }
}
