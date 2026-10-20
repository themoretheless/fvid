use fvid_codecs::{
    mp3_frame::Granule,
    mp3_hybrid::Window,
    mp3_scalefactors::ScaleFactors,
    mp3_spectral::{Bands, reconstruct},
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
fn all_rates_windows_mixed_gains_and_every_huffman_magnitude_match_decimal_oracle() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/playback-errors/mp3-spectral/spectra.bin");
    let data = std::fs::read(path).unwrap();
    assert_eq!(&data[..8], b"FVMP3S1\0");
    let count = u32::from_be_bytes(data[8..12].try_into().unwrap());
    assert_eq!(count, 393);
    let mut at = 12;
    for case in 0..count {
        let h = &data[at..at + 12];
        at += 12;
        let rate = u32::from_be_bytes(h[..4].try_into().unwrap());
        let g = Granule {
            window: window(h[4]),
            mixed: h[5] != 0,
            global_gain: h[6],
            scalefac_scale: h[7] != 0,
            subblock_gain: h[9..12].try_into().unwrap(),
            ..Granule::default()
        };
        let mut sf = factors(g.window);
        sf.preflag = h[8] != 0;
        sf.long.copy_from_slice(&data[at..at + 22]);
        at += 22;
        for band in &mut sf.short {
            band.copy_from_slice(&data[at..at + 3]);
            at += 3;
        }
        let q = std::array::from_fn(|_| {
            let n = i32::from_le_bytes(data[at..at + 4].try_into().unwrap());
            at += 4;
            n
        });
        let result = reconstruct(&q, rate, &g, &sf).unwrap();
        for (line, actual) in result.iter().enumerate() {
            let expected = f64::from_le_bytes(data[at..at + 8].try_into().unwrap());
            at += 8;
            assert!(
                (actual - expected).abs() <= expected.abs() * 2e-14,
                "case {case} rate {rate} line {line}: {actual} != {expected}"
            );
            assert!(actual.is_finite());
        }
        let bands = Bands::for_rate(rate).unwrap();
        assert_eq!(bands.long[22], 576);
        assert_eq!(bands.short[13], 192);
        assert_eq!(bands.long[bands.mixed_long_bands], 3 * bands.short[3]);
        assert_eq!(bands.mixed_subbands(), if rate == 8000 { 4 } else { 2 });
    }
    assert_eq!(at, data.len());
}
#[test]
fn invalid_public_parameters_refuse_without_touching_inputs() {
    let sf = factors(Window::Normal);
    let g = Granule::default();
    let input = [0; 576];
    assert!(reconstruct(&input, 12345, &g, &sf).is_err());
    for value in [8207, -8207, i32::MIN, i32::MAX] {
        let mut input = input;
        input[575] = value;
        let saved = input;
        assert!(reconstruct(&input, 44100, &g, &sf).is_err());
        assert_eq!(input, saved);
    }
    let mut bad = sf.clone();
    bad.short[12][2] = 64;
    assert!(reconstruct(&input, 44100, &g, &bad).is_err());
    let mut bad = sf.clone();
    bad.window = Window::Short;
    assert!(reconstruct(&input, 44100, &g, &bad).is_err());
    let g = Granule {
        subblock_gain: [0, 0, 8],
        ..g
    };
    assert!(reconstruct(&input, 44100, &g, &sf).is_err());
}
