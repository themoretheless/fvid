use std::path::Path;
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}

use fvid_media::owned_aac::NativeAacDecoder;
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&bytes("aac-hcr.json")).unwrap()
}
fn hex(s: &str) -> Vec<u8> {
    s.as_bytes()
        .chunks_exact(2)
        .map(|v| u8::from_str_radix(std::str::from_utf8(v).unwrap(), 16).unwrap())
        .collect()
}
#[test]
fn hcr_native_spectral_codewords_stereo_and_tns_match_scalar_pcm() {
    let blob = bytes("aac-hcr-packets.bin");
    let gold = bytes("aac-hcr-reference.f32le");
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = hex(c["asc"].as_str().unwrap());
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        assert_eq!(
            fvid_media::owned_aac::config::AacConfig::parse(&asc)
                .unwrap()
                .object_type,
            17
        );
        let mut first = vec![];
        for pass in 0..2 {
            if pass != 0 {
                decoder.reset();
            }
            let mut out = vec![];
            for row in c["frames"].as_array().unwrap() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let raw = &blob[at..at + len];
                let saved = decoder.checkpoint();
                let mut bad = raw.to_vec();
                bad.push(0);
                assert!(
                    decoder
                        .decode(&bad)
                        .err()
                        .unwrap()
                        .to_string()
                        .contains("trailing bytes after ER AAC block")
                );
                assert!(decoder.decode(&raw[..raw.len() - 1]).is_err());
                let pcm = decoder.decode(raw).unwrap();
                decoder.restore(&saved).unwrap();
                assert_eq!(decoder.decode(raw).unwrap(), pcm);
                out.extend(pcm);
            }
            let at = c["reference_offset"].as_u64().unwrap() as usize;
            let count = c["reference_bytes"].as_u64().unwrap() as usize;
            assert_eq!(out.len() * 4, count);
            for (i, (a, b)) in out
                .iter()
                .zip(gold[at..at + count].chunks_exact(4))
                .enumerate()
            {
                let b = f32::from_le_bytes(b.try_into().unwrap());
                assert!((a - b).abs() < 1e-7, "{} sample {i}: {a} vs {b}", c["name"]);
            }
            if pass == 0 {
                first = out;
            } else {
                assert_eq!(out, first);
            }
        }
    }
}
#[test]
fn hcr_public_mp4_acceptance_matches_owned_pcm() {
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut out = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut out).unwrap();
        let mut owned = vec![];
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            std::io::Cursor::new(&data),
            &mut owned,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(out, owned);
        assert_eq!(out.len(), c["reference_bytes"].as_u64().unwrap() as usize);
        let gold = bytes("aac-hcr-reference.f32le");
        let at = c["reference_offset"].as_u64().unwrap() as usize;
        for (a, b) in out
            .chunks_exact(4)
            .zip(gold[at..at + out.len()].chunks_exact(4))
        {
            assert!(
                (f32::from_le_bytes(a.try_into().unwrap())
                    - f32::from_le_bytes(b.try_into().unwrap()))
                .abs()
                    < 1e-7
            );
        }
    }
}

use fvid::audio::AudioStream;
fn play(reader: &mut dyn AudioStream) -> Vec<u8> {
    let mut decoder = reader.make_decoder().unwrap();
    let mut output = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if let Some(frame) = decoder
            .decode_packet(&packet.data, packet.pts, packet.duration as u64)
            .unwrap()
        {
            if let Some(pcm) = reader
                .present_decoded(frame.packet, frame.source_pts)
                .unwrap()
            {
                output.extend(pcm.data);
            }
        }
    }
    while let Some(frame) = decoder.finish_packet().unwrap() {
        if let Some(pcm) = reader
            .present_decoded(frame.packet, frame.source_pts)
            .unwrap()
        {
            output.extend(pcm.data);
        }
    }
    assert!(decoder.finish_packet().unwrap().is_none());
    output
}

#[test]
fn hcr_player_ranges_rewind_seek_preserve_pcm_and_channels() {
    use std::{io::Cursor, time::Duration};
    for c in manifest()["cases"].as_array().unwrap() {
        let data = bytes(c["video"]["file"].as_str().unwrap());
        let mut full = vec![];
        fvid::native_media::decode_mp4_aac_pcm(&data, &mut full).unwrap();
        let stride = c["channels"].as_u64().unwrap() as usize * 4;
        for (from, to) in [(130, 230), (10, 60), (130, 230)] {
            let mut out = vec![];
            fvid::native_media::decode_mp4_aac_pcm_interval(
                &data,
                &mut out,
                Some((Duration::from_millis(from), Duration::from_millis(to))),
            )
            .unwrap();
            assert_eq!(
                out,
                full[from as usize * 24 * stride..to as usize * 24 * stride]
            );
        }
        let mut reader =
            fvid::playback_mp4_audio::Mp4AudioReader::open(Cursor::new(&data), Default::default())
                .unwrap();
        assert_eq!(
            (reader.channels(), reader.sample_rate()),
            (c["channels"].as_u64().unwrap() as u16, 24000)
        );
        assert_eq!(play(&mut reader), full);
        reader.rewind();
        assert_eq!(play(&mut reader), full);
        for target in [1100, 5800, 9000, (full.len() / stride) as i64] {
            let landed = reader.seek_to(target);
            assert_eq!(play(&mut reader), full[landed as usize * stride..]);
        }
    }
}

#[test]
fn malformed_hcr_videos_fail_specific_errors_without_changing_history() {
    let m = manifest();
    let blob = bytes("aac-hcr-packets.bin");
    for bad in m["malformed"].as_array().unwrap() {
        let asc = hex(bad["asc"].as_str().unwrap());
        let c = m["cases"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["asc"] == bad["asc"])
            .unwrap();
        let mut decoder = NativeAacDecoder::new(&asc).unwrap();
        let mut control = NativeAacDecoder::new(&asc).unwrap();
        let row = &bad["frame"];
        let at = row["offset"].as_u64().unwrap() as usize;
        let len = row["bytes"].as_u64().unwrap() as usize;
        let broken = &blob[at..at + len];
        for row in c["frames"].as_array().unwrap() {
            let e = decoder.decode(broken).err().unwrap();
            assert!(
                e.to_string().contains(bad["error"].as_str().unwrap()),
                "{}: {e}",
                bad["kind"]
            );
            let at = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            assert_eq!(
                decoder.decode(&blob[at..at + len]).unwrap(),
                control.decode(&blob[at..at + len]).unwrap()
            );
        }
        let mut out = vec![];
        let e = fvid::native_media::decode_mp4_aac_pcm(
            &bytes(bad["video"]["file"].as_str().unwrap()),
            &mut out,
        )
        .err()
        .unwrap();
        assert!(
            e.to_string().contains(bad["error"].as_str().unwrap()),
            "{}: {e}",
            bad["kind"]
        );
    }
}

#[test]
fn hcr_reordering_restores_exact_quantized_coefficients_for_all_books() {
    use fvid_media::owned_aac::{
        aac_ics::IcsInfo,
        aac_spectral::HcrHeader,
        aac_synthesis::{WindowSequence, WindowShape},
        bits::BitReader,
    };
    let m = manifest();
    let regions = bytes("aac-hcr-regions.bin");
    let reference = bytes("aac-hcr-quantized.i16le");
    let mut seen = std::collections::BTreeSet::new();
    for case in m["cases"].as_array().unwrap() {
        for frame in case["frames"].as_array().unwrap() {
            for row in frame["hcr_channels"].as_array().unwrap() {
                let fields = format!(
                    "{:014b}{:06b}",
                    row["bits"].as_u64().unwrap(),
                    row["longest"].as_u64().unwrap()
                );
                let mut raw_header = [0u8; 3];
                for (i, b) in fields.bytes().enumerate() {
                    raw_header[i / 8] |= (b - b'0') << (7 - i % 8);
                }
                let header = HcrHeader::read(
                    &mut BitReader::new(&raw_header),
                    case["channels"].as_u64().unwrap() == 2,
                )
                .unwrap();
                let groups: Vec<u8> = serde_json::from_value(row["groups"].clone()).unwrap();
                let books: Vec<Vec<u8>> = serde_json::from_value(row["books"].clone()).unwrap();
                seen.extend(books.iter().flatten().copied());
                let offsets: Vec<usize> = serde_json::from_value(row["offsets"].clone()).unwrap();
                let info = IcsInfo {
                    sequence: match row["sequence"].as_u64().unwrap() {
                        0 => WindowSequence::OnlyLong,
                        1 => WindowSequence::LongStart,
                        2 => WindowSequence::EightShort,
                        3 => WindowSequence::LongStop,
                        _ => unreachable!(),
                    },
                    shape: WindowShape::Sine,
                    max_sfb: 8,
                    group_lengths: groups,
                    prediction: None,
                };
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let prefix = row["prefix"].as_u64().unwrap() as usize;
                let mut bits = BitReader::new(&regions[at..at + len]);
                bits.skip(prefix).unwrap();
                let out = header
                    .decode(
                        &mut bits,
                        &info,
                        &offsets,
                        &books,
                        case["n"].as_u64().unwrap() as usize,
                    )
                    .unwrap();
                assert_eq!(bits.remaining(), 0);
                let at_ref = row["reference_offset"].as_u64().unwrap() as usize;
                let bytes_ref = row["reference_bytes"].as_u64().unwrap() as usize;
                let gold: Vec<i16> = reference[at_ref..at_ref + bytes_ref]
                    .chunks_exact(2)
                    .map(|v| i16::from_le_bytes(v.try_into().unwrap()))
                    .collect();
                assert_eq!(out, gold, "{}", case["name"]);
                if len > 0 {
                    let mut truncated = BitReader::new(&regions[at..at + len - 1]);
                    let prefix = prefix.min(truncated.remaining());
                    truncated.skip(prefix).unwrap();
                    assert!(
                        header
                            .decode(
                                &mut truncated,
                                &info,
                                &offsets,
                                &books,
                                case["n"].as_u64().unwrap() as usize
                            )
                            .is_err()
                    );
                    assert_eq!(truncated.position(), prefix);
                } else {
                    assert!(out.iter().all(|&q| q == 0));
                }
            }
        }
    }
    for book in (1..=11).chain(16..=31) {
        assert!(seen.contains(&book), "book {book} missing");
    }
    assert!(
        m["coverage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["sets"].as_u64().unwrap() >= 3)
    );
    for key in ["splits", "shifts", "tail"] {
        assert!(
            m["coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r[key].as_u64().unwrap() > 0),
            "{key} not exercised"
        );
    }
}
