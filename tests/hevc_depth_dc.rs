use fvid::{
    Result,
    codec::{
        hevc_cabac::Syntax,
        hevc_depth_dc::{CodingUnit, read_cu_offsets, read_dc_only, read_offsets},
        hevc_residual::ResidualBins,
    },
};
use std::collections::VecDeque;

#[derive(Default)]
struct Bins(VecDeque<(Option<&'static str>, bool)>);
impl Bins {
    fn decision(&mut self, name: &'static str, value: bool) {
        self.0.push_back((Some(name), value));
    }
    fn bypass(&mut self, value: bool) {
        self.0.push_back((None, value));
    }
    fn absolute(&mut self, value: u32) {
        for i in 0..3 {
            self.decision("abs", value > i);
            if value <= i {
                return;
            }
        }
        // Independent EG0 code: unary order, followed by the remainder.
        let code = value - 3 + 1;
        let order = 31 - code.leading_zeros();
        for _ in 0..order {
            self.bypass(true);
        }
        self.bypass(false);
        for shift in (0..order).rev() {
            self.bypass(code & (1 << shift) != 0);
        }
    }
}
impl ResidualBins for Bins {
    fn decision(&mut self, syntax: Syntax, increment: usize) -> Result<bool> {
        assert_eq!(increment, 0);
        let expected = match syntax {
            Syntax::DepthDcPresent => "present",
            Syntax::DepthDcAbs => "abs",
            Syntax::DepthDcOnly => "only",
            _ => panic!("wrong syntax"),
        };
        let (kind, bit) = self.0.pop_front().expect("unexpected context read");
        assert_eq!(kind, Some(expected));
        Ok(bit)
    }
    fn bypass(&mut self) -> Result<bool> {
        let (kind, bit) = self.0.pop_front().expect("unexpected bypass read");
        assert_eq!(kind, None);
        Ok(bit)
    }
}

#[test]
fn depth_dc_context_prefix_suffix_sign_and_presence() {
    for dim in [false, true] {
        for intra in [false, true] {
            for dc_only in [false, true] {
                if !dim && !dc_only {
                    continue;
                }
                for value in (0..1024).chain([65535, 1 << 20, (1 << 30) - 1]) {
                    for negative in [false, true] {
                        let mut bins = Bins::default();
                        if intra && dc_only {
                            bins.decision("present", true);
                        }
                        let magnitude = value + u32::from(!dim);
                        for _ in 0..if dim { 2 } else { 1 } {
                            bins.absolute(value);
                            if magnitude != 0 {
                                bins.bypass(negative);
                            }
                        }
                        let expected = if negative {
                            -(magnitude as i32)
                        } else {
                            magnitude as i32
                        };
                        assert_eq!(
                            read_offsets(&mut bins, intra, dc_only, dim).unwrap(),
                            [expected, if dim { expected } else { 0 }]
                        );
                        assert!(bins.0.is_empty());
                    }
                }
            }
        }
        let mut absent = Bins::default();
        absent.decision("present", false);
        assert_eq!(
            read_offsets(&mut absent, true, true, dim).unwrap(),
            [i32::from(!dim), 0]
        );
        assert!(absent.0.is_empty());
    }
}

#[test]
fn depth_dc_rejects_ineligible_blocks_and_unbounded_suffix() {
    let mut bins = Bins::default();
    assert!(read_offsets(&mut bins, false, false, false).is_err());
    assert!(bins.0.is_empty());
    for _ in 0..3 {
        bins.decision("abs", true);
    }
    for _ in 0..31 {
        bins.bypass(true);
    }
    assert_eq!(
        read_offsets(&mut bins, false, true, false)
            .unwrap_err()
            .to_string(),
        "HEVC depth DC magnitude overflow"
    );
}

#[test]
fn authored_arithmetic_streams_decode_depth_dc_and_terminate() {
    use fvid::codec::hevc_cabac::{HevcCabac, SliceType};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-depth-dc");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("manifest.json")).unwrap()).unwrap();
    for case in manifest["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let slice = match case["slice"].as_str().unwrap() {
            "I" => SliceType::I,
            "P" => SliceType::P,
            "B" => SliceType::B,
            _ => unreachable!(),
        };
        let mut bins = HevcCabac::new(
            &bytes,
            0,
            slice,
            case["swap"].as_bool().unwrap(),
            case["qp"].as_i64().unwrap() as i32,
        )
        .unwrap();
        for block in case["blocks"].as_array().unwrap() {
            let dc_only = read_dc_only(
                &mut bins,
                CodingUnit {
                    intra: block["intra"].as_bool().unwrap(),
                    full_partition: true,
                    skipped: false,
                    skip_intra: false,
                    enabled: [true; 2],
                },
            )
            .unwrap();
            assert_eq!(dc_only, block["dc_only"].as_bool().unwrap());
            let got = read_offsets(
                &mut bins,
                block["intra"].as_bool().unwrap(),
                block["dc_only"].as_bool().unwrap(),
                block["dim"].as_bool().unwrap(),
            )
            .unwrap();
            let expected: Vec<i32> = block["offsets"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_i64().unwrap() as i32)
                .collect();
            assert_eq!(got.as_slice(), expected, "{}: {block}", case["file"]);
        }
        for cu in case["cu_blocks"].as_array().unwrap() {
            let dim = std::array::from_fn(|i| cu["dim"][i].as_bool().unwrap());
            let expected: [[i32; 2]; 4] = std::array::from_fn(|i| {
                std::array::from_fn(|j| cu["offsets"][i][j].as_i64().unwrap() as i32)
            });
            assert_eq!(
                read_cu_offsets(&mut bins, true, true, false, false, dim).unwrap(),
                expected
            );
        }
        assert!(bins.terminate().unwrap(), "{}", case["file"]);
    }
}

#[test]
fn dc_only_presence_does_not_consume_unavailable_syntax() {
    for intra in [false, true] {
        for full_partition in [false, true] {
            for skipped in [false, true] {
                for skip_intra in [false, true] {
                    for enabled in [[false, false], [true, false], [false, true], [true, true]] {
                        let present = full_partition
                            && !skipped
                            && !skip_intra
                            && enabled[usize::from(!intra)];
                        for signalled in [false, true] {
                            let mut bins = Bins::default();
                            if present {
                                bins.decision("only", signalled);
                            }
                            let cu = CodingUnit {
                                intra,
                                full_partition,
                                skipped,
                                skip_intra,
                                enabled,
                            };
                            assert_eq!(read_dc_only(&mut bins, cu).unwrap(), present && signalled);
                            assert!(bins.0.is_empty());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn coding_unit_offsets_follow_raster_partition_and_dim_selection() {
    for mask in 0..16 {
        let dim = std::array::from_fn(|i| mask & (1 << i) != 0);
        let mut bins = Bins::default();
        let mut expected = [[0; 2]; 4];
        for i in 0..4 {
            if dim[i] {
                for segment in 0..2 {
                    let value = (i * 2 + segment) as u32;
                    bins.absolute(value);
                    if value != 0 {
                        bins.bypass(segment != 0);
                    }
                    expected[i][segment] = if segment == 0 {
                        value as i32
                    } else {
                        -(value as i32)
                    };
                }
            }
        }
        assert_eq!(
            read_cu_offsets(&mut bins, true, true, false, false, dim).unwrap(),
            expected
        );
        assert!(bins.0.is_empty());
    }
    for intra in [false, true] {
        let mut bins = Bins::default();
        assert_eq!(
            read_cu_offsets(&mut bins, intra, false, false, true, [true; 4]).unwrap(),
            [[0; 2]; 4]
        );
        assert!(bins.0.is_empty());
    }
    for (intra, nxn, only, skip) in [
        (false, true, false, false),
        (true, true, true, false),
        (true, false, true, true),
    ] {
        let mut bins = Bins::default();
        assert!(read_cu_offsets(&mut bins, intra, nxn, only, skip, [false; 4]).is_err());
        assert!(bins.0.is_empty());
    }
}
