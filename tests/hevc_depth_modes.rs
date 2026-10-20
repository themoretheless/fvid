use fvid::codec::{
    hevc_cabac::{HevcCabac, SliceType},
    hevc_depth_dc::{IntraMode, read_intra_mode},
    hevc_wedge::Table,
};

#[test]
fn arithmetic_intra_modes_respect_enable_flags_sizes_and_context_initialization() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("depth-modes.json")).unwrap()).unwrap();
    let tables: Vec<_> = [4, 8, 16, 32]
        .into_iter()
        .map(|n| Table::new(n, 1 << 20).unwrap())
        .collect();
    for case in manifest["cases"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(case["file"].as_str().unwrap())).unwrap();
        let slice = match case["slice"].as_str().unwrap() {
            "I" => SliceType::I,
            "P" => SliceType::P,
            _ => SliceType::B,
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
            let log = block["log"].as_u64().unwrap() as u8;
            let enabled = std::array::from_fn(|i| block["enabled"][i].as_bool().unwrap());
            let mode =
                read_intra_mode(&mut bins, log, enabled, 1, tables.get(usize::from(log - 2)))
                    .unwrap();
            let expected = match block["mode"].as_str().unwrap() {
                "ordinary" => IntraMode::Ordinary,
                "contour" => IntraMode::Contour,
                _ => IntraMode::Wedge(block["index"].as_u64().unwrap() as usize),
            };
            assert_eq!(mode, expected, "{} {block}", case["file"]);
        }
        assert!(bins.terminate().unwrap());
    }
}

#[test]
fn contour_is_forbidden_in_bla_idr_and_unavailable_syntax_is_not_read() {
    use fvid::{
        Result,
        codec::{hevc_cabac::Syntax, hevc_residual::ResidualBins},
    };
    struct Bins(std::collections::VecDeque<bool>);
    impl ResidualBins for Bins {
        fn decision(&mut self, syntax: Syntax, index: usize) -> Result<bool> {
            assert!(matches!(
                syntax,
                Syntax::DepthNoDim | Syntax::DepthIntraMode
            ));
            assert_eq!(index, 0);
            Ok(self.0.pop_front().expect("unexpected decision"))
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("unexpected wedge index")
        }
    }
    for enabled in [[false, true], [true, true]] {
        for nal in 16..=21 {
            let bits = if enabled[0] {
                vec![false, true]
            } else {
                vec![false]
            };
            let mut bins = Bins(bits.into());
            let result = read_intra_mode(&mut bins, 4, enabled, nal, None);
            if nal == 21 {
                assert_eq!(result.unwrap(), IntraMode::Contour);
            } else {
                assert_eq!(
                    result.unwrap_err().to_string(),
                    "HEVC IRAP depth block cannot use contour"
                );
            }
            assert!(bins.0.is_empty());
        }
    }
    for (log, enabled) in [(6, [true, true]), (4, [false, false])] {
        let mut bins = Bins(Default::default());
        assert_eq!(
            read_intra_mode(&mut bins, log, enabled, 1, None).unwrap(),
            IntraMode::Ordinary
        );
    }
}
