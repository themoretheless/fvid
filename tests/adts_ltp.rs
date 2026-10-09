use serde_json::Value;
use std::{io::Cursor, path::Path, time::Duration};
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn manifest() -> Value {
    serde_json::from_slice(&bytes("adts-ltp.json")).unwrap()
}
#[test]
fn ltp_crc_spans_and_block_extents_match_independent_bit_writer() {
    let blob = bytes("adts-ltp-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let asc = (0..c["asc"].as_str().unwrap().len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&c["asc"].as_str().unwrap()[i..i + 2], 16).unwrap())
            .collect::<Vec<_>>();
        for row in c["frames"].as_array().unwrap() {
            let start = row["offset"].as_u64().unwrap() as usize;
            let len = row["bytes"].as_u64().unwrap() as usize;
            let packet = &blob[start..start + len];
            assert_eq!(
                fvid_media::owned_aac::adts_crc::raw_block_bytes(packet, &asc).unwrap(),
                len
            );
            let spans = fvid_media::owned_aac::adts_crc::regions(packet, &asc).unwrap();
            let expected = row["regions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| fvid_media::owned_aac::adts_crc::Region {
                    start: r["start"].as_u64().unwrap() as usize,
                    end: r["end"].as_u64().unwrap() as usize,
                    width: r["width"].as_u64().unwrap() as usize,
                })
                .collect::<Vec<_>>();
            assert_eq!(spans, expected, "{}", c["name"]);
            if c["name"] == "mono-sbr" {
                assert!(fvid_media::owned_aac::adts_crc::has_sbr_fill(packet, &asc).unwrap());
                // Clock discovery starts from the initial header-bearing block;
                // later SBR payloads legitimately reuse previous header state.
                if row == &c["frames"][0] {
                    assert_eq!(
                        fvid_media::owned_aac::adts::probe_output_rate(packet, &asc).unwrap(),
                        Some(48000)
                    );
                }
            }
        }
    }
}
#[test]
fn active_ltp_single_and_multiblock_adts_match_scalar_pcm_and_ranges() {
    let blob = bytes("adts-ltp-packets.bin");
    for c in manifest()["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let channels = c["channels"].as_u64().unwrap() as usize;
        let rate = c["container_rate"].as_u64().unwrap();
        let samples = c["samples"].as_u64().unwrap();
        let mut expected = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(c["video"]["file"].as_str().unwrap())),
            &mut expected,
            None,
            &Default::default(),
        )
        .unwrap();
        let (gold, width, tolerance) = if let Some(pair) = name.strip_prefix("pair-") {
            (
                bytes(&format!("aac-ltp-pair-{pair}-scalar-reference.f32le")),
                4,
                1e-7,
            )
        } else if name == "mono-sbr" {
            (bytes("aac-ltp-sbr-48000-reference.f64le"), 8, 1e-9)
        } else {
            let point = name.strip_prefix("cce-").unwrap().parse::<usize>().unwrap();
            let point = if point == 3 { 2 } else { point };
            let all = bytes("aac-ltp-phase-reference.f32le");
            (
                all[point * 12288 * 4..(point + 1) * 12288 * 4].to_vec(),
                4,
                1e-7,
            )
        };
        assert_eq!(expected.len() / 4, gold.len() / width);
        for (i, (a, b)) in expected
            .chunks_exact(4)
            .zip(gold.chunks_exact(width))
            .enumerate()
        {
            let a = f32::from_le_bytes(a.try_into().unwrap()) as f64;
            let b = if width == 4 {
                f32::from_le_bytes(b.try_into().unwrap()) as f64
            } else {
                f64::from_le_bytes(b.try_into().unwrap())
            };
            assert!((a - b).abs() < tolerance, "{name} sample {i}: {a} vs {b}");
        }
        for file in c["transports"].as_array().unwrap() {
            let data = bytes(file["file"].as_str().unwrap());
            let parsed = fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default())
                .unwrap_or_else(|e| panic!("{}: {e}", file["file"]));
            let root = fvid::container::adts::Aac::parse(&data, &Default::default()).unwrap();
            let mut stream =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            let mut root_stream =
                fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap();
            assert_eq!(parsed.packets(), c["frames"].as_array().unwrap().len());
            for (i, row) in c["frames"].as_array().unwrap().iter().enumerate() {
                let at = row["offset"].as_u64().unwrap() as usize;
                let len = row["bytes"].as_u64().unwrap() as usize;
                let packet = &blob[at..at + len];
                assert_eq!(parsed.packet(i), packet);
                assert_eq!(root.packet(i), packet);
                assert_eq!(parsed.frames[i].pts, i as u64 * 1024);
                assert_eq!(stream.next_packet().unwrap().unwrap(), packet);
                assert_eq!(root_stream.next_packet().unwrap().unwrap(), packet);
            }
            assert!(stream.next_packet().unwrap().is_none());
            assert!(root_stream.next_packet().unwrap().is_none());
            let mut output = Vec::new();
            let stats = fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&data),
                &mut output,
                None,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(
                (stats.sample_rate as u64, stats.sample_frames),
                (rate, samples)
            );
            assert_eq!(output, expected, "{}", file["file"]);
            let mut exported = Vec::new();
            fvid::native_media::decode_aac_pcm_interval(
                &data,
                &mut exported,
                &Default::default(),
                None,
            )
            .unwrap();
            assert_eq!(exported, expected);
            for (from, to) in [(130, 200), (10, 60), (130, 200)] {
                let mut selected = Vec::new();
                fvid_media::owned_aac::decode_adts_pcm(
                    Cursor::new(&data),
                    &mut selected,
                    Some((Duration::from_millis(from), Duration::from_millis(to))),
                    &Default::default(),
                )
                .unwrap();
                assert_eq!(
                    selected,
                    expected[(from * rate / 1000) as usize * channels * 4
                        ..(to * rate / 1000) as usize * channels * 4]
                );
            }
        }
    }
}
#[test]
fn protected_active_ltp_corruption_reports_crc_and_poisons_stream() {
    for c in manifest()["invalid"].as_array().unwrap() {
        let data = bytes(c["file"].as_str().unwrap());
        assert_eq!(
            fvid_media::owned_aac::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS CRC mismatch"
        );
        assert_eq!(
            fvid::container::adts::Aac::parse(&data, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS CRC mismatch"
        );
        let mut owned =
            fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&data)).unwrap();
        let mut root = fvid::container::adts::StreamReader::open(Cursor::new(&data)).unwrap();
        for _ in 0..3 {
            assert_eq!(
                owned.next_packet().unwrap().unwrap(),
                root.next_packet().unwrap().unwrap()
            );
        }
        assert_eq!(
            owned.next_packet().unwrap_err().to_string(),
            "ADTS CRC mismatch"
        );
        assert_eq!(
            root.next_packet().unwrap_err().to_string(),
            "ADTS CRC mismatch"
        );
        assert!(owned.next_packet().unwrap().is_none());
        assert!(root.next_packet().unwrap().is_none());
    }
}
