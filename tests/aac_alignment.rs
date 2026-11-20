use fvid::audio::AudioStream;
use serde_json::Value;
use std::io::Cursor;

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/aac-alignment")
}
fn manifest() -> Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
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
    output
}

#[test]
fn authored_movies_isolate_aac_byte_alignment_without_changing_syntax() {
    let m = manifest();
    assert_eq!(m["programmes"].as_array().unwrap().len(), 33);
    let packets = std::fs::read(root().join("packets.bin")).unwrap();
    for c in m["programmes"].as_array().unwrap() {
        let variants = c["variants"].as_array().unwrap();
        let control = &variants[0];
        let mut output = Vec::new();
        let stats = fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(control["video"].as_str().unwrap())).unwrap(),
            &mut output,
        )
        .unwrap();
        assert_eq!(stats.sample_frames, control["samples"].as_u64().unwrap());
        assert!(output.iter().any(|&v| v != 0));
        for v in &variants[1..] {
            assert_eq!(v["asc"], control["asc"]);
            let mut changes = 0;
            for (a, b) in control["frames"]
                .as_array()
                .unwrap()
                .iter()
                .zip(v["frames"].as_array().unwrap())
            {
                assert_eq!(a["syntax_bits"], b["syntax_bits"]);
                let bits = a["syntax_bits"].as_u64().unwrap() as usize;
                let bytes = a["bytes"].as_u64().unwrap() as usize;
                let at = a["offset"].as_u64().unwrap() as usize;
                let bt = b["offset"].as_u64().unwrap() as usize;
                let (a, b) = (&packets[at..at + bytes], &packets[bt..bt + bytes]);
                assert_eq!(&a[..bits / 8], &b[..bits / 8]);
                if bits % 8 != 0 {
                    let mask = 0xffu8 << (8 - bits % 8);
                    assert_eq!(a[bits / 8] & mask, b[bits / 8] & mask);
                }
                changes += usize::from(a != b);
            }
            assert!(changes > 0);
        }
    }
}

#[test]
fn arbitrary_aac_alignment_plays_seeks_and_rewinds_with_identical_pcm() {
    for c in manifest()["programmes"].as_array().unwrap() {
        let variants = c["variants"].as_array().unwrap();
        let mut expected = Vec::new();
        fvid::native_media::decode_mp4_aac_pcm(
            &std::fs::read(root().join(variants[0]["video"].as_str().unwrap())).unwrap(),
            &mut expected,
        )
        .unwrap();
        for v in variants {
            let movie = std::fs::read(root().join(v["video"].as_str().unwrap())).unwrap();
            let mut actual = Vec::new();
            fvid::native_media::decode_mp4_aac_pcm(&movie, &mut actual)
                .unwrap_or_else(|e| panic!("{}: {e}", v["name"]));
            assert_eq!(actual, expected, "{} whole PCM", v["name"]);
            let mut mkv = Vec::new();
            fvid::native_media::decode_matroska_aac_reader(
                fvid::container::webm::WebmReader::open(
                    Cursor::new(std::fs::read(root().join(v["file"].as_str().unwrap())).unwrap()),
                    Default::default(),
                )
                .unwrap(),
                &mut mkv,
                None,
            )
            .unwrap_or_else(|e| panic!("{} Matroska: {e}", v["name"]));
            assert_eq!(mkv, expected);
            let mut reader = fvid::playback_mp4_audio::Mp4AudioReader::open(
                Cursor::new(movie),
                Default::default(),
            )
            .unwrap();
            assert_eq!(play(&mut reader), expected);
            reader.rewind();
            assert_eq!(play(&mut reader), expected);
            for sample in [13, 512, 7] {
                let at = sample * i64::from(reader.timescale()) / i64::from(reader.sample_rate());
                let landed = reader.seek_to(at);
                let offset = (landed.max(0) as u128 * u128::from(reader.sample_rate()))
                    .div_ceil(u128::from(reader.timescale())) as usize
                    * usize::from(reader.channels())
                    * 4;
                assert_eq!(
                    play(&mut reader),
                    expected[offset.min(expected.len())..],
                    "{} seek {sample}",
                    v["name"]
                );
            }
        }
    }
}
