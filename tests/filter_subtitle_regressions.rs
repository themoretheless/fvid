use std::path::{Path, PathBuf};

fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("playback-errors")
        .join("filter-subtitle")
        .join(name)
}

#[test]
fn srt_conversion_accepts_dot_decimals_trailing_coords_and_uppercase_entities() {
    let source = fixture_path("uppercase-entity-coords.srt");
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch196-srt-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let destination = dir.join("out.mkv");
    let stats = fvid::native_subtitle::try_convert(&source, &destination, &[])
        .expect("SRT with dot decimals and trailing coordinate tags should parse")
        .expect("uppercase HTML entities should convert natively without fallback");
    assert_eq!(stats.cues, 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ass_conversion_accepts_spaced_case_insensitive_keys_and_variable_fraction_digits() {
    let source = fixture_path("spaced-format-millis.ass");
    let dir = std::env::temp_dir().join(format!(
        "fvid-batch196-ass-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let destination = dir.join("out.mkv");
    let stats = fvid::native_subtitle::try_convert(&source, &destination, &[])
        .expect("ASS with 1-digit and 3-digit fractional seconds should parse")
        .expect("ASS with spaced lowercase format/dialogue keys should convert natively");
    assert_eq!(stats.cues, 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drawbox_rejects_negative_thickness_and_accepts_double_quoted_options() {
    assert!(
        fvid_media::owned_draw::Draw::box_filter("x=0:y=0:w=8:h=8:t=-1").is_err(),
        "negative drawbox thickness must be rejected"
    );
    assert!(
        fvid_media::owned_draw::Draw::grid_filter("x=0:y=0:w=8:h=8:t=-2").is_err(),
        "negative drawgrid thickness must be rejected"
    );
    let draw = fvid_media::owned_draw::Draw::box_filter(
        r#"x="1":y="1":w="2":h="2":color="red":t="fill":enable="between(n,0,2)""#,
    )
    .expect("double-quoted drawbox options should parse");
    let mut rgba = vec![0u8; 4 * 4 * 4];
    draw.apply_rgba(&mut rgba, 4, 4, 0, Some(0.0))
        .expect("drawbox apply_rgba should succeed");
    assert_eq!(&rgba[(1 * 4 + 1) * 4..(1 * 4 + 1) * 4 + 4], &[255, 0, 0, 0]);
}

#[test]
fn fade_and_colorhold_validate_16bit_sample_precision_before_mutation() {
    let mut bad_fade = [0u8, 0, 0, 0, 255, 255];
    let original = bad_fade;
    assert!(
        fvid_media::owned_fade::Fade::parse("t=in:s=0:n=10:c=red")
            .unwrap()
            .apply_rgb(&mut bad_fade, 12, 3, 5)
            .is_err(),
        "12-bit fade should be rejected or validate precision"
    );
    assert_eq!(bad_fade, original);

    let mut bad_hold = [0u8, 0, 0, 0, 255, 255];
    assert!(
        fvid_media::owned_colorhold::ColorHold::parse("color=red")
            .unwrap()
            .apply_rgb(&mut bad_hold, 12, 3)
            .is_err(),
        "12-bit colorhold should be rejected"
    );
}
