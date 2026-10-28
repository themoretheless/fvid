use fvid_media::owned_sl::{Config, PacketKind};
use serde_json::Value;
use std::path::PathBuf;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/playback-errors/mpeg4-sl")
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
    let at = row["offset"].as_u64().unwrap() as usize;
    &blob[at..at + row["bytes"].as_u64().unwrap() as usize]
}
#[test]
fn sl_wide_headers_timestamps_and_non_octet_payloads_match_independent_writer() {
    let m = manifest();
    let blob = read("packets.bin");
    assert_eq!(m["vectors"].as_array().unwrap().len(), 35);
    for c in m["vectors"].as_array().unwrap() {
        let raw = read(c["config"].as_str().unwrap());
        let config = Config::parse(&raw).unwrap();
        assert_eq!(config, fvid::codec::sl::Config::parse(&raw).unwrap());
        let data = packet(&blob, &c["packet"]);
        let p = config.parse_packet(data, None).unwrap();
        assert_eq!(p.kind, PacketKind::Data);
        assert!(p.access_unit_start && p.random_access);
        assert_eq!(p.access_unit_end, Some(true));
        assert_eq!(p.header_bytes, c["header_bytes"].as_u64().unwrap() as usize);
        assert_eq!(p.payload, hex(c["payload"].as_str().unwrap()));
        assert_eq!(p.payload_bits, 9);
        assert_eq!(p.ocr, c["value"].as_u64());
        assert_eq!(p.decoding_timestamp, c["value"].as_u64().map(|v| v - 1));
        assert_eq!(p.composition_timestamp, c["value"].as_u64());
        assert_eq!(p.packet_sequence, Some(65535));
        assert_eq!(p.au_sequence, Some(65535));
        assert_eq!(p.degradation_priority, Some(32767));
        assert_eq!(p.access_unit_length, Some(2));
        if let Some(expected) = c["bitrate"].as_str() {
            let field = p.instant_bitrate.unwrap();
            assert_eq!(usize::from(field.bit_len), expected.len());
            for (i, bit) in expected.bytes().enumerate() {
                assert_eq!(field.bit(i as u8), Some(bit == b'1'));
            }
            assert_eq!(field.bit(field.bit_len), None);
            assert_eq!(
                field.as_u64(),
                if expected.len() <= 64 {
                    Some(u64::from_str_radix(expected, 2).unwrap())
                } else {
                    None
                }
            );
        } else {
            assert!(p.instant_bitrate.is_none());
        }
        for end in 0..raw.len() {
            assert!(
                Config::parse(&raw[..end]).is_err(),
                "{} config prefix {end}",
                c["name"]
            );
        }
        for end in 0..p.header_bytes {
            assert!(
                config.parse_packet(&data[..end], None).is_err(),
                "{} packet header prefix {end}",
                c["name"]
            );
        }
        let mut wrong = data.to_vec();
        *wrong.last_mut().unwrap() |= 1;
        let error = config.parse_packet(&wrong, None).unwrap_err().to_string();
        assert!(error.contains("nonzero SL payload padding bits"), "{error}");
        let error = config
            .parse_packet(&data[..p.header_bytes], None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("padding exceeds packet payload"), "{error}");
    }
}
#[test]
fn predefined_custom_start_timestamps_and_malformed_configs_have_explicit_results() {
    let c = Config::parse(&read("mp4.slc")).unwrap();
    // This is a framed SL packet, with explicit zero timestamp-presence
    // flags. MP4 samples supply their clocks outside SL packet syntax.
    let p = c.parse_packet(&[0, 0xab, 0xcd], None).unwrap();
    assert_eq!(
        (p.header_bytes, p.payload_bits, p.access_unit_end),
        (1, 16, Some(true))
    );
    assert_eq!(p.payload, &[0xab, 0xcd]);
    let sample = c.mp4_sample(b"abc", 24).unwrap();
    assert_eq!((sample.data, sample.bit_len), (&b"abc"[..], 24));
    assert_eq!(c.mp4_sample(&[0xff], 3).unwrap().bit_len, 3);
    assert!(
        c.mp4_sample(&[0xff], 9)
            .unwrap_err()
            .to_string()
            .contains("bit boundary disagrees")
    );
    let c = Config::parse(&read("null.slc")).unwrap();
    assert!(
        c.mp4_sample(b"abc", 24)
            .unwrap_err()
            .to_string()
            .contains("requires predefined SL configuration 2")
    );
    assert_eq!(c.timestamp_resolution, 1000);
    assert_eq!(c.start_timestamps, Some((123, 456)));
    assert_eq!(c.parse_packet(b"abc", None).unwrap().payload, b"abc");
    let c = Config::parse(&read("initial.slc")).unwrap();
    assert_eq!(c.start_timestamps, Some(((1 << 33) - 1, 19)));
    let d = c.duration.unwrap();
    assert_eq!(
        (d.timescale, d.access_unit, d.composition_unit),
        (24000, 960, 1024)
    );
    for c in manifest()["malformed"].as_array().unwrap() {
        let error = Config::parse(&read(c["config"].as_str().unwrap()))
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
fn packetized_esc_access_units_reconstruct_exact_information_and_independent_pcm() {
    use fvid_media::owned_aac::{NativeAacDecoder, aac_ep1::AccessUnit};
    let m = manifest();
    let blob = read("packets.bin");
    assert_eq!(
        m["native"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["ps"] == false)
            .count(),
        10
    );
    for c in m["native"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ps"] == false)
    {
        let cfg = Config::parse(&read(c["config"].as_str().unwrap())).unwrap();
        let asc = read(c["asc"].as_str().unwrap());
        let gold = read(c["golden"].as_str().unwrap());
        let mut d = NativeAacDecoder::new(&asc).unwrap();
        for _ in 0..2 {
            let mut actual = Vec::new();
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let mut input = Vec::new();
                for u in row["units"].as_array().unwrap() {
                    let p = cfg.parse_packet(packet(&blob, &u["packet"]), None).unwrap();
                    assert!(p.access_unit_start);
                    assert_eq!(p.access_unit_end, Some(true));
                    assert_eq!(p.payload_bits, u["bits"].as_u64().unwrap() as usize);
                    assert_eq!(p.packet_sequence, Some(i as u16));
                    assert_eq!(p.au_sequence, Some(i as u16));
                    let source = hex(u["source"].as_str().unwrap());
                    for bit in 0..p.payload_bits {
                        assert_eq!(
                            (p.payload[bit / 8] >> (7 - bit % 8)) & 1,
                            (source[bit / 8] >> (7 - bit % 8)) & 1
                        );
                    }
                    input.push(AccessUnit {
                        stream_id: u["stream_id"].as_u64().unwrap() as u16,
                        depends_on: u["depends_on"].as_u64().map(|n| n as u16),
                        data: p.payload,
                        bit_len: p.payload_bits,
                        pts: i64::try_from(p.composition_timestamp.unwrap()).unwrap(),
                        duration: row["duration"].as_u64().unwrap(),
                        timescale: cfg.timestamp_resolution,
                    });
                }
                let checkpoint = d.checkpoint();
                let frame = d.decode_esc(&input).unwrap().unwrap();
                assert_eq!(frame.pts, row["pts"].as_i64().unwrap());
                d.restore(&checkpoint).unwrap();
                assert_eq!(
                    frame.samples,
                    d.decode_esc(&input).unwrap().unwrap().samples
                );
                actual.extend(frame.samples);
            }
            assert_eq!(actual.len() * 4, gold.len());
            let error = actual
                .iter()
                .zip(gold.chunks_exact(4))
                .map(|(a, b)| (a - f32::from_le_bytes(b.try_into().unwrap())).abs())
                .fold(0f32, f32::max);
            assert!(error < 1e-7, "{} PCM {error}", c["name"]);
            d.reset();
        }
    }
}

#[test]
fn implicit_boundaries_idle_padding_and_absent_metadata_do_not_consume_payload_fields() {
    let m = manifest();
    let blob = read("packets.bin");
    for c in m["controls"].as_array().unwrap() {
        let cfg = Config::parse(&read(c["config"].as_str().unwrap())).unwrap();
        let raw = packet(&blob, &c["packet"]);
        let previous = c["previous_end"].as_bool();
        let result = cfg.parse_packet(raw, previous);
        if let Some(expected) = c["error"].as_str() {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "{}: {error}", c["name"]);
            continue;
        }
        let p = result.unwrap();
        match c["kind"].as_str().unwrap() {
            "idle" => {
                assert_eq!(p.kind, PacketKind::Idle);
                assert_eq!(p.payload_bits, 0);
                assert!(p.payload.is_empty());
            }
            "padding" => {
                assert_eq!(p.kind, PacketKind::PaddingOnly);
                assert_eq!(p.payload_bits, 0);
                assert!(p.payload.is_empty());
            }
            "data" => {
                assert_eq!(p.kind, PacketKind::Data);
                assert_eq!(p.payload_bits, 13);
                assert_eq!(p.payload, hex("daa0"));
                assert_eq!(p.access_unit_start, c["start"].as_bool().unwrap());
                assert_eq!(
                    p.access_unit_end,
                    if cfg.use_end || !cfg.use_start {
                        c["end"].as_bool()
                    } else {
                        None
                    }
                );
            }
            _ => unreachable!(),
        }
        if c["name"] == "unknown-au-length" {
            assert!(p.access_unit_length.is_none());
        }
        if c["name"] == "rap-only" {
            assert!(p.random_access);
        }
        if c["name"] == "dts-only" {
            assert_eq!(p.decoding_timestamp, Some(100));
            assert!(p.composition_timestamp.is_none());
        }
        if c["name"] == "absent-fields" {
            assert!(p.ocr.is_none());
            assert!(p.instant_bitrate.is_none());
            assert!(p.degradation_priority.is_none());
            assert!(p.decoding_timestamp.is_none());
            assert!(p.composition_timestamp.is_none());
        }
        if cfg.use_end && !cfg.use_start && p.kind == PacketKind::Data {
            let error = cfg.parse_packet(raw, None).unwrap_err().to_string();
            assert!(
                error.contains("requires preceding packet context"),
                "{error}"
            );
        }
    }
}

#[test]
fn packetized_ps_escs_preserve_delayed_frame_identity_and_complete_pcm() {
    use fvid_media::owned_aac::{aac_ep1::AccessUnit, aac_ps_native::NativePsAacDecoder};
    let m = manifest();
    let blob = read("packets.bin");
    let mut checked = 0;
    for c in m["native"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["ps"] == true)
    {
        checked += 1;
        let cfg = Config::parse(&read(c["config"].as_str().unwrap())).unwrap();
        let asc = read(c["asc"].as_str().unwrap());
        let plain_asc = hex(c["plain_asc"].as_str().unwrap());
        let logical = read(c["logical"].as_str().unwrap());
        let build = |asc: &[u8]| {
            if c["signal"] == "implicit" {
                NativePsAacDecoder::new_with_in_band_ps(asc, c["rate"].as_u64().unwrap() as u32)
                    .unwrap()
            } else {
                NativePsAacDecoder::new(asc).unwrap()
            }
        };
        let mut d = build(&asc);
        let mut reference = build(&plain_asc);
        let mut actual = Vec::new();
        let mut expected = Vec::new();
        for row in c["frames"].as_array().unwrap() {
            let input = row["units"]
                .as_array()
                .unwrap()
                .iter()
                .map(|u| {
                    let p = cfg.parse_packet(packet(&blob, &u["packet"]), None).unwrap();
                    AccessUnit {
                        stream_id: u["stream_id"].as_u64().unwrap() as u16,
                        depends_on: u["depends_on"].as_u64().map(|v| v as u16),
                        data: p.payload,
                        bit_len: p.payload_bits,
                        pts: i64::try_from(p.composition_timestamp.unwrap()).unwrap(),
                        duration: row["duration"].as_u64().unwrap(),
                        timescale: cfg.timestamp_resolution,
                    }
                })
                .collect::<Vec<_>>();
            let checkpoint = d.checkpoint();
            let frame = d.decode_esc(&input).unwrap();
            d.restore(&checkpoint).unwrap();
            assert_eq!(frame, d.decode_esc(&input).unwrap());
            let raw = reference.decode(packet(&logical, row)).unwrap();
            assert_eq!(frame, raw, "{} delayed identity", c["name"]);
            if let Some(frame) = frame {
                actual.extend(frame.pcm);
            }
            if let Some(frame) = raw {
                expected.extend(frame.pcm);
            }
        }
        while let Some(frame) = d.finish().unwrap() {
            let raw = reference.finish().unwrap().unwrap();
            assert_eq!(frame, raw);
            actual.extend(frame.pcm);
            expected.extend(raw.pcm);
        }
        assert!(reference.finish().unwrap().is_none());
        assert_eq!(actual, expected);
        assert!(actual.iter().all(|v| v.is_finite()));
    }
    assert_eq!(checked, 2);
}
