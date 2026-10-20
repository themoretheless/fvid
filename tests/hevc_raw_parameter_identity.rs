use fvid::{
    codec::hevc_decoder::HevcDecoder,
    container::mp4::{Limits, Mp4Reader},
    playback_mp4::Mp4VideoReader,
};
use std::{
    io::Cursor,
    path::{Path, PathBuf},
};
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-raw-parameter-identity")
}
fn manifest() -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap()
}
#[test]
fn opaque_parameter_updates_obey_exact_no_update_guidance() {
    for c in manifest()["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root().join(c["file"].as_str().unwrap())).unwrap();
        let mut reader =
            Mp4VideoReader::open_software(Cursor::new(bytes), Limits::default(), 32 << 20).unwrap();
        if c["refusal"].as_bool().unwrap() {
            for _ in 0..2 {
                let mut refused = false;
                for _ in 0..18 {
                    match reader.read_frame() {
                        Ok(Some(_)) => {}
                        Ok(None) => panic!("{} accepted forbidden raw change", c["file"]),
                        Err(e) => {
                            assert_eq!(
                                e.to_string(),
                                "HEVC active parameter SEI forbids parameter updates",
                                "{}",
                                c["file"]
                            );
                            refused = true;
                            break;
                        }
                    }
                }
                assert!(refused);
                reader.rewind();
            }
        } else {
            let control = std::fs::read(
                root()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join(format!("hevc/{}.mp4", c["seed"].as_str().unwrap())),
            )
            .unwrap();
            let mut baseline =
                Mp4VideoReader::open_software(Cursor::new(control), Limits::default(), 32 << 20)
                    .unwrap();
            let mut gold = Vec::new();
            while let Some(f) = baseline.read_frame().unwrap() {
                gold.push(f);
            }
            assert_eq!(gold.len(), 17);
            for _ in 0..2 {
                for target in &gold {
                    let frame = reader.read_frame().unwrap().unwrap();
                    assert_eq!(frame.presentation_time, target.presentation_time);
                    assert_eq!(frame.duration, target.duration);
                    assert_eq!(frame.picture.y, target.picture.y);
                    assert_eq!(frame.picture.cb, target.picture.cb);
                    assert_eq!(frame.picture.cr, target.picture.cr);
                }
                assert!(reader.read_frame().unwrap().is_none());
                reader.rewind();
            }
        }
    }
}
#[test]
fn parameter_only_packets_cannot_bypass_no_update_and_latest_priming_is_retained() {
    for kind in ["vps", "sps", "pps"] {
        let mut movie = Mp4Reader::open(
            std::fs::File::open(root().join(format!("main-ipb-{kind}-forbidden.mp4"))).unwrap(),
            Limits::default(),
        )
        .unwrap();
        let config = movie.tracks()[0].configuration.clone();
        let mut first = Vec::new();
        movie.read_packet(0, 0, &mut first).unwrap();
        let nal = std::fs::read(root().join(format!("main-ipb-{kind}-changed.nal"))).unwrap();
        let packet = [(nal.len() as u32).to_be_bytes().as_slice(), nal.as_slice()].concat();
        let mut decoder = HevcDecoder::from_configuration(&config, 32 << 20).unwrap();
        decoder.decode_packet(&first).unwrap();
        let error = decoder
            .decode_packet(&packet)
            .err()
            .expect("parameter-only overwrite must fail");
        assert_eq!(
            error.to_string(),
            "HEVC active parameter SEI forbids parameter updates"
        );
        decoder.reset();
        decoder.prime_parameters(&packet).unwrap();
        decoder.decode_packet(&first).unwrap();
        assert!(decoder.decode_packet(&packet).unwrap().is_none());
        let original = std::fs::read(root().join(format!("main-ipb-{kind}-original.nal"))).unwrap();
        let original_packet = [
            (original.len() as u32).to_be_bytes().as_slice(),
            original.as_slice(),
        ]
        .concat();
        let error = decoder
            .decode_packet(&original_packet)
            .err()
            .expect("identity must use latest primed version");
        assert_eq!(
            error.to_string(),
            "HEVC active parameter SEI forbids parameter updates"
        );
    }
}

#[test]
fn retained_raw_parameters_respect_budget_and_share_initial_bytes() {
    let read = |name: &str| std::fs::read(root().join(name)).unwrap();
    let movie = Mp4Reader::open(
        Cursor::new(read("main-ipb-sps-large-config.mp4")),
        Limits::default(),
    )
    .unwrap();
    let config = &movie.tracks()[0].configuration;
    let nal = read("main-ipb-sps-large.nal");
    assert!(nal.len() + 4 < 4096);
    fvid::codec::hevc_sps::Sps::parse(&nal, 4096).unwrap();
    let error = HevcDecoder::from_configuration(config, 4096)
        .err()
        .expect("retained bytes must be admitted before publication");
    assert_eq!(
        error.to_string(),
        "HEVC raw parameter storage exceeds decode budget"
    );
    // Initial/current banks share the same Arc rather than charging a second NAL.
    HevcDecoder::from_configuration(config, 5000).unwrap();
    let control = Mp4Reader::open(
        Cursor::new(read("main-ipb-sps-identical.mp4")),
        Limits::default(),
    )
    .unwrap();
    let mut decoder =
        HevcDecoder::from_configuration(&control.tracks()[0].configuration, 4096).unwrap();
    let packet = [(nal.len() as u32).to_be_bytes().as_slice(), nal.as_slice()].concat();
    let error = decoder.prime_parameters(&packet).unwrap_err();
    assert_eq!(
        error.to_string(),
        "HEVC raw parameter storage exceeds decode budget"
    );
    decoder.reset();
    let small = read("main-ipb-sps-changed.nal");
    let packet = [
        (small.len() as u32).to_be_bytes().as_slice(),
        small.as_slice(),
    ]
    .concat();
    decoder.prime_parameters(&packet).unwrap();
}

#[test]
fn pending_no_update_survives_separate_sei_only_packets() {
    for seed in ["main-ipb", "main10-ipb"] {
        for kind in ["vps", "sps", "pps"] {
            let mut movie = Mp4Reader::open(
                std::fs::File::open(root().join(format!("{seed}-{kind}-same-packet-revoked.mp4")))
                    .unwrap(),
                Limits::default(),
            )
            .unwrap();
            let mut first = Vec::new();
            let mut next = Vec::new();
            movie.read_packet(0, 0, &mut first).unwrap();
            movie.read_packet(0, 1, &mut next).unwrap();
            let prefix_end =
                |bytes: &[u8]| 4 + u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize;
            let first_end = prefix_end(&first);
            let second_end = first_end + prefix_end(&first[first_end..]);
            assert_eq!(&first[4..first_end], &[78, 1, 129, 1, 7, 128]);
            assert_eq!(&first[first_end + 4..second_end], &[78, 1, 129, 1, 3, 128]);
            let mut decoder =
                HevcDecoder::from_configuration(&movie.tracks()[0].configuration, 32 << 20)
                    .unwrap();
            for _ in 0..2 {
                assert!(
                    decoder
                        .decode_packet(&first[..first_end])
                        .unwrap()
                        .is_none()
                );
                assert!(
                    decoder
                        .decode_packet(&first[first_end..second_end])
                        .unwrap()
                        .is_none()
                );
                assert!(
                    decoder
                        .decode_packet(&first[second_end..])
                        .unwrap()
                        .is_some()
                );
                assert!(
                    decoder
                        .active_parameter_sets()
                        .unwrap()
                        .no_parameter_set_update
                );
                assert_eq!(
                    decoder.decode_packet(&next).err().unwrap().to_string(),
                    "HEVC active parameter SEI forbids parameter updates"
                );
                decoder.reset();
            }
        }
    }
}
