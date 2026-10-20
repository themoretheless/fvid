use fvid_media::owned_webm::{Limits, WebmReader};
use std::{fs, io::Cursor, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/matroska-header-stripping")
}
fn cases() -> Vec<serde_json::Value> {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(root().join("manifest.json")).unwrap()).unwrap();
    manifest["cases"].as_array().unwrap().clone()
}
fn read(name: &str) -> fvid_media::owned_webm::Result<WebmReader<Cursor<Vec<u8>>>> {
    WebmReader::open(
        Cursor::new(fs::read(root().join(name)).unwrap()),
        Limits::default(),
    )
}

#[test]
fn controls_decode_all_three_original_synthetic_pictures() {
    for name in ["control.mkv", "laced-control.mkv"] {
        let mut reader = read(name).unwrap();
        reader.scan_all().unwrap();
        assert_eq!(reader.packets.len(), 3);
        for index in 0..3 {
            let decoded = fvid_codecs::codec::vp9_decoder::Decoder::new(32 << 20)
                .decode(&reader.read_packet(index).unwrap())
                .unwrap();
            assert_eq!(decoded.picture.size, [[16, 16], [32, 16], [16, 32]][index]);
        }
    }
}

#[test]
fn restored_headers_participate_in_index_and_packet_budgets() {
    for case in cases() {
        let mut encoded = read(case["file"].as_str().unwrap()).unwrap();
        let control = read(case["control"].as_str().unwrap()).unwrap();
        assert!(
            encoded.estimated_index_payload_bytes().unwrap()
                >= control.estimated_index_payload_bytes().unwrap()
                    + case["prefix_length"].as_u64().unwrap() as usize
        );
        let index = case["probe_packet"].as_u64().unwrap() as usize;
        let packet = encoded.read_packet(index).unwrap();
        assert_eq!(encoded.packet_payload_size(index).unwrap(), packet.len());
        encoded.restrict_packet_bytes(packet.len());
        assert_eq!(encoded.read_packet(index).unwrap(), packet);
        encoded.restrict_packet_bytes(packet.len() - 1);
        let mut out = vec![0xa5, 0x5a];
        let error = encoded.read_packet_into(index, &mut out).unwrap_err();
        assert_eq!(out, [0xa5, 0x5a]);
        assert_eq!(error.to_string(), "WebM packet exceeds budget");
    }
}

#[test]
fn unimplemented_bzip2_compression_remains_a_declared_refusal() {
    let error = match read("unsupported-bzip2.mkv") {
        Ok(_) => panic!("bzip2 support needs its own acceptance checks"),
        Err(error) => error,
    };
    assert!(error.is_unsupported());
    assert_eq!(
        error.to_string(),
        "encoded/encrypted WebM tracks not supported"
    );
}

#[test]
fn opus_lace_control_decodes_three_twenty_millisecond_packets() {
    let data = fs::read(root().join("opus-control.mkv")).unwrap();
    let mut pcm = Vec::new();
    fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
        Cursor::new(data),
        &mut pcm,
        None,
        &Default::default(),
    )
    .unwrap();
    assert_eq!(pcm.len(), 3 * 960 * 4);
}

#[test]
fn stripped_opus_laces_preserve_the_entire_control_pcm() {
    let decode = |name: &str| {
        let mut output = Vec::new();
        fvid_media::owned_matroska_opus::decode_matroska_opus_pcm(
            Cursor::new(fs::read(root().join(name)).unwrap()),
            &mut output,
            None,
            &Default::default(),
        )
        .unwrap();
        output
    };
    let control = decode("opus-control.mkv");
    assert_eq!(control.len(), 3 * 960 * 4);
    for name in ["opus-laced.mkv", "opus-whole.mkv"] {
        assert_eq!(decode(name), control);
    }
}

#[test]
fn header_stripped_lacing_still_requires_a_count_inside_the_block() {
    let error = match read("missing-lace-count.mkv") {
        Ok(_) => panic!("a compression prefix cannot replace a missing lace count"),
        Err(error) => error,
    };
    assert!(!error.is_unsupported());
    assert_eq!(error.to_string(), "truncated Matroska lace header");
}

#[test]
fn header_stripping_restores_every_original_codec_packet_and_clock() {
    for case in cases() {
        let mut encoded = read(case["file"].as_str().unwrap()).unwrap();
        let mut control = read(case["control"].as_str().unwrap()).unwrap();
        encoded.scan_all().unwrap();
        control.scan_all().unwrap();
        assert_eq!(
            encoded.packets.len(),
            case["packet_count"].as_u64().unwrap() as usize
        );
        for index in 0..encoded.packets.len() {
            assert_eq!(
                encoded.read_packet(index).unwrap(),
                control.read_packet(index).unwrap()
            );
            assert_eq!(encoded.packets[index].pts_ns, control.packets[index].pts_ns);
            assert_eq!(
                encoded.packets[index].duration_ns,
                control.packets[index].duration_ns
            );
        }
    }
}

#[test]
fn formerly_unsupported_zlib_fixture_restores_the_control_packets(){
 let mut encoded=read("unsupported-zlib.mkv").unwrap();encoded.scan_all().unwrap();
 let mut control=read("control.mkv").unwrap();control.scan_all().unwrap();
 assert_eq!(encoded.packets.len(),control.packets.len());
 for i in 0..control.packets.len(){assert_eq!(encoded.read_packet(i).unwrap(),control.read_packet(i).unwrap());}
}
