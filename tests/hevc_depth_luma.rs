use fvid::codec::{
    hevc_cabac::{HevcCabac, SliceType},
    hevc_intra_syntax::{DepthLumaCode, LumaCode, read_depth_luma},
    hevc_wedge::Table,
};
#[test]
fn mixed_prediction_blocks_preserve_interleaving_and_two_pass_order() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge");
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("depth-luma.json")).unwrap()).unwrap();
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
        for unit in case["units"].as_array().unwrap() {
            let log = unit["log"].as_u64().unwrap() as u8;
            let expected: Vec<_> = unit["codes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| match c["mode"].as_str().unwrap() {
                    "ordinary" => DepthLumaCode::Ordinary(LumaCode {
                        mpm: c["mpm"].as_bool().unwrap(),
                        value: c["value"].as_u64().unwrap() as u8,
                    }),
                    "wedge" => DepthLumaCode::Wedge(c["index"].as_u64().unwrap() as usize),
                    _ => DepthLumaCode::Contour,
                })
                .collect();
            let got = read_depth_luma(
                &mut bins,
                true,
                log,
                [unit["enabled"].as_bool().unwrap(); 2],
                1,
                Some(&tables[usize::from(log - 2)]),
            )
            .unwrap();
            assert_eq!(got, expected, "{} {unit}", case["file"]);
        }
        assert!(bins.terminate().unwrap());
    }
}

#[test]
fn invalid_partition_size_consumes_no_entropy() {
    use fvid::{
        Result,
        codec::{hevc_cabac::Syntax, hevc_residual::ResidualBins},
    };
    struct Unread;
    impl ResidualBins for Unread {
        fn decision(&mut self, _: Syntax, _: usize) -> Result<bool> {
            panic!("invalid geometry consumed a context bin")
        }
        fn bypass(&mut self) -> Result<bool> {
            panic!("invalid geometry consumed a bypass bin")
        }
    }
    for (nxn, log) in [(false, 1), (false, 7), (true, 6)] {
        assert_eq!(
            read_depth_luma(&mut Unread, nxn, log, [true; 2], 1, None)
                .unwrap_err()
                .to_string(),
            "invalid HEVC depth intra partition size"
        );
    }
}
