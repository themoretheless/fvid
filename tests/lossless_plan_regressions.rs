use fvid::media_info::{LosslessTransform, RotateAngle, ScaleSize};
use std::path::Path;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/lossless-plan/source-8x4.y4m")
}

#[test]
fn lossless_planner_emits_filter_steps_for_all_supported_pixel_filters() {
    let source = fixture();
    let cases = [
        (
            "tmix",
            LosslessTransform {
                tmix: Some("frames=2".into()),
                ..Default::default()
            },
        ),
        (
            "hqdn3d",
            LosslessTransform {
                hqdn3d: Some("4:3:6:4.5".into()),
                ..Default::default()
            },
        ),
        (
            "smartblur",
            LosslessTransform {
                smartblur: Some("lr=1.5:ls=-0.5".into()),
                ..Default::default()
            },
        ),
        (
            "sab",
            LosslessTransform {
                sab: Some("lr=1.5:cr=1.5".into()),
                ..Default::default()
            },
        ),
        (
            "cas",
            LosslessTransform {
                cas: Some("strength=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "vignette",
            LosslessTransform {
                vignette: Some("angle=PI/4".into()),
                ..Default::default()
            },
        ),
        (
            "curves",
            LosslessTransform {
                curves: Some("preset=lighter".into()),
                ..Default::default()
            },
        ),
        (
            "colorbalance",
            LosslessTransform {
                colorbalance: Some("rs=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "colorlevels",
            LosslessTransform {
                colorlevels: Some("rimin=0.05".into()),
                ..Default::default()
            },
        ),
        (
            "colorchannelmixer",
            LosslessTransform {
                colorchannelmixer: Some("rr=0.9:gg=1.0:bb=1.0".into()),
                ..Default::default()
            },
        ),
        (
            "monochrome",
            LosslessTransform {
                monochrome: Some("cb=0.1:cr=-0.1".into()),
                ..Default::default()
            },
        ),
        (
            "grayworld",
            LosslessTransform {
                grayworld: Some(String::new()),
                ..Default::default()
            },
        ),
        (
            "drawbox",
            LosslessTransform {
                drawbox: Some("x=0:y=0:w=4:h=2:color=white".into()),
                ..Default::default()
            },
        ),
        (
            "drawgrid",
            LosslessTransform {
                drawgrid: Some("w=2:h=2:color=white".into()),
                ..Default::default()
            },
        ),
        (
            "lagfun",
            LosslessTransform {
                lagfun: Some("decay=0.9".into()),
                ..Default::default()
            },
        ),
        (
            "bitplanenoise",
            LosslessTransform {
                bitplanenoise: Some("bitplane=1:filter=1".into()),
                ..Default::default()
            },
        ),
        (
            "deband",
            LosslessTransform {
                deband: Some("1thr=0.02".into()),
                ..Default::default()
            },
        ),
        (
            "gradfun",
            LosslessTransform {
                gradfun: Some("strength=1.2".into()),
                ..Default::default()
            },
        ),
        (
            "lenscorrection",
            LosslessTransform {
                lenscorrection: Some("k1=-0.1".into()),
                ..Default::default()
            },
        ),
        (
            "removegrain",
            LosslessTransform {
                removegrain: Some("m0=1".into()),
                ..Default::default()
            },
        ),
        (
            "yaepblur",
            LosslessTransform {
                yaepblur: Some("r=3".into()),
                ..Default::default()
            },
        ),
        (
            "vibrance",
            LosslessTransform {
                vibrance: Some("intensity=0.2".into()),
                ..Default::default()
            },
        ),
        (
            "colorize",
            LosslessTransform {
                colorize: Some("hue=120:saturation=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "exposure",
            LosslessTransform {
                exposure: Some("exposure=0.5".into()),
                ..Default::default()
            },
        ),
        (
            "colorcontrast",
            LosslessTransform {
                colorcontrast: Some("rc=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "colorcorrect",
            LosslessTransform {
                colorcorrect: Some("rl=0.1".into()),
                ..Default::default()
            },
        ),
        (
            "lutyuv",
            LosslessTransform {
                lutyuv: Some("y=val".into()),
                ..Default::default()
            },
        ),
        (
            "colorhold",
            LosslessTransform {
                colorhold: Some("color=red:similarity=0.2".into()),
                ..Default::default()
            },
        ),
        (
            "perspective",
            LosslessTransform {
                perspective: Some(String::new()),
                ..Default::default()
            },
        ),
    ];

    for (name, transform) in cases {
        let plan = fvid::native_plan::transcode_lossless(&source, &transform)
            .unwrap_or_else(|e| panic!("{name} plan failed: {e}"));
        let filter_steps: Vec<_> = plan
            .steps
            .iter()
            .filter(|s| s.action == "filter")
            .collect();
        assert_eq!(
            filter_steps.len(),
            1,
            "{name} expected 1 filter step in transcode_lossless plan, got {filter_steps:?}"
        );
        assert!(
            filter_steps[0].detail.starts_with(&format!("FVid {name}=")),
            "{name} unexpected filter step detail: {}",
            filter_steps[0].detail
        );
    }
}

#[test]
fn lossless_supports_rejects_nonempty_zscale_and_planner_records_framestep() {
    let source = fixture();
    let with_zscale = LosslessTransform {
        zscale: Some("matrix=bt709".into()),
        ..Default::default()
    };
    assert!(
        !fvid::native_lossless::supports(&with_zscale),
        "LosslessTransform with zscale must not be claimed as supported by native_lossless"
    );
    assert!(
        fvid::native_plan::transcode_lossless(&source, &with_zscale).is_err(),
        "transcode_lossless must refuse unsupported zscale"
    );

    let with_framestep = LosslessTransform {
        framestep: Some("2".into()),
        ..Default::default()
    };
    assert!(fvid::native_lossless::supports_framestep(&with_framestep));
    let plan = fvid::native_plan::transcode_lossless(&source, &with_framestep)
        .expect("transcode_lossless should plan supported framestep");
    assert!(
        plan.steps
            .iter()
            .any(|s| s.action == "filter" && s.detail.contains("framestep=2")),
        "expected framestep filter step in plan: {:?}",
        plan.steps
    );
}

#[test]
fn rotated_and_scaled_y4m_export_computes_pixel_aspect_from_post_rotation_canvas() {
    let source = fixture(); // 8x4, A1:1
    let temp = std::env::temp_dir().join(format!("fvid-rotated-aspect-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).unwrap();
    let out = temp.join("rotated-scaled.y4m");

    let transform = LosslessTransform {
        rotate: Some(RotateAngle::parse("90").unwrap()),
        scale: Some(ScaleSize {
            width: 4,
            height: 8,
        }),
        ..Default::default()
    };
    assert_eq!(transform.rotate.unwrap().size(8, 4), (4, 8));
    let (geometry, filters) = fvid::native_lossless::configuration(&transform).unwrap();
    fvid::native_export::export_y4m_pipeline(&source, &out, None, &geometry, &filters).unwrap();
    let bytes = std::fs::read(&out).unwrap();
    let header_end = bytes.iter().position(|&b| b == b'\n').unwrap();
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
    // 8x4 rotated by 90 deg becomes 4x8; scaling to 4x8 is 1:1 on the rotated canvas,
    // so pixel aspect must remain A1:1 (not A4:1 from pre-rotation 8x4)!
    assert!(
        header.contains(" W4 H8 ") && header.contains(" A1:1 "),
        "unexpected Y4M header after 90-degree rotate + 4x8 identity scale: {header}"
    );

    std::fs::remove_dir_all(&temp).unwrap();
}
