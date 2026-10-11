//! Composed reference-sample acceptance, not scalable slice/DPB playback.
use fvid::codec::hevc_resample::MotionSource;
use fvid::codec::{
    hevc_pps::{ColourMappingPicture, LayeredPps, LayeredPpsContext},
    hevc_resample::Resampling,
    hevc_sps::{LayeredSps, LayeredSpsContext},
    hevc_vps::LayeredVps,
};
fn data(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/playback-errors/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}
fn motions(v: &serde_json::Value) -> Vec<fvid::codec::hevc_motion::Motion> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            std::array::from_fn(|c| {
                if entry[c].is_null() {
                    None
                } else {
                    Some(fvid::codec::hevc_motion::Vector {
                        reference: entry[c]["reference"].as_u64().unwrap() as u8,
                        mv: std::array::from_fn(|i| entry[c]["mv"][i].as_i64().unwrap() as i16),
                    })
                }
            })
        })
        .collect()
}
fn minimum(mut valid: impl FnMut(usize) -> bool) -> usize {
    let (mut lo, mut hi) = (0, 1 << 20);
    assert!(valid(hi));
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if valid(mid) { hi = mid } else { lo = mid + 1 }
    }
    lo
}
#[test]
fn composed_reference_samples_apply_active_parameters_order_and_peak_memory() {
    use fvid::{
        codec::config::HevcConfig,
        container::mp4::{Limits, Mp4Reader},
    };
    let manifest: serde_json::Value =
        serde_json::from_slice(&data("hevc-reference-samples.json")).unwrap();
    let (mut peak_checks, mut identity_checks, mut order_checks) = (0, 0, 0);
    for case in manifest["cases"].as_array().unwrap() {
        let name = case["prefix"].as_str().unwrap();
        let read = |suffix: &str| data(&format!("{name}.{suffix}"));
        let vps =
            LayeredVps::parse(&read("vps"), 1 << 20).unwrap_or_else(|e| panic!("{name} VPS: {e}"));
        let base = LayeredSps::parse(
            &read("base.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 0,
                output_layer_set: 0,
                active_sps: &[],
            },
            1 << 20,
        )
        .unwrap_or_else(|e| panic!("{name} base: {e}"));
        let current = LayeredSps::parse(
            &read("current.sps"),
            &LayeredSpsContext {
                vps: &vps,
                layer_id: 3,
                output_layer_set: 1,
                active_sps: std::slice::from_ref(&base),
            },
            1 << 20,
        )
        .unwrap_or_else(|e| panic!("{name} current: {e}"));
        let active = [base, current];
        let context = LayeredPpsContext {
            vps: &vps,
            sps: &active[1],
            active_sps: &active,
            active_pps: &[],
        };
        let raw = read("pps");
        let pps = LayeredPps::parse(&raw, &context, 1 << 20)
            .unwrap_or_else(|e| panic!("{name} PPS: {e}"));
        let movie = Mp4Reader::open(std::io::Cursor::new(read("mp4")), Limits::default()).unwrap();
        let config = HevcConfig::parse(&movie.tracks()[0].configuration).unwrap();
        assert_eq!(
            *config
                .arrays
                .iter()
                .find(|a| a.nal_type == 34)
                .unwrap()
                .units
                .last()
                .unwrap(),
            raw
        );
        for (kind, suffix) in [(32, "vps"), (33, "current.sps")] {
            assert_eq!(
                *config
                    .arrays
                    .iter()
                    .find(|a| a.nal_type == kind)
                    .unwrap()
                    .units
                    .last()
                    .unwrap(),
                read(suffix)
            );
        }
        let [rw, rh] = active[0].sps.dimensions.map(|v| v as usize);
        let sub = |fmt| match fmt {
            1 => [2, 2],
            2 => [2, 1],
            _ => [1, 1],
        };
        let rs = sub(active[0].sps.chroma_format);
        let cs = sub(active[1].sps.chroma_format);
        let bytes = read("y4m");
        let mut at = bytes.iter().position(|&v| v == b'\n').unwrap() + 1;
        assert!(
            std::str::from_utf8(&bytes[..at])
                .unwrap()
                .starts_with(&format!("YUV4MPEG2 W{rw} H{rh} "))
        );
        let storage_depth = *active[0].sps.depth.iter().max().unwrap();
        let counts = [rw * rh, rw * rh / rs[0] / rs[1], rw * rh / rs[0] / rs[1]];
        for (frame_index, frame) in case["frames"].as_array().unwrap().iter().enumerate() {
            assert_eq!(&bytes[at..at + 6], b"FRAME\n");
            at += 6;
            let source: [Vec<u16>; 3] = std::array::from_fn(|c| {
                (0..counts[c])
                    .map(|_| {
                        let v = if storage_depth == 8 {
                            u16::from(bytes[at])
                        } else {
                            u16::from_le_bytes([bytes[at], bytes[at + 1]])
                        };
                        at += if storage_depth == 8 { 1 } else { 2 };
                        v
                    })
                    .collect()
            });
            assert_eq!(serde_json::to_value(&source).unwrap(), frame["input"]);
            let snapshot = source.clone();
            let input = || std::array::from_fn(|c| source[c].as_slice());
            let motion = motions(&frame["motion_input"]);
            let result_motion =
                pps.reference_motion(&context, 0, MotionSource::Grid(&motion), 1 << 20);
            if (case["dependency"].as_u64().unwrap() + 1) & 2 != 0 {
                let output = result_motion.unwrap_or_else(|e| panic!("{name} motion: {e}"));
                assert_eq!(output, motions(&frame["motion_output"]), "{name} motion");
                let intra = pps
                    .reference_motion(&context, 0, MotionSource::Intra, 1 << 20)
                    .unwrap();
                assert_eq!(intra.len(), output.len());
                assert!(intra.iter().all(|m| *m == [None; 2]));

                assert!(
                    pps.reference_motion(&context, 0, MotionSource::Grid(&motion), 0)
                        .unwrap_err()
                        .to_string()
                        .contains("budget")
                );
                assert!(
                    pps.reference_motion(&context, 3, MotionSource::Grid(&motion), 1 << 20)
                        .is_err()
                );
            } else {
                assert!(
                    result_motion
                        .unwrap_err()
                        .to_string()
                        .contains("disables inter-layer motion prediction")
                );
            }
            let result = pps.reference_samples(&context, 0, input(), 1 << 20);
            if let Some(message) = case["error"].as_str() {
                let e = result.err().unwrap();
                assert!(matches!(e, fvid::Error::Invalid(_)), "{name}: {e}");
                assert!(e.to_string().contains(message), "{name}: {e}");
                continue;
            }
            let output = result.unwrap_or_else(|e| panic!("{name}: {e}"));
            let replay = pps
                .reference_samples(&context, 0, input(), 1 << 20)
                .unwrap();
            for c in 0..3 {
                assert!(output[c].complete());
                let [w, h] = active[1].sps.dimensions.map(|v| v as usize);
                assert_eq!(
                    output[c].dimensions(),
                    if c == 0 {
                        [w, h]
                    } else {
                        [w / cs[0], h / cs[1]]
                    }
                );
                assert_eq!(
                    serde_json::to_value(output[c].samples()).unwrap(),
                    frame["output"][c],
                    "{name} component {c}"
                );
                assert_eq!(output[c].samples(), replay[c].samples());
            }
            assert_eq!(source, snapshot);
            if name.ends_with("partition-1-1") {
                assert!(frame["reverse_order_diff_samples"].as_u64().unwrap() > 0);
                order_checks += 1;
            }
            assert!(
                pps.reference_samples(&context, 3, input(), 1 << 20)
                    .is_err()
            );
            assert!(
                pps.reference_samples(&context, 63, input(), 1 << 20)
                    .is_err()
            );
            let mut foreign = pps.clone();
            foreign.vps_id ^= 1;
            assert!(
                foreign
                    .reference_samples(&context, 0, input(), 1 << 20)
                    .is_err()
            );
            let no_reference = LayeredPpsContext {
                vps: &vps,
                sps: &active[1],
                active_sps: &active[1..],
                active_pps: &[],
            };
            assert!(
                pps.reference_samples(&no_reference, 0, input(), 1 << 20)
                    .is_err()
            );
            if let Some(table) = pps.colour_mapping.as_ref() {
                let view = || ColourMappingPicture {
                    dimensions: [rw, rh],
                    chroma_format: active[0].sps.chroma_format,
                    depth: active[0].sps.depth,
                    planes: input(),
                };
                let mapped = table.map_picture(view(), 1 << 20).unwrap();
                for c in 0..3 {
                    assert_eq!(
                        serde_json::to_value(mapped[c].samples()).unwrap(),
                        frame["mapped"][c],
                        "{name} mapped {c}"
                    );
                }
                if frame_index == 0
                    && (name.ends_with("partition-1-1") || name.ends_with("swap-3-3"))
                {
                    let mut reference = active[0].sps.clone();
                    reference.depth = table.output_depth;
                    let default = fvid::codec::hevc_pps::ReferenceLocation {
                        layer_id: 0,
                        scaled_offsets: None,
                        region_offsets: None,
                        phases: None,
                    };
                    let resampling = Resampling::new(&active[1].sps, &reference, &default).unwrap();
                    let mapped_input = || std::array::from_fn(|c| mapped[c].samples());
                    let limit = minimum(|b| {
                        table.map_picture(view(), b).is_ok()
                            && resampling.picture(mapped_input(), b).is_ok()
                    });
                    let e = pps
                        .reference_samples(&context, 0, input(), limit)
                        .err()
                        .unwrap_or_else(|| {
                            panic!("{name}: peak memory budget must include mapped storage")
                        });
                    assert!(e.to_string().contains("budget"), "{name}: {e}");
                    peak_checks += 1;
                }
                if frame_index == 0 && name.contains("mapped-identity16") {
                    let limit = minimum(|b| table.map_picture(view(), b).is_ok());
                    let output = pps.reference_samples(&context, 0, input(), limit).unwrap_or_else(|e| panic!("{name}: mapped reference must be reused without second allocation: {e}"));
                    for c in 0..3 {
                        assert_eq!(output[c].samples(), mapped[c].samples());
                    }
                    identity_checks += 1;
                }
            }
        }
        assert_eq!(at, bytes.len());
    }
    assert_eq!((peak_checks, identity_checks, order_checks), (2, 3, 2));
}
