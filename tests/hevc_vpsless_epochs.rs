use fvid::{
    codec::config::HevcConfig,
    container::mp4::{Limits, Mp4Reader},
    playback_native::{NativeReader, RawFrame},
};
use std::{io::Cursor, path::PathBuf, time::Duration};
const BUDGET: usize = 64 << 20;
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-vpsless-epochs")
}
fn pixels(f: RawFrame) -> Vec<u8> {
    match f {
        RawFrame::Planar(p) => p.frame.data.clone(),
        RawFrame::Planar8(p) => p.y.iter().chain(&p.cb).chain(&p.cr).copied().collect(),
        RawFrame::Avc { picture, .. } => picture
            .y
            .iter()
            .chain(&picture.cb)
            .chain(&picture.cr)
            .map(|&v| u8::try_from(v).unwrap())
            .collect(),
        _ => panic!("owned planar picture expected"),
    }
}
#[test]
fn native_vpsless_prefix_reaches_layered_epochs_and_survives_seek() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let oracle = std::fs::read(root().join(c["oracle"].as_str().unwrap())).unwrap();
        let mp4 = std::fs::read(root().join(format!("{name}.mp4"))).unwrap();
        let demux = Mp4Reader::open_indexed(Cursor::new(&mp4), Limits::default()).unwrap();
        let cfg = HevcConfig::parse(&demux.tracks()[0].configuration).unwrap();
        assert!(
            cfg.arrays
                .iter()
                .all(|a| a.nal_type != 32 || a.units.is_empty())
        );
        for ext in ["mp4", "mkv"] {
            let data = std::fs::read(root().join(format!("{name}.{ext}"))).unwrap();
            let mut r = NativeReader::software(Cursor::new(data), BUDGET)
                .unwrap_or_else(|e| panic!("{name}.{ext}: {e}"));
            let origin = if ext == "mkv" && c["inline"].as_bool().unwrap() {
                1
            } else {
                0
            };
            for _ in 0..2 {
                for (i, sample) in c["samples"].as_array().unwrap().iter().enumerate() {
                    let p = pixels(
                        r.read_frame_raw()
                            .unwrap_or_else(|e| panic!("{name}.{ext}: {e}"))
                            .unwrap(),
                    );
                    assert_eq!(p, &oracle[..p.len()], "{name}.{ext}");
                    assert_eq!(
                        r.hdr().light.max_cll,
                        c["hdr"][i].as_f64().unwrap() as f32,
                        "{name}.{ext} HDR"
                    );
                    let (ticks, scale) = r.current_pts().unwrap();
                    assert_eq!(
                        u128::try_from(ticks).unwrap() * 25,
                        u128::from(sample.as_u64().unwrap() - origin) * u128::from(scale)
                    );
                }
                assert!(r.read_frame_raw().unwrap().is_none());
                r.rewind().unwrap();
            }
            for (i, sample) in c["samples"].as_array().unwrap().iter().enumerate().rev() {
                let t = sample.as_u64().unwrap() - origin;
                let p = pixels(r.seek_raw(Duration::from_millis(t * 40)).unwrap().unwrap());
                assert_eq!(p, &oracle[..p.len()]);
                assert_eq!(
                    r.hdr().light.max_cll,
                    c["hdr"][i].as_f64().unwrap() as f32,
                    "{name}.{ext} seek HDR"
                );
                let (ticks, scale) = r.current_pts().unwrap();
                assert_eq!(
                    u128::try_from(ticks).unwrap() * 25,
                    u128::from(t) * u128::from(scale)
                );
            }
        }
    }
}

#[test]
fn late_selected_entropy_failure_requires_rewind_and_preserves_prefix_recovery() {
    for ext in ["mp4", "mkv"] {
        let data = std::fs::read(root().join(format!("corrupt-selected.{ext}"))).unwrap();
        let mut r = NativeReader::software(Cursor::new(data), BUDGET).unwrap();
        let first = pixels(r.read_frame_raw().unwrap().unwrap());
        assert_eq!(r.hdr().light.max_cll, 100.0);
        let mut error = None;
        for _ in 0..8 {
            match r.read_frame_raw() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("corrupt selected entropy was silently ignored"),
                Err(e) => {
                    error = Some(e.to_string());
                    break;
                }
            }
        }
        assert!(error.is_some(), "{ext}");
        assert!(r.read_frame_raw().is_err());
        r.rewind().unwrap();
        assert_eq!(pixels(r.read_frame_raw().unwrap().unwrap()), first);
        assert_eq!(r.hdr().light.max_cll, 100.0);
    }
}

#[test]
fn public_frontends_preserve_actual_layer_identity_across_the_handoff() {
    use fvid::playback::video::{mp4::Mp4VideoReader, webm::WebmVideoReader};
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["cases"].as_array().unwrap() {
        let name = c["name"].as_str().unwrap();
        let mut mp4 = Mp4VideoReader::open_software(
            Cursor::new(std::fs::read(root().join(format!("{name}.mp4"))).unwrap()),
            Limits::default(),
            BUDGET,
        )
        .unwrap();
        let mut mkv = WebmVideoReader::open_software(
            Cursor::new(std::fs::read(root().join(format!("{name}.mkv"))).unwrap()),
            BUDGET,
        )
        .unwrap();
        for (i, sample) in c["samples"].as_array().unwrap().iter().enumerate() {
            let f = mp4.read_frame().unwrap().unwrap();
            assert_eq!(f.sample_index, sample.as_u64().unwrap() as usize);
            assert_eq!(
                mp4.selected_layer_id(),
                Some(c["layers"][i].as_u64().unwrap() as u8)
            );
            assert!(mkv.read_frame_raw().unwrap().is_some());
            assert_eq!(
                mkv.selected_layer_id(),
                Some(c["layers"][i].as_u64().unwrap() as u8)
            );
        }
        assert!(mp4.read_frame().unwrap().is_none());
        assert!(mkv.read_frame_raw().unwrap().is_none());
    }
}

#[test]
fn mixed_codec_windows_accept_vpsless_prefixes_with_or_without_later_vps() {
    let m: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root().join("manifest.json")).unwrap()).unwrap();
    for c in m["mixed"].as_array().unwrap() {
        let data =
            std::fs::read(root().join(format!("{}.mp4", c["name"].as_str().unwrap()))).unwrap();
        let mut r = NativeReader::software(Cursor::new(data), BUDGET).unwrap();
        for _ in 0..2 {
            for (i, sample) in c["samples"].as_array().unwrap().iter().enumerate() {
                let oracle = std::fs::read(root().join(c["oracles"][i].as_str().unwrap())).unwrap();
                let p = pixels(r.read_frame_raw().unwrap().unwrap());
                assert!(p == oracle[..p.len()], "{} frame {i} pixels", c["name"]);
                assert_eq!(r.hdr().light.max_cll, c["hdr"][i].as_f64().unwrap() as f32);
                let (ticks, scale) = r.current_pts().unwrap();
                assert_eq!(
                    u128::try_from(ticks).unwrap() * 25,
                    u128::from(sample.as_u64().unwrap()) * u128::from(scale)
                );
            }
            assert!(r.read_frame_raw().unwrap().is_none());
            r.rewind().unwrap();
        }
        for (i, sample) in c["samples"].as_array().unwrap().iter().enumerate().rev() {
            let oracle = std::fs::read(root().join(c["oracles"][i].as_str().unwrap())).unwrap();
            let p = pixels(
                r.seek_raw(Duration::from_millis(sample.as_u64().unwrap() * 40))
                    .unwrap()
                    .unwrap(),
            );
            assert!(p == oracle[..p.len()], "{} seek {i} pixels", c["name"]);
            assert_eq!(r.hdr().light.max_cll, c["hdr"][i].as_f64().unwrap() as f32);
        }
    }
}

#[test]
fn parameter_handoff_snapshots_only_received_bytes_and_keeps_missing_vps_missing() {
    use fvid::codec::{config::NalUnits, hevc_decoder::LayeredParameters, hevc_nal::NalHeader};
    let data = std::fs::read(root().join("base-to-multi-inline.mp4")).unwrap();
    let mut demux = Mp4Reader::open_indexed(Cursor::new(data), Limits::default()).unwrap();
    let template = demux.tracks()[0].configuration.clone();
    let mut bank = LayeredParameters::from_configuration(&template, BUDGET).unwrap();
    let mut received = Vec::new();
    let mut packet = Vec::new();
    for i in 0..=4 {
        demux.read_packet(0, i, &mut packet).unwrap();
        for n in NalUnits::new(&packet, 4).unwrap() {
            let n = n.unwrap();
            let h = NalHeader::parse(n).unwrap();
            if matches!(h.unit_type, 32..=34) {
                bank.receive(n).unwrap();
                received.push(n.to_vec());
            }
        }
        let snapshot = bank.received_configuration(&template, BUDGET).unwrap();
        assert_eq!(&snapshot[..22], &template[..22]);
        let cfg = HevcConfig::parse(&snapshot).unwrap();
        for n in cfg.arrays.iter().flat_map(|a| &a.units) {
            assert!(received.iter().any(|r| r == n));
        }
        assert_eq!(cfg.arrays.iter().any(|a| a.nal_type == 32), i == 4);
        assert!(bank.received_configuration(&template, 16).is_err());
        LayeredParameters::from_configuration(&snapshot, BUDGET).unwrap();
    }
}

#[test]
fn introducing_vps_in_a_running_cvs_does_not_bypass_the_activation_guard() {
    for ext in ["mp4", "mkv"] {
        let data = std::fs::read(root().join(format!("illegal-handoff.{ext}"))).unwrap();
        let error = NativeReader::software(Cursor::new(data), BUDGET)
            .err()
            .expect("illegal activation must be refused");
        assert_eq!(
            error.to_string(),
            "HEVC active VPS changed before sequence end",
            "{ext}"
        );
    }
}
