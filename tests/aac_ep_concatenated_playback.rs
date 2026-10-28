use fvid_media::owned_aac::{NativeAacDecoder, config::AudioSpecificConfig};
use serde_json::Value;
use std::{io::Cursor, path::PathBuf, time::Duration};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-ep-concatenated")
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
fn packet<'a>(blob: &'a [u8], row: &Value) -> &'a [u8] {
    let a = row["offset"].as_u64().unwrap() as usize;
    &blob[a..a + row["bytes"].as_u64().unwrap() as usize]
}
fn pcm_bytes(pcm: Vec<f32>) -> Vec<u8> {
    pcm.into_iter().flat_map(f32::to_le_bytes).collect()
}
fn near(c: &Value, actual: &[u8]) {
    let gold = read(c["golden"].as_str().unwrap());
    assert_eq!(actual.len(), gold.len(), "{}", c["name"]);
    let worst = actual
        .chunks_exact(4)
        .zip(gold.chunks_exact(4))
        .map(|(a, b)| {
            (f32::from_le_bytes(a.try_into().unwrap()) - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
        })
        .fold(0f32, f32::max);
    assert!(worst < 1e-7, "{}: PCM error {worst}", c["name"]);
}
#[test]
fn source_aus_are_restored_in_frame_order_before_native_synthesis() {
    let m = manifest();
    let blob = read("packets.bin");
    let plain = read("plain-packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        let ep = config.protection.unwrap();
        assert_eq!(u64::from(ep.frames), c["source_frames"].as_u64().unwrap());
        let mut d = fvid::codec::aac_native::NativeAacDecoder::new(&asc)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        for _ in 0..2 {
            let mut pcm = vec![];
            for row in c["frames"].as_array().unwrap() {
                let raw = packet(&blob, row);
                let units = ep
                    .decode_raw_access_units(raw)
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
                assert_eq!(units.len(), ep.frames as usize);
                for (unit, expected) in units.iter().zip(row["units"].as_array().unwrap()) {
                    assert_eq!(unit, packet(&plain, expected), "{} raw AU", c["name"]);
                }
                assert!(
                    ep.decode_raw_access_unit(raw)
                        .unwrap_err()
                        .to_string()
                        .contains("use decode_raw_access_units")
                );
                pcm.extend(
                    d.decode(raw)
                        .unwrap_or_else(|e| panic!("{}: {e}", c["name"])),
                );
            }
            near(c, &pcm_bytes(pcm));
            d.reset();
        }
    }
}
#[test]
fn fixed_fill_sources_and_packet_clocks_preserve_original_geometry() {
    let m = manifest();
    for c in m["padding"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let mut pcm = vec![];
        for raw in c["raw"].as_array().unwrap() {
            pcm.extend(d.decode(&hex(raw.as_str().unwrap())).unwrap());
        }
        near(c, &pcm_bytes(pcm));
    }
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let original = AudioSpecificConfig::parse(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        assert_eq!(parsed.core, original.core);
        let esds = fvid_media::owned_aac::adts::esds_for(&asc).unwrap();
        let n = fvid_media::owned_mp4_audio_packet_frames::frames(
            b"mp4a",
            &esds,
            c["rate"].as_u64().unwrap() as u32,
            c["channels"].as_u64().unwrap() as u16,
            0,
            1,
            None,
        )
        .unwrap();
        assert_eq!(
            n,
            c["container_frame_samples"].as_u64().unwrap(),
            "{} packet clock",
            c["name"]
        );
    }
}
#[test]
fn complete_pcm_exports_and_ranges_agree_in_both_containers() {
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let mp4 = read(c["video"].as_str().unwrap());
        let mkv = read(c["matroska"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        near(c, &full);
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(owned, full);
        let mut out = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, None).unwrap();
        assert_eq!(out, full, "{} Matroska", c["name"]);
        out.clear();
        fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&mkv),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, full);
        let rate = c["rate"].as_u64().unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (a, b) in [(7, 35), (2, 19), (7, 35)] {
            let range = Some((Duration::from_millis(a), Duration::from_millis(b)));
            let start = ((a * rate).div_ceil(1000) as usize * stride).min(full.len());
            let end = ((b * rate).div_ceil(1000) as usize * stride).min(full.len());
            out.clear();
            fvid::native_media::decode_mp4_aac_pcm_interval(&mp4, &mut out, range).unwrap();
            assert_eq!(out, full[start..end], "{} MP4 interval", c["name"]);
            out.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, range).unwrap();
            assert_eq!(out, full[start..end], "{} Matroska interval", c["name"]);
        }
    }
}
#[cfg(feature = "player")]
fn play(reader: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut out = vec![];
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            out.extend(pcm.data);
        }
    }
    reader.validate_eof().unwrap();
    out
}
#[cfg(feature = "player")]
#[test]
fn playback_rewind_and_seek_cross_internal_and_packet_frame_edges() {
    use fvid::audio::AudioStream;
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let mp4 = read(c["video"].as_str().unwrap());
        let mkv = read(c["matroska"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
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
            assert_eq!(
                play(reader.as_mut()),
                full,
                "{} container {container}",
                c["name"]
            );
            reader.rewind();
            assert_eq!(play(reader.as_mut()), full);
            let edge = c["frame_samples"].as_i64().unwrap();
            let packet_edge = c["container_frame_samples"].as_i64().unwrap();
            for target in [
                0,
                13,
                edge - 1,
                edge,
                edge + 1,
                packet_edge - 1,
                packet_edge,
                packet_edge + 1,
                (full.len() / stride) as i64,
                13,
                13,
                0,
            ] {
                let ticks = (i128::from(target) * i128::from(reader.timescale())
                    / i128::from(reader.sample_rate())) as i64;
                let landed = reader.seek_to(ticks);
                assert!(landed >= 0);
                let first = ((landed as u128 * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale()))
                    as usize
                    * stride)
                    .min(full.len());
                assert_eq!(
                    play(reader.as_mut()),
                    full[first..],
                    "{} container {container} seek {target}, landed {landed}",
                    c["name"]
                );
            }
        }
    }
}
#[test]
fn complete_packet_checkpoint_and_timestamp_overflow_are_transactional() {
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        for row in c["frames"].as_array().unwrap() {
            let raw = packet(&blob, row);
            let saved = d.checkpoint();
            assert!(
                d.retained_payload_bytes_with_checkpoint(Some(&saved))
                    .unwrap()
                    > d.retained_payload_bytes().unwrap()
            );
            let error = d
                .decode_timed_in_clock(raw, i64::MAX, u64::MAX, 1000)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("AAC concatenated frame timestamp overflow"),
                "{}: {error}",
                c["name"]
            );
            let timed = d
                .decode_timed_in_clock(raw, -123, 53, 1000)
                .unwrap()
                .unwrap();
            assert_eq!((timed.pts, timed.duration), (-123, 53));
            d.restore(&saved).unwrap();
            assert_eq!(
                timed.samples,
                d.decode(raw).unwrap(),
                "{} complete packet rollback",
                c["name"]
            );
        }
    }
}
#[test]
fn malformed_protected_movies_have_specific_failures_and_preserve_history() {
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["malformed"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["name"] == c["good"])
            .unwrap();
        assert_eq!(c["asc"], good["asc"]);
        let mut d = NativeAacDecoder::new(&hex(c["asc"].as_str().unwrap())).unwrap();
        let index = c["bad_index"].as_u64().unwrap() as usize;
        for row in &c["frames"].as_array().unwrap()[..index] {
            d.decode(packet(&blob, row)).unwrap();
        }
        let saved = d.checkpoint();
        if c["error"].as_str().unwrap().contains("AAC ELD alignment") {
            let ep = AudioSpecificConfig::parse(&hex(c["asc"].as_str().unwrap()))
                .unwrap()
                .protection
                .unwrap();
            let units = ep
                .decode_raw_access_units(packet(&blob, &c["frames"][index]))
                .unwrap();
            let plain = read("plain-packets.bin");
            for (unit, row) in units
                .iter()
                .zip(c["frames"][index]["units"].as_array().unwrap())
            {
                assert_eq!(
                    unit,
                    packet(&plain, row),
                    "AAC refusal must follow successful EP recovery"
                );
            }
        }
        let error = d
            .decode(packet(&blob, &c["frames"][index]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        let actual = d.decode(packet(&blob, &good["frames"][index])).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(
            actual,
            d.decode(packet(&blob, &good["frames"][index])).unwrap()
        );
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut vec![],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        let error = fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["matroska"].as_str().unwrap()),
            &mut vec![],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
    }
}

#[test]
fn concatenation_escape_restrictions_refuse_saved_invalid_configurations() {
    let m = manifest();
    for c in m["invalid_configs"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let error = NativeAacDecoder::new(&asc).err().unwrap().to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        let error = fvid::native_media::decode_mp4_aac_pcm(
            &read(c["video"].as_str().unwrap()),
            &mut vec![],
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
        let error = fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(c["matroska"].as_str().unwrap()),
            &mut vec![],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains(c["error"].as_str().unwrap()),
            "{}: {error}",
            c["name"]
        );
    }
}

#[test]
fn mp4_to_matroska_remux_preserves_multi_source_packet_clock_and_pcm() {
    let m = manifest();
    let count = m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("-srcpc-i0-mixed"))
        .count();
    assert_eq!(
        count, 84,
        "all fourteen source profiles and six frame counts must be remuxed"
    );
    for c in m["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["name"].as_str().unwrap().ends_with("-srcpc-i0-mixed"))
    {
        let mp4 = read(c["video"].as_str().unwrap());
        let mut input =
            fvid_media::owned_mp4::Mp4Reader::open(Cursor::new(mp4), Default::default()).unwrap();
        let mut output = Cursor::new(Vec::new());
        fvid_media::owned_mp4_matroska::write(&mut input, &mut output, None, None)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        let data = output.into_inner();
        let mut pcm = vec![];
        fvid::native_media::decode_matroska_aac_pcm_interval(&data, &mut pcm, None)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        near(c, &pcm);
        #[cfg(feature = "player")]
        {
            use fvid::audio::AudioStream;
            let mut reader = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(data),
                Default::default(),
            )
            .unwrap();
            assert_eq!(play(&mut reader), pcm);
            reader.rewind();
            let target = c["frame_samples"].as_u64().unwrap() + 1;
            let ticks = (u128::from(target) * u128::from(reader.timescale())
                / u128::from(reader.sample_rate())) as i64;
            let landed = reader.seek_to(ticks);
            let stride = c["channels"].as_u64().unwrap() as usize * 4;
            let first = ((landed as u128 * u128::from(reader.sample_rate()))
                .div_ceil(u128::from(reader.timescale())) as usize
                * stride)
                .min(pcm.len());
            assert_eq!(play(&mut reader), pcm[first..]);
        }
    }
}
