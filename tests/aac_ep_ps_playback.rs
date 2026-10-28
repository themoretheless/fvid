use fvid_media::owned_aac::{
    aac_ps_native::{Frame, InBandPsProbe, NativePsAacDecoder},
    config::AudioSpecificConfig,
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::Cursor,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/aac-ep-ps")
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
fn floats(data: &[u8]) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
        .collect()
}
fn decoder(c: &Value, protected: bool) -> NativePsAacDecoder {
    let asc = hex(c[if protected { "asc" } else { "plain_asc" }]
        .as_str()
        .unwrap());
    if c["signal"] == "implicit" {
        NativePsAacDecoder::new_with_in_band_ps(&asc, c["rate"].as_u64().unwrap() as u32).unwrap()
    } else {
        NativePsAacDecoder::new(&asc).unwrap()
    }
}
fn reference(c: &Value) -> Arc<Vec<f32>> {
    type Key = (u64, u64, u64, usize);
    static CACHE: OnceLock<Mutex<BTreeMap<Key, Arc<Vec<f32>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let rows = c["raw_frames"].as_array().unwrap();
    let key = (
        c["aot"].as_u64().unwrap(),
        c["n"].as_u64().unwrap(),
        c["rate"].as_u64().unwrap(),
        rows.len(),
    );
    if let Some(v) = cache.lock().unwrap().get(&key) {
        return v.clone();
    }
    use fvid_media::owned_aac::{aac_sbr_dsp::OutputRate, aac_sbr_ps::Decoder, bits::BitReader};
    let core = floats(&read(c["core_reference"].as_str().unwrap()));
    let n = c["n"].as_u64().unwrap() as usize;
    let mode = if c["rate"] == 24000 {
        OutputRate::Core
    } else {
        OutputRate::Double
    };
    let mut stage = Decoder::default();
    let mut output = vec![];
    let mut ordinal = 0;
    let mut append = |frame: fvid_media::owned_aac::aac_sbr_ps::Frame| {
        assert_eq!(frame.frame_index, ordinal);
        ordinal += 1;
        output.extend(
            frame.pcm[0]
                .iter()
                .zip(&frame.pcm[1])
                .flat_map(|(l, r)| [*l as f32, *r as f32]),
        );
    };
    for (i, row) in rows.iter().enumerate() {
        let raw = hex(row["payload"].as_str().unwrap());
        let mut bits = BitReader::new(&raw);
        assert_eq!(bits.read(4).unwrap(), 13);
        if let Some(frame) = stage
            .read(
                &mut bits,
                raw.len() * 8,
                false,
                &core[i * n..(i + 1) * n],
                48000,
                c["slots"].as_u64().unwrap() as u8,
                mode,
            )
            .unwrap()
        {
            append(frame);
        }
    }
    while let Some(frame) = stage.finish().unwrap() {
        append(frame);
    }
    assert_eq!(ordinal as usize, rows.len());
    let output = Arc::new(output);
    cache.lock().unwrap().insert(key, output.clone());
    output
}
fn near(c: &Value, actual: &[f32]) {
    let expected = reference(c);
    assert_eq!(actual.len(), expected.len(), "{} sample count", c["name"]);
    assert!(actual.iter().all(|v| v.is_finite()));
    let worst = actual
        .iter()
        .zip(expected.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(worst < 2e-7, "{} PCM error {worst}", c["name"]);
}
fn append(c: &Value, frame: Frame, next: &mut u64, out: &mut Vec<f32>) {
    assert_eq!(frame.frame_index, *next, "{} packet identity", c["name"]);
    assert_eq!(u64::from(frame.sample_rate), c["rate"].as_u64().unwrap());
    assert_eq!(
        frame.pcm.len() as u64,
        c["container_frame_samples"].as_u64().unwrap() * 2
    );
    *next += 1;
    out.extend(frame.pcm);
}
#[test]
fn recovered_aus_and_ps_probe_validate_full_protected_packets() {
    let m = manifest();
    let blob = read("packets.bin");
    let plain = read("plain-packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let parsed = AudioSpecificConfig::parse(&asc).unwrap();
        let original = AudioSpecificConfig::parse(&hex(c["plain_asc"].as_str().unwrap())).unwrap();
        assert_eq!(parsed.core, original.core);
        assert_eq!(parsed.ps_present, original.ps_present);
        let ep = parsed.protection.unwrap();
        assert_eq!(u64::from(ep.frames), c["source_frames"].as_u64().unwrap());
        let mut probe = InBandPsProbe::new(&asc, c["rate"].as_u64().unwrap() as u32).unwrap();
        assert_eq!(probe.requires_in_band_ps(), c["signal"] == "implicit");
        for _ in 0..2 {
            for row in c["frames"].as_array().unwrap() {
                let raw = packet(&blob, row);
                let units = ep.decode_raw_access_units(raw).unwrap();
                assert_eq!(units.len(), ep.frames as usize);
                for (unit, expected) in units.iter().zip(row["units"].as_array().unwrap()) {
                    assert_eq!(unit, packet(&plain, expected));
                }
                assert!(
                    probe
                        .read(raw)
                        .unwrap_or_else(|e| panic!("{}: {e}", c["name"]))
                );
            }
            assert!(probe.ps_detected());
            probe.reset();
            assert!(!probe.ps_detected());
        }
    }
}
#[test]
fn native_stereo_matches_scalar_core_and_component_ps_oracle_with_reset() {
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let mut d = decoder(c, true);
        for _ in 0..2 {
            let mut output = vec![];
            let mut next = 0;
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let saved = d.checkpoint();
                let raw = packet(&blob, row);
                let frame = d
                    .decode(raw)
                    .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
                d.restore(&saved).unwrap();
                assert_eq!(frame, d.decode(raw).unwrap());
                assert_eq!(d.pending_frame_index(), Some(i as u64));
                if let Some(frame) = frame {
                    append(c, frame, &mut next, &mut output);
                }
            }
            while let Some(frame) = d.finish().unwrap() {
                append(c, frame, &mut next, &mut output);
            }
            assert!(d.finish().unwrap().is_none());
            assert_eq!(next as usize, c["frames"].as_array().unwrap().len());
            near(c, &output);
            let mut plain = decoder(c, false);
            assert!(
                plain
                    .restore(&d.checkpoint())
                    .unwrap_err()
                    .to_string()
                    .contains("PS AAC checkpoint configuration mismatch")
            );
            d.reset();
            assert!(!d.ps_detected());
            assert_eq!(d.pending_frame_index(), None);
        }
    }
}
#[test]
fn container_exports_and_intervals_match_complete_stereo_pcm() {
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let mp4 = read(c["video"].as_str().unwrap());
        let mkv = read(c["matroska"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        near(c, &floats(&full));
        let mut out = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(&mp4),
            &mut out,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, full);
        out.clear();
        fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, None)
            .unwrap_or_else(|e| panic!("{}: {e}", c["name"]));
        assert_eq!(out, full);
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
        for (a, b) in [(7, 35), (2, 19), (7, 35)] {
            let interval = Some((Duration::from_millis(a), Duration::from_millis(b)));
            let first = ((a * rate).div_ceil(1000) as usize * 8).min(full.len());
            let last = ((b * rate).div_ceil(1000) as usize * 8).min(full.len());
            out.clear();
            fvid::native_media::decode_mp4_aac_pcm_interval(&mp4, &mut out, interval).unwrap();
            assert_eq!(out, full[first..last]);
            out.clear();
            fvid::native_media::decode_matroska_aac_pcm_interval(&mkv, &mut out, interval).unwrap();
            assert_eq!(out, full[first..last], "{} range", c["name"]);
        }
    }
}
#[cfg(feature = "player")]
fn play(stream: &mut dyn fvid::audio::AudioStream) -> Vec<u8> {
    let mut decoder = stream.make_decoder().unwrap();
    let mut out = vec![];
    loop {
        use fvid::audio::AudioStep;
        let decoded = match stream.next_step().unwrap() {
            Some(AudioStep::Encoded(p)) => decoder
                .decode_packet(&p.data, p.pts, p.duration as u64)
                .unwrap(),
            Some(AudioStep::Pcm(p)) => {
                out.extend(p.data);
                continue;
            }
            Some(AudioStep::ReconfigureDecoder) => panic!("unexpected reconfiguration"),
            Some(AudioStep::ResetDecoder) => {
                decoder.reset();
                continue;
            }
            None => {
                if !stream.drain_decoded_at_eof() {
                    decoder.reset();
                    stream.validate_eof().unwrap();
                    break;
                }
                let frame = decoder.finish_packet().unwrap();
                if frame.is_none() {
                    stream.validate_eof().unwrap();
                    break;
                }
                frame
            }
        };
        if let Some(frame) = decoded {
            if let Some(pcm) = stream
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                out.extend(pcm.data);
            }
        }
    }
    out
}
#[cfg(feature = "player")]
#[test]
fn delayed_stereo_playback_rewind_and_seek_preserve_source_packet_identity() {
    use fvid::audio::AudioStream;
    let m = manifest();
    for c in m["cases"].as_array().unwrap() {
        let mp4 = read(c["video"].as_str().unwrap());
        let mkv = read(c["matroska"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&mp4, &mut full).unwrap();
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
                (full.len() / 8) as i64,
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
                    * 8)
                .min(full.len());
                assert_eq!(
                    play(reader.as_mut()),
                    full[first..],
                    "{} container {container} seek {target} landed {landed}",
                    c["name"]
                );
            }
        }
    }
}
#[test]
fn integrity_and_late_sbr_errors_roll_back_ps_history_partial_pcm_and_presence() {
    let m = manifest();
    let blob = read("packets.bin");
    let plain = read("plain-packets.bin");
    for bad in m["malformed"].as_array().unwrap() {
        let good = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["name"] == bad["good"])
            .unwrap();
        assert_eq!(bad["asc"], good["asc"]);
        let asc = hex(bad["asc"].as_str().unwrap());
        let index = bad["bad_index"].as_u64().unwrap() as usize;
        let raw = packet(&blob, &bad["frames"][index]);
        let mut d = decoder(bad, true);
        let mut probe = InBandPsProbe::new(&asc, bad["rate"].as_u64().unwrap() as u32).unwrap();
        let mut prefix = Vec::new();
        for row in &bad["frames"].as_array().unwrap()[..index] {
            if let Some(frame) = d.decode(packet(&blob, row)).unwrap() {
                prefix.extend(frame.pcm);
            }
            assert!(probe.read(packet(&blob, row)).unwrap());
        }
        let saved = d.checkpoint();
        let seen_before = d.ps_detected();
        let pending_before = d.pending_frame_index();
        let probe_seen_before = probe.ps_detected();
        let error = d.decode(raw).unwrap_err().to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{}: {error}",
            bad["name"]
        );
        let error = probe.read(raw).unwrap_err().to_string();
        assert!(
            error.contains(bad["error"].as_str().unwrap()),
            "{} probe: {error}",
            bad["name"]
        );
        assert_eq!(d.ps_detected(), seen_before);
        assert_eq!(d.pending_frame_index(), pending_before);
        assert_eq!(probe.ps_detected(), probe_seen_before);
        if bad["error"] == "ER AAC SBR CRC is forbidden" {
            let ep = AudioSpecificConfig::parse(&asc)
                .unwrap()
                .protection
                .unwrap();
            let units = ep.decode_raw_access_units(raw).unwrap();
            for (unit, row) in units
                .iter()
                .zip(bad["frames"][index]["units"].as_array().unwrap())
            {
                assert_eq!(unit, packet(&plain, row));
            }
        }
        let raw = packet(&blob, &good["frames"][index]);
        let actual = d.decode(raw).unwrap();
        d.restore(&saved).unwrap();
        assert_eq!(actual, d.decode(raw).unwrap());
        let mut output = prefix;
        output.extend(actual.into_iter().flat_map(|f| f.pcm));
        for row in &good["frames"].as_array().unwrap()[index + 1..] {
            if let Some(f) = d.decode(packet(&blob, row)).unwrap() {
                output.extend(f.pcm);
            }
        }
        while let Some(f) = d.finish().unwrap() {
            output.extend(f.pcm);
        }
        near(good, &output);
        let e = fvid::native_media::decode_mp4_aac_pcm(
            &read(bad["video"].as_str().unwrap()),
            &mut vec![],
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains(bad["error"].as_str().unwrap()),
            "{} MP4: {e}",
            bad["name"]
        );
        let e = fvid::native_media::decode_matroska_aac_pcm_interval(
            &read(bad["matroska"].as_str().unwrap()),
            &mut vec![],
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains(bad["error"].as_str().unwrap()),
            "{} MKV: {e}",
            bad["name"]
        );
        let data = read(bad["matroska"].as_str().unwrap());
        let e = fvid_media::owned_matroska_aac::decode_matroska_aac_pcm(
            Cursor::new(&data),
            &mut vec![],
            None,
            &Default::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            e.contains(bad["error"].as_str().unwrap()),
            "{} owned MKV: {e}",
            bad["name"]
        );
        if bad["signal"] == "implicit" && bad["source_frames"] == 4 {
            let e = fvid::playback_webm_audio::WebmAudioReader::open(
                Cursor::new(data),
                Default::default(),
            )
            .err()
            .unwrap()
            .to_string();
            assert!(
                e.contains(bad["error"].as_str().unwrap()),
                "{} reader must retain the first probe failure: {e}",
                bad["name"]
            );
        }
    }
}
#[test]
fn playback_bridge_keeps_original_signed_timestamps_durations_and_eof() {
    use fvid::codec::aac_ps_playback::PsAacDecoder;
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["cases"].as_array().unwrap() {
        let esds = fvid_media::owned_aac::adts::esds_for(&hex(c["asc"].as_str().unwrap())).unwrap();
        let rate = c["rate"].as_u64().unwrap() as u32;
        let mut d = if c["signal"] == "implicit" {
            PsAacDecoder::new_with_in_band_ps(&esds, rate, 2).unwrap()
        } else {
            PsAacDecoder::new(&esds, rate, 2).unwrap()
        };
        let rows = c["frames"].as_array().unwrap();
        let expected = reference(c);
        let width = c["container_frame_samples"].as_u64().unwrap() as usize * 2;
        let mut next = 0usize;
        let mut check = |frame: fvid::codec::aac_ps_playback::DecodedFrame| {
            let i = frame.frame_index as usize;
            assert_eq!(i, next);
            next += 1;
            assert_eq!(frame.source_pts, -37 + i as i64 * 113);
            assert_eq!(frame.source_duration, 53 + i as u64 % 3);
            let pcm = floats(&frame.packet.data);
            assert_eq!(pcm.len(), width);
            let worst = pcm
                .iter()
                .zip(&expected[i * width..(i + 1) * width])
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            assert!(worst < 2e-7);
        };
        for (i, row) in rows.iter().enumerate() {
            if let Some(frame) = d
                .decode(packet(&blob, row), -37 + i as i64 * 113, 53 + i as u64 % 3)
                .unwrap()
            {
                check(frame);
            }
        }
        while let Some(frame) = d.finish().unwrap() {
            check(frame);
        }
        assert_eq!(next, rows.len());
        assert!(d.finish().unwrap().is_none());
    }
}

#[test]
fn original_and_ancillary_padded_sources_are_bit_exact_without_ep() {
    let m = manifest();
    assert_eq!(m["padded"].as_array().unwrap().len(), 24);
    for c in m["padded"].as_array().unwrap() {
        let mut settings = c.clone();
        settings["plain_asc"] = c["asc"].clone();
        let run = |field: &str| {
            let mut d = decoder(&settings, false);
            let mut pcm = vec![];
            let mut index = 0;
            for raw in c[field].as_array().unwrap() {
                if let Some(frame) = d.decode(&hex(raw.as_str().unwrap())).unwrap() {
                    assert_eq!(frame.frame_index, index);
                    index += 1;
                    pcm.extend(frame.pcm);
                }
            }
            while let Some(frame) = d.finish().unwrap() {
                assert_eq!(frame.frame_index, index);
                index += 1;
                pcm.extend(frame.pcm);
            }
            assert_eq!(index, 14);
            pcm
        };
        let original = run("original");
        let padded = run("padded");
        assert_eq!(
            original, padded,
            "{} valid ancillary fill changed PCM",
            c["name"]
        );
        assert!(padded.iter().any(|v| v.abs() > 1e-7));
    }
}

#[cfg(feature = "player")]
#[test]
fn protected_mono_core_without_ps_keeps_ordinary_fallback() {
    use fvid::audio::AudioStream;
    let old = root().parent().unwrap().join("aac-er-ep");
    let m: Value =
        serde_json::from_slice(&std::fs::read(old.join("manifest.json")).unwrap()).unwrap();
    let blob = std::fs::read(old.join("packets.bin")).unwrap();
    let mut tested = 0;
    for c in m["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let config = AudioSpecificConfig::parse(&asc).unwrap();
        if config.core.channels != 1 || !matches!(config.core.object_type, 17 | 19) {
            continue;
        }
        tested += 1;
        let mut probe = InBandPsProbe::new(&asc, 24_000).unwrap();
        assert!(probe.requires_ep_recovery());
        for row in c["frames"].as_array().unwrap() {
            assert!(
                !probe
                    .read(packet(&blob, row))
                    .unwrap_or_else(|e| panic!("{} valid mono fallback: {e}", c["name"]))
            );
        }
        let data = std::fs::read(old.join(c["matroska"].as_str().unwrap())).unwrap();
        let mut reader =
            fvid::playback_webm_audio::WebmAudioReader::open(Cursor::new(data), Default::default())
                .unwrap_or_else(|e| panic!("{} mono admission: {e}", c["name"]));
        assert_eq!(reader.channels(), 1);
        let actual = floats(&play(&mut reader));
        let golden = floats(&std::fs::read(old.join(c["golden"].as_str().unwrap())).unwrap());
        assert_eq!(actual.len(), golden.len());
        let worst = actual
            .iter()
            .zip(&golden)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(worst < 1e-7, "{} mono PCM: {worst}", c["name"]);
    }
    assert!(
        tested > 100,
        "protected mono fallback matrix must be exercised"
    );
}
