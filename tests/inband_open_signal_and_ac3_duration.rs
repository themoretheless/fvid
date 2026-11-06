#![cfg(feature = "player")]
use fvid::{
    audio::{AudioStep, AudioStream},
    color::{ColourDescription, hdr::HdrMetadata},
    container::mp4::Limits,
    media::probe,
    playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader},
    playback_native::{AvcColour, RawFrame},
    playback_webm_audio::WebmAudioReader,
};
use std::{io::Cursor, path::PathBuf, time::Duration};

const BUDGET: usize = 64 << 20;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/inband-open-signal-and-ac3-duration")
}

fn data(name: &str) -> Vec<u8> {
    std::fs::read(root().join(name)).unwrap()
}

fn manifest() -> serde_json::Value {
    serde_json::from_slice(&data("manifest.json")).unwrap()
}

fn assert_colour_eq(actual: AvcColour, expected: AvcColour) {
    assert_eq!(actual.full, expected.full);
    assert!((actual.kr - expected.kr).abs() < 1e-9);
    assert!((actual.kb - expected.kb).abs() < 1e-9);
}

fn check_hdr(actual: HdrMetadata, expected: &serde_json::Value) {
    let master = actual
        .mastering
        .expect("mastering display metadata must be present");
    for (point, value) in [
        (master.green, &expected["primaries"][0]),
        (master.blue, &expected["primaries"][1]),
        (master.red, &expected["primaries"][2]),
        (master.white, &expected["white"]),
    ] {
        assert!((point.x - value[0].as_u64().unwrap() as f64 / 50000.0).abs() < 1e-9);
        assert!((point.y - value[1].as_u64().unwrap() as f64 / 50000.0).abs() < 1e-9);
    }
    assert!(
        (master.max_luminance as f64 - expected["maximum"].as_u64().unwrap() as f64 / 10000.0)
            .abs()
            < 0.001
    );
    assert!(
        (master.min_luminance as f64 - expected["minimum"].as_u64().unwrap() as f64 / 10000.0)
            .abs()
            < 1e-8
    );
    assert_eq!(
        actual.light.max_cll,
        expected["cll"].as_u64().unwrap() as f32
    );
    assert_eq!(
        actual.light.max_fall,
        expected["fall"].as_u64().unwrap() as f32
    );
}

#[test]
fn matroska_avc_and_hevc_preserve_inband_opening_colour_and_hdr_across_rewind_and_seek() {
    let m = manifest();

    // 1. AVC Matroska in-band SPS VUI override
    let expected_avc_signal = ColourDescription {
        primaries: 9,
        transfer: 16,
        matrix: 9,
        full_range: true,
    };
    let mut avc_mkv =
        WebmVideoReader::open_software(Cursor::new(data("avc-inband-vui.mkv")), BUDGET).unwrap();
    assert_eq!(avc_mkv.bitstream_colour(), expected_avc_signal);
    for round in 0..3 {
        if round == 1 {
            avc_mkv.rewind();
            assert_eq!(avc_mkv.bitstream_colour(), expected_avc_signal);
        } else if round == 2 {
            assert_eq!(avc_mkv.seek_to_sync(0).unwrap(), 0);
            assert_eq!(avc_mkv.bitstream_colour(), expected_avc_signal);
        }
        for i in 0..2 {
            let RawFrame::Planar(frame) = avc_mkv.read_frame_raw().unwrap().unwrap() else {
                panic!("expected planar AVC frame");
            };
            assert_eq!(
                frame.frame.data,
                data(m["avc_frames"][i].as_str().unwrap()),
                "avc mkv frame {i}"
            );
            assert_eq!(avc_mkv.bitstream_colour(), expected_avc_signal);
        }
        assert!(avc_mkv.read_frame_raw().unwrap().is_none());
    }

    // 2. Single-layer HEVC Matroska in-band SPS VUI + HDR SEI
    let expected_hevc_signal = ColourDescription {
        primaries: 1,
        transfer: 13,
        matrix: 0,
        full_range: true,
    };
    let mut hevc_mkv =
        WebmVideoReader::open_software(Cursor::new(data("hevc-single-inband.mkv")), BUDGET)
            .unwrap();
    assert_eq!(hevc_mkv.bitstream_colour(), expected_hevc_signal);
    check_hdr(hevc_mkv.bitstream_hdr(), &m["hdr0"]);
    for round in 0..3 {
        if round == 1 {
            hevc_mkv.rewind();
            assert_eq!(hevc_mkv.bitstream_colour(), expected_hevc_signal);
            check_hdr(hevc_mkv.bitstream_hdr(), &m["hdr0"]);
        } else if round == 2 {
            assert_eq!(hevc_mkv.seek_to_sync(0).unwrap(), 0);
            assert_eq!(hevc_mkv.bitstream_colour(), expected_hevc_signal);
            check_hdr(hevc_mkv.bitstream_hdr(), &m["hdr0"]);
        }
        for i in 0..3 {
            let RawFrame::Planar(p) = hevc_mkv.read_frame_raw().unwrap().unwrap() else {
                panic!("expected Identity RGB packed planar frame from in-band VUI");
            };
            let oracle = std::fs::read(
                root()
                    .parent()
                    .unwrap()
                    .join(format!("hevc-identity-rgb-8-full-r0-crop0-{i}.rgb")),
            )
            .unwrap();
            assert_eq!(p.frame.subsampling, None);
            assert_eq!(p.frame.data, oracle, "hevc mkv frame {i}");
            assert_eq!(hevc_mkv.bitstream_colour(), expected_hevc_signal);
            check_hdr(hevc_mkv.bitstream_hdr(), &m["hdr0"]);
        }
        assert!(hevc_mkv.read_frame_raw().unwrap().is_none());
    }

    // 3. Multilayer HEVC Matroska with empty hvcC and in-band parameters + layer-3 HDR SEI
    let mut layered_mkv =
        WebmVideoReader::open_software(Cursor::new(data("hevc-layered-inband.mkv")), BUDGET)
            .unwrap();
    check_hdr(layered_mkv.bitstream_hdr(), &m["hdr1"]);
    let layered_oracle = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    for round in 0..3 {
        if round == 1 {
            layered_mkv.rewind();
            check_hdr(layered_mkv.bitstream_hdr(), &m["hdr1"]);
        } else if round == 2 {
            assert_eq!(layered_mkv.seek_to_sync(0).unwrap(), 0);
            check_hdr(layered_mkv.bitstream_hdr(), &m["hdr1"]);
        }
        let RawFrame::Planar(p) = layered_mkv.read_frame_raw().unwrap().unwrap() else {
            panic!("expected planar layered HEVC frame");
        };
        assert_eq!(p.frame.data, &layered_oracle[..p.frame.data.len()]);
        assert_eq!(layered_mkv.selected_layer_id(), Some(3));
        check_hdr(layered_mkv.bitstream_hdr(), &m["hdr1"]);
        assert!(layered_mkv.read_frame_raw().unwrap().is_none());
    }
}

#[test]
fn mp4_single_layer_multilayer_and_mixed_hevc_preserve_inband_opening_colour_and_hdr() {
    let m = manifest();

    // 1. MP4 avc3 in-band SPS VUI override
    let expected_avc_signal = ColourDescription {
        primaries: 9,
        transfer: 16,
        matrix: 9,
        full_range: true,
    };
    let expected_avc_colour = AvcColour {
        kr: 0.2627,
        kb: 0.0593,
        full: true,
    };
    let mut avc_mp4 = Mp4VideoReader::open_software(
        Cursor::new(data("avc-inband-vui.mp4")),
        Limits::default(),
        BUDGET,
    )
    .unwrap();
    assert_eq!(avc_mp4.bitstream_colour(), expected_avc_signal);
    assert_colour_eq(avc_mp4.active_colour().unwrap(), expected_avc_colour);
    for round in 0..2 {
        if round == 1 {
            avc_mp4.rewind();
            assert_eq!(avc_mp4.bitstream_colour(), expected_avc_signal);
            assert_colour_eq(avc_mp4.active_colour().unwrap(), expected_avc_colour);
        }
        for i in 0..2 {
            let f = avc_mp4.read_frame().unwrap().unwrap();
            assert_eq!(
                f.packed.unwrap().frame.data,
                data(m["avc_frames"][i].as_str().unwrap()),
                "avc mp4 frame {i}"
            );
            assert_eq!(avc_mp4.bitstream_colour(), expected_avc_signal);
            assert_colour_eq(avc_mp4.active_colour().unwrap(), expected_avc_colour);
        }
        assert!(avc_mp4.read_frame().unwrap().is_none());
    }

    // 2. MP4 single-layer hev1 in-band SPS VUI + HDR SEI
    let expected_hevc_signal = ColourDescription {
        primaries: 1,
        transfer: 13,
        matrix: 0,
        full_range: true,
    };
    let expected_hevc_colour = AvcColour {
        kr: 0.2126,
        kb: 0.0722,
        full: true,
    };
    let mut hevc_mp4 = Mp4VideoReader::open_software(
        Cursor::new(data("hevc-single-inband.mp4")),
        Limits::default(),
        BUDGET,
    )
    .unwrap();
    assert_eq!(hevc_mp4.bitstream_colour(), expected_hevc_signal);
    assert_colour_eq(hevc_mp4.active_colour().unwrap(), expected_hevc_colour);
    check_hdr(hevc_mp4.bitstream_hdr(), &m["hdr0"]);
    for round in 0..2 {
        if round == 1 {
            hevc_mp4.rewind();
            assert_eq!(hevc_mp4.bitstream_colour(), expected_hevc_signal);
            assert_colour_eq(hevc_mp4.active_colour().unwrap(), expected_hevc_colour);
            check_hdr(hevc_mp4.bitstream_hdr(), &m["hdr0"]);
        }
        for i in 0..3 {
            let f = hevc_mp4.read_frame().unwrap().unwrap();
            let packed = f.packed.unwrap();
            let oracle = std::fs::read(
                root()
                    .parent()
                    .unwrap()
                    .join(format!("hevc-identity-rgb-8-full-r0-crop0-{i}.rgb")),
            )
            .unwrap();
            assert_eq!(packed.frame.subsampling, None);
            assert_eq!(packed.frame.data, oracle, "hevc mp4 frame {i}");
            assert_eq!(hevc_mp4.bitstream_colour(), expected_hevc_signal);
            assert_colour_eq(hevc_mp4.active_colour().unwrap(), expected_hevc_colour);
            check_hdr(hevc_mp4.bitstream_hdr(), &m["hdr0"]);
        }
        assert!(hevc_mp4.read_frame().unwrap().is_none());
    }

    // 3. MP4 multilayer SingleView hev1 with empty hvcC + in-band layer-3 HDR SEI
    let layered_oracle = std::fs::read(
        root()
            .parent()
            .unwrap()
            .join("hevc-pcm-reference-rext8.yuv"),
    )
    .unwrap();
    let mut layered_mp4 = Mp4VideoReader::open_software(
        Cursor::new(data("hevc-layered-inband.mp4")),
        Limits::default(),
        BUDGET,
    )
    .unwrap();
    check_hdr(layered_mp4.bitstream_hdr(), &m["hdr1"]);
    for round in 0..2 {
        if round == 1 {
            layered_mp4.rewind();
            check_hdr(layered_mp4.bitstream_hdr(), &m["hdr1"]);
        }
        let f = layered_mp4.read_frame().unwrap().unwrap();
        let packed = f.packed.unwrap().frame.data.clone();
        assert_eq!(packed, &layered_oracle[..packed.len()]);
        assert_eq!(layered_mp4.selected_layer_id(), Some(3));
        check_hdr(layered_mp4.bitstream_hdr(), &m["hdr1"]);
        assert!(layered_mp4.read_frame().unwrap().is_none());
    }

    // 4. MP4 MixedHevc (multilayer hev1 + VP9) with in-band layer-3 HDR SEI in sample 0
    let vp9_oracle = data("vp9-64.yuv");
    let mut mixed_mp4 = Mp4VideoReader::open_software(
        Cursor::new(data("hevc-mixed-inband.mp4")),
        Limits::default(),
        BUDGET,
    )
    .unwrap();
    check_hdr(mixed_mp4.bitstream_hdr(), &m["hdr1"]);
    for round in 0..2 {
        if round == 1 {
            mixed_mp4.rewind();
            check_hdr(mixed_mp4.bitstream_hdr(), &m["hdr1"]);
        }
        let f0 = mixed_mp4.read_frame().unwrap().unwrap();
        let p0 = f0.packed.unwrap().frame.data.clone();
        assert_eq!(p0, &layered_oracle[..p0.len()]);
        assert_eq!(mixed_mp4.selected_layer_id(), Some(3));
        check_hdr(mixed_mp4.bitstream_hdr(), &m["hdr1"]);

        let f1 = mixed_mp4.read_frame().unwrap().unwrap();
        let p1: Vec<u8> = f1
            .picture
            .y
            .iter()
            .chain(&f1.picture.cb)
            .chain(&f1.picture.cr)
            .map(|&n| u8::try_from(n).unwrap())
            .collect();
        assert_eq!(p1, vp9_oracle);
        assert_eq!(mixed_mp4.selected_layer_id(), None);
        assert_eq!(mixed_mp4.bitstream_hdr(), HdrMetadata::default());
        assert!(mixed_mp4.read_frame().unwrap().is_none());
    }
}

#[test]
fn matroska_avc_media_probe_reports_profile_and_level_matching_mp4() {
    let mkv_probe = probe(&root().join("avc-inband-vui.mkv")).unwrap();
    let mp4_probe = probe(&root().join("avc-inband-vui.mp4")).unwrap();
    assert_eq!(mkv_probe.streams.len(), 1);
    assert_eq!(mp4_probe.streams.len(), 1);
    assert_eq!(
        mkv_probe.streams[0].profile.as_deref(),
        Some("High 4:4:4 Predictive")
    );
    assert_eq!(mkv_probe.streams[0].level, Some(51));
    assert_eq!(mkv_probe.streams[0].profile, mp4_probe.streams[0].profile);
    assert_eq!(mkv_probe.streams[0].level, mp4_probe.streams[0].level);
}

#[test]
fn matroska_ac3_and_eac3_without_segment_duration_report_indexed_duration_on_open() {
    for file in [
        "ac3-no-segment-duration-default.mkv",
        "ac3-no-segment-duration-gap.mkv",
        "eac3-no-segment-duration.mkv",
    ] {
        let mut reader =
            WebmAudioReader::open(Cursor::new(data(file)), Default::default()).unwrap();
        assert_eq!(
            reader.duration(),
            Some(Duration::from_millis(96)),
            "{file}: duration must be available immediately on open before decoding to EOF"
        );
        let mut dec = reader.make_decoder().unwrap();
        let mut total_frames = 0usize;
        let width = reader.channels() as usize;
        while let Some(step) = reader.next_step().unwrap() {
            if let AudioStep::Encoded(p) = step {
                if let Some(decoded) = dec.decode_packet(&p.data, p.pts, p.duration as u64).unwrap()
                {
                    if let Some(presented) = reader
                        .present_decoded(decoded.packet, decoded.source_pts)
                        .unwrap()
                    {
                        total_frames += presented.data.len() / (width * 4);
                    }
                }
            }
        }
        while let Some(decoded) = dec.finish_packet().unwrap() {
            if let Some(presented) = reader
                .present_decoded(decoded.packet, decoded.source_pts)
                .unwrap()
            {
                total_frames += presented.data.len() / (width * 4);
            }
        }
        reader.validate_eof().unwrap();
        assert_eq!(total_frames, 3 * 1536, "{file}: decoded frame count");
        assert_eq!(reader.duration(), Some(Duration::from_millis(96)));
        reader.rewind();
        assert_eq!(reader.duration(), Some(Duration::from_millis(96)));
    }
}
