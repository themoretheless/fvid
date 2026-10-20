use fvid::{container::mp4::Limits, playback::video::mp4::LayeredMp4VideoReader};
#[test]
fn invalid_camera_precision_must_not_be_ignored_as_future_metadata() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge/camera-parameters-bad-precision.mp4");
    let result = LayeredMp4VideoReader::open(
        std::io::Cursor::new(std::fs::read(path).unwrap()),
        Limits::default(),
        1,
        0,
        64 << 20,
    );
    let error = match result {
        Err(e) => e.to_string(),
        Ok(mut reader) => loop {
            match reader.read_frame() {
                Err(e) => break e.to_string(),
                Ok(Some(_)) => (),
                Ok(None) => panic!("invalid camera precision silently accepted"),
            }
        },
    };
    assert!(
        error.contains("HEVC VPS camera precision exceeds range"),
        "{error}"
    );
}

fn root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/playback-errors/hevc-wedge")
}
#[test]
fn camera_metadata_preserves_static_deferred_and_sparse_values() {
    use fvid::codec::hevc_vps::LayeredVps;
    let parse = |name: &str| {
        LayeredVps::parse(
            &std::fs::read(root().join(format!("camera-parameters-{name}.vps"))).unwrap(),
            1 << 20,
        )
        .unwrap()
    };
    let opaque = parse("unrelated-namespace");
    assert!(opaque.future_extension);
    assert!(
        opaque.three_d_extension.is_none(),
        "unrelated profile namespaces retain opaque extension semantics"
    );
    let vps = parse("static");
    let c = vps.three_d_extension.as_ref().unwrap();
    assert_eq!(c.precision, 5);
    assert_eq!(
        c.views.iter().map(|v| v.view_index).collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!(c.views[0].pairs.is_empty());
    assert_eq!(c.views[1].pairs[0].reference_view, 0);
    assert_eq!(c.views[1].pairs[0].parameters, Some([3, -4, 5, -6]));
    assert_eq!(c.views[2].pairs.len(), 2);
    assert_eq!(c.views[2].pairs[0].parameters, Some([128, -64, 7, -8]));
    assert_eq!(c.views[2].pairs[1].reference_view, 1);
    assert_eq!(c.views[2].pairs[1].parameters, Some([-11, 12, -13, 14]));
    assert!(c.views.iter().all(|v| !v.in_slice_header));
    let deferred = parse("deferred");
    let c = deferred.three_d_extension.as_ref().unwrap();
    assert_eq!(c.precision, 2);
    for view in &c.views[1..] {
        assert!(view.in_slice_header);
        assert!(view.pairs.iter().all(|p| p.parameters.is_none()));
    }
    let sparse = parse("sparse");
    let c = sparse.three_d_extension.as_ref().unwrap();
    assert_eq!(c.views[1].pairs[0].reference_view, 65535);
    assert_eq!(
        c.views[1].pairs[0].parameters,
        Some([i32::MAX, -i32::MAX, 0, 1])
    );
    assert!(
        c.retained_bytes() < 1024,
        "sparse references must not allocate a dense view matrix"
    );
}
#[test]
fn malformed_camera_metadata_produces_specific_parameter_and_movie_errors() {
    use fvid::codec::hevc_vps::LayeredVps;
    for (name, expected) in [
        ("bad-precision", "HEVC VPS camera precision exceeds range"),
        ("duplicate", "duplicate HEVC VPS camera reference"),
        ("bad-reference", "HEVC VPS camera reference exceeds range"),
        ("truncated", "truncated HEVC VPS 3D extension"),
    ] {
        let data = std::fs::read(root().join(format!("camera-parameters-{name}.vps"))).unwrap();
        let error = LayeredVps::parse(&data, 1 << 20).unwrap_err().to_string();
        assert!(error.contains(expected), "{name}: {error}");
        let data = std::fs::read(root().join(format!("camera-parameters-{name}.mp4"))).unwrap();
        let result = LayeredMp4VideoReader::open(
            std::io::Cursor::new(data),
            Limits::default(),
            1,
            0,
            64 << 20,
        );
        let error = match result {
            Err(e) => e.to_string(),
            Ok(mut r) => loop {
                match r.read_frame() {
                    Err(e) => break e.to_string(),
                    Ok(Some(_)) => (),
                    Ok(None) => panic!("{name}: malformed camera silently accepted"),
                }
            },
        };
        assert!(error.contains(expected), "{name}: {error}");
    }
}
#[test]
fn unused_static_camera_parameters_preserve_owned_movie_pixels_and_timing() {
    let decode = |name: &str| {
        let data = std::fs::read(root().join(format!("camera-parameters-{name}.mp4"))).unwrap();
        let mut r = LayeredMp4VideoReader::open(
            std::io::Cursor::new(data),
            Limits::default(),
            1,
            0,
            64 << 20,
        )
        .unwrap();
        let mut previous = None;
        for _ in 0..2 {
            let mut frames = Vec::new();
            while let Some(f) = r.read_frame().unwrap() {
                let p = &f.decoded.picture;
                frames.push((
                    f.decoded.layer_id,
                    f.decoded.poc,
                    f.presentation_time.ticks,
                    f.duration.ticks,
                    p.planes
                        .iter()
                        .map(|p| p.samples().to_vec())
                        .collect::<Vec<_>>(),
                ));
            }
            assert_eq!(frames.len(), 6);
            if let Some(p) = previous {
                assert_eq!(frames, p);
            }
            previous = Some(frames);
            r.rewind();
        }
        previous.unwrap()
    };
    assert_eq!(decode("none"), decode("static"));
}
