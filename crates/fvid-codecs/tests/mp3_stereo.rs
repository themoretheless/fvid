use fvid_codecs::{
    mp3_frame::{Granule, Version},
    mp3_hybrid::Window,
    mp3_scalefactors::ScaleFactors,
    mp3_stereo::process,
};
fn window(n: u8) -> Window {
    match n {
        0 => Window::Normal,
        1 => Window::Start,
        2 => Window::Short,
        3 => Window::Stop,
        _ => panic!(),
    }
}
fn factors(w: Window) -> ScaleFactors {
    ScaleFactors {
        long: [0; 22],
        short: [[0; 3]; 13],
        long_bits: [0; 22],
        short_bits: [[0; 3]; 13],
        preflag: false,
        intensity_scale: false,
        window: w,
    }
}
#[test]
fn joint_stereo_all_rates_layouts_bounds_positions_and_lsf_scales_match_oracles() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-stereo/stereo.bin");
    let bytes = std::fs::read(path).unwrap();
    assert_eq!(&bytes[..8], b"FVMP3T1\0");
    let count = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    assert_eq!(count, 630);
    let mut at = 12;
    for case in 0..count {
        let h = &bytes[at..at + 9];
        at += 9;
        let rate = u32::from_be_bytes(h[..4].try_into().unwrap());
        let version = match h[4] {
            3 => Version::Mpeg1,
            2 => Version::Mpeg2,
            0 => Version::Mpeg25,
            _ => panic!(),
        };
        let g = Granule {
            window: window(h[5]),
            mixed: h[6] != 0,
            ..Granule::default()
        };
        let mut sf = factors(g.window);
        sf.intensity_scale = h[8] != 0;
        sf.long.copy_from_slice(&bytes[at..at + 22]);
        at += 22;
        for band in &mut sf.short {
            band.copy_from_slice(&bytes[at..at + 3]);
            at += 3;
        }
        sf.long_bits.copy_from_slice(&bytes[at..at + 22]);
        at += 22;
        for band in &mut sf.short_bits {
            band.copy_from_slice(&bytes[at..at + 3]);
            at += 3;
        }
        let input: [[f64; 576]; 2] = std::array::from_fn(|_| {
            std::array::from_fn(|_| {
                let v = i16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
                at += 2;
                f64::from(v)
            })
        });
        let actual = process(&input[0], &input[1], version, rate, &g, &g, &sf, 1, h[7]).unwrap();
        for (line, value) in actual.iter().flatten().enumerate() {
            let expected = f64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
            at += 8;
            assert!(
                (value - expected).abs() <= expected.abs() * 3e-14 + 1e-14,
                "case {case} rate {rate} coefficient {line}: {value} != {expected}"
            );
        }
    }
    assert_eq!(at, bytes.len());
}
#[test]
fn independent_stereo_ignores_extensions_and_ms_preserves_energy() {
    let l = std::array::from_fn(|i| (i as f64 - 256.0) / 16.0);
    let r = std::array::from_fn(|i| ((i * 7) % 97) as f64 / 32.0);
    let g = Granule::default();
    let sf = factors(Window::Normal);
    for mode in [0, 2] {
        for extension in 0..4 {
            assert_eq!(
                process(&l, &r, Version::Mpeg1, 44100, &g, &g, &sf, mode, extension).unwrap(),
                [l, r]
            );
        }
    }
    let out = process(&l, &r, Version::Mpeg1, 44100, &g, &g, &sf, 1, 2).unwrap();
    for i in 0..576 {
        let input = l[i] * l[i] + r[i] * r[i];
        let output = out[0][i] * out[0][i] + out[1][i] * out[1][i];
        assert!((input - output).abs() <= input * 1e-14);
        assert!((out[0][i] - (l[i] + r[i]) / 2.0f64.sqrt()).abs() < 1e-14);
    }
}
#[test]
fn invalid_stereo_and_failed_output_preserve_caller_arrays() {
    let left = [1.; 576];
    let right = [0.; 576];
    let g = Granule::default();
    let sf = factors(Window::Normal);
    let short = Granule {
        window: Window::Short,
        ..g.clone()
    };
    assert!(process(&left, &right, Version::Mpeg1, 44100, &g, &short, &sf, 1, 2).is_err());
    for window in [Window::Start, Window::Short, Window::Stop] {
        let a = Granule {
            window,
            mixed: false,
            ..g
        };
        let b = Granule {
            window,
            mixed: true,
            ..g
        };
        assert!(
            process(
                &left,
                &right,
                Version::Mpeg1,
                44100,
                &a,
                &b,
                &factors(window),
                1,
                2
            )
            .is_err()
        );
    }
    for (mode, extension) in [(3, 0), (0, 4), (4, 0)] {
        assert!(
            process(
                &left,
                &right,
                Version::Mpeg1,
                44100,
                &g,
                &g,
                &sf,
                mode,
                extension
            )
            .is_err()
        );
    }
    let mut bad = sf.clone();
    bad.long_bits[20] = 6;
    assert!(process(&left, &right, Version::Mpeg2, 22050, &g, &g, &bad, 1, 1).is_err());
    bad = sf.clone();
    bad.long[20] = 1; // zero-width fields imply position 0.
    assert!(process(&left, &right, Version::Mpeg2, 22050, &g, &g, &bad, 1, 1).is_err());
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut input = left;
        input[575] = value;
        assert!(process(&input, &right, Version::Mpeg1, 44100, &g, &g, &sf, 0, 0).is_err());
        assert_eq!(input[575].to_bits(), value.to_bits());
    }
    let huge = [f64::MAX; 576];
    assert!(process(&huge, &huge, Version::Mpeg1, 44100, &g, &g, &sf, 1, 2).is_err());
    assert_eq!(huge, [f64::MAX; 576]);
    assert_eq!(left, [1.; 576]);
    assert_eq!(right, [0.; 576]);
}
