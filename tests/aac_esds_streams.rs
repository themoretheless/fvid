use serde_json::Value;
use std::{io::Cursor, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-esds-streams")
}
fn read(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&read("manifest.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn dependency_url_ocr_priority_and_opaque_sl_are_retained_without_changing_asc() {
    let m = manifest();
    assert_eq!(m["cases"].as_array().unwrap().len(), 11);
    for c in m["cases"].as_array().unwrap() {
        let raw = read(&format!("{}.esds", c["name"].as_str().unwrap()));
        let owned = fvid_media::owned_codec_config::aac_stream_descriptor(&raw).unwrap();
        let public = fvid::codec::config::aac_stream_descriptor(&raw).unwrap();
        assert_eq!(owned.es_id, c["es_id"].as_u64().unwrap() as u16);
        assert_eq!(owned.depends_on, c["depends_on"].as_u64().map(|v| v as u16));
        assert_eq!(owned.ocr_es_id, c["ocr_es_id"].as_u64().map(|v| v as u16));
        assert_eq!(owned.priority, c["priority"].as_u64().unwrap() as u8);
        assert_eq!(owned.url.map(|v| v.to_vec()), c["url"].as_str().map(hex));
        assert_eq!(
            owned.sl_config.map(|v| v.to_vec()),
            c["sl_config"].as_str().map(hex)
        );
        assert_eq!(owned.asc, hex(c["asc"].as_str().unwrap()));
        assert_eq!(
            (
                public.es_id,
                public.depends_on,
                public.ocr_es_id,
                public.priority,
                public.asc,
                public.sl_config,
                public.url
            ),
            (
                owned.es_id,
                owned.depends_on,
                owned.ocr_es_id,
                owned.priority,
                owned.asc,
                owned.sl_config,
                owned.url
            )
        );
        assert_eq!(
            fvid::codec::config::aac_specific_config(&raw).unwrap(),
            owned.asc
        );
        assert_eq!(
            fvid_media::owned_codec_config::aac_specific_config(&raw).unwrap(),
            owned.asc
        );
        for end in 0..raw.len() {
            assert!(
                fvid_media::owned_codec_config::aac_stream_descriptor(&raw[..end]).is_err(),
                "{} prefix {end}",
                c["name"]
            );
        }
    }
}
#[test]
fn mp4_audio_keeps_extreme_es_ids_and_priority_while_preserving_pcm() {
    let m = manifest();
    let golden = read(m["golden"].as_str().unwrap());
    for c in m["movies"].as_array().unwrap() {
        let data = read(c["video"].as_str().unwrap());
        let reader =
            fvid::container::mp4::Mp4Reader::open(Cursor::new(&data), Default::default()).unwrap();
        let track = reader
            .tracks()
            .iter()
            .find(|t| t.codec == *b"mp4a")
            .unwrap();
        let descriptor = fvid::codec::config::aac_stream_descriptor(&track.configuration).unwrap();
        assert_eq!(descriptor.es_id, c["es_id"].as_u64().unwrap() as u16);
        assert_eq!(descriptor.priority, c["priority"].as_u64().unwrap() as u8);
        assert_eq!(descriptor.sl_config, Some(&[2][..]));
        assert!(descriptor.depends_on.is_none());
        let mut output = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut output).unwrap();
        assert_eq!(output.len(), golden.len());
        let error = output
            .chunks_exact(4)
            .zip(golden.chunks_exact(4))
            .map(|(a, b)| {
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
            })
            .fold(0f32, f32::max);
        assert!(error < 1e-7, "{} PCM {error}", c["name"]);
    }
}
#[test]
fn duplicate_sl_descriptor_is_a_specific_error_in_both_parsers_and_movie_export() {
    let raw = read("duplicate-sl.esds");
    for error in [
        fvid_media::owned_codec_config::aac_stream_descriptor(&raw)
            .unwrap_err()
            .to_string(),
        fvid::codec::config::aac_specific_config(&raw)
            .unwrap_err()
            .to_string(),
    ] {
        assert!(error.contains("duplicate SL configuration"), "{error}");
    }
    let mut out = Vec::new();
    let error = fvid::native_media::decode_mp4_aac_pcm(&read("duplicate-sl.mp4"), &mut out)
        .unwrap_err()
        .to_string();
    assert!(error.contains("duplicate SL configuration"), "{error}");
}

#[cfg(feature = "player")]
#[test]
fn stream_descriptor_movies_preserve_player_pcm_and_specific_duplicate_failure() {
    use fvid::audio::AudioStream;
    let play = |data: Vec<u8>| -> Result<Vec<u8>, String> {
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(data), Default::default())
                .map_err(|e| e.to_string())?;
        let mut decoder = reader.make_decoder().map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        while let Some(packet) = reader.next_packet().map_err(|e| e.to_string())? {
            if let Some(frame) = decoder
                .decode_packet(&packet.data, packet.pts, packet.duration as u64)
                .map_err(|e| e.to_string())?
            {
                if let Some(pcm) = reader
                    .present_decoded(frame.packet, frame.source_pts)
                    .map_err(|e| e.to_string())?
                {
                    out.extend(pcm.data);
                }
            }
        }
        while let Some(frame) = decoder.finish_packet().map_err(|e| e.to_string())? {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .map_err(|e| e.to_string())?
            {
                out.extend(pcm.data);
            }
        }
        reader.validate_eof().map_err(|e| e.to_string())?;
        Ok(out)
    };
    let m = manifest();
    for c in m["movies"].as_array().unwrap() {
        let data = read(c["video"].as_str().unwrap());
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut expected).unwrap();
        assert_eq!(play(data).unwrap(), expected);
    }
    let error = play(read("duplicate-sl.mp4")).unwrap_err();
    assert!(error.contains("duplicate SL configuration"), "{error}");
}
