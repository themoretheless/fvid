use fvid_media::owned_aac::adts_crc::{self, Region};
use serde_json::Value;
use std::io::Cursor;
fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/playback-errors/adts-crc.json")).unwrap()
}
fn bytes(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/playback-errors")
            .join(name),
    )
    .unwrap()
}
fn hex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .chunks_exact(2)
        .map(|b| u8::from_str_radix(std::str::from_utf8(b).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn owned_adts_regions_and_checksum_match_authored_boundaries_and_polynomial_division() {
    let blob = include_bytes!("fixtures/playback-errors/adts-crc-packets.bin");
    for case in manifest()["cases"].as_array().unwrap() {
        let config = hex(case["asc"].as_str().unwrap());
        for row in case["frames"].as_array().unwrap() {
            let start = row["offset"].as_u64().unwrap() as usize;
            let payload = &blob[start..start + row["bytes"].as_u64().unwrap() as usize];
            let expected: Vec<_> = row["regions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| Region {
                    start: r["start"].as_u64().unwrap() as usize,
                    end: r["end"].as_u64().unwrap() as usize,
                    width: r["width"].as_u64().unwrap() as usize,
                })
                .collect();
            assert_eq!(
                adts_crc::regions(payload, &config)
                    .unwrap_or_else(|e| panic!("{}: {e}", case["name"])),
                expected,
                "{}",
                case["name"]
            );
            let header: [u8; 7] = hex(row["header"].as_str().unwrap()).try_into().unwrap();
            let crc = row["crc"].as_u64().unwrap() as u16;
            assert_eq!(
                adts_crc::checksum(&header, payload, &config).unwrap(),
                crc,
                "{}",
                case["name"]
            );
            adts_crc::verify(&header, crc, payload, &config).unwrap();
            for shift in 0..16 {
                assert_eq!(
                    adts_crc::verify(&header, crc ^ (1 << shift), payload, &config)
                        .unwrap_err()
                        .to_string(),
                    "ADTS CRC mismatch"
                );
            }
        }
    }
}

#[test]
fn own_crc_companion_videos_and_adts_have_identical_complete_nonzero_pcm() {
    for case in manifest()["cases"].as_array().unwrap() {
        let data = bytes(case["adts"].as_str().unwrap());
        let mut adts = Vec::new();
        fvid_media::owned_aac::decode_adts_pcm(
            Cursor::new(&data),
            &mut adts,
            None,
            &Default::default(),
        )
        .unwrap_or_else(|e| panic!("{}: {e}", case["name"]));
        let mut video = Vec::new();
        fvid_media::owned_mp4_audio::decode_mp4_audio_pcm(
            Cursor::new(bytes(case["video"]["file"].as_str().unwrap())),
            &mut video,
            None,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(adts, video, "{}", case["name"]);
        assert_eq!(
            adts.len(),
            4096 * case["channels"].as_u64().unwrap() as usize * 4
        );
        assert!(adts
            .chunks_exact(4)
            .any(|b| f32::from_le_bytes(b.try_into().unwrap()) != 0.));
    }
}

#[test]
fn invalid_adts_checksums_are_refused_before_decode_and_poison_stream_reader() {
    for case in manifest()["cases"].as_array().unwrap() {
        let bad = bytes(case["invalid"].as_str().unwrap());
        assert_eq!(
            fvid_media::owned_aac::adts::Aac::parse(&bad, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS CRC mismatch"
        );
        assert_eq!(
            fvid::container::adts::Aac::parse(&bad, &Default::default())
                .unwrap_err()
                .to_string(),
            "ADTS CRC mismatch"
        );
        if case["explicit"].as_bool().unwrap() {
            assert_eq!(
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&bad))
                    .err()
                    .unwrap()
                    .to_string(),
                "ADTS CRC mismatch"
            );
            assert_eq!(
                fvid::container::adts::StreamReader::open(Cursor::new(&bad))
                    .err()
                    .unwrap()
                    .to_string(),
                "ADTS CRC mismatch"
            );
        } else {
            let mut owned =
                fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&bad)).unwrap();
            let mut root = fvid::container::adts::StreamReader::open(Cursor::new(&bad)).unwrap();
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
        let mut later = bytes(case["adts"].as_str().unwrap());
        let first = fvid_media::owned_aac::adts::header(&later)
            .unwrap()
            .frame_bytes;
        later[first + 8] ^= 1;
        let mut owned =
            fvid_media::owned_aac::adts::StreamReader::open(Cursor::new(&later)).unwrap();
        let mut root = fvid::container::adts::StreamReader::open(Cursor::new(&later)).unwrap();
        assert_eq!(owned.next_packet().unwrap(), root.next_packet().unwrap());
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
        let mut output = Vec::new();
        assert_eq!(
            fvid_media::owned_aac::decode_adts_pcm(
                Cursor::new(&bad),
                &mut output,
                None,
                &Default::default()
            )
            .unwrap_err()
            .to_string(),
            "ADTS CRC mismatch"
        );
        assert!(output.is_empty());
    }
}
