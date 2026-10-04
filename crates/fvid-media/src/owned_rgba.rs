//! Owned named/hex RGBA parsing, including explicit decimal/hex opacity.
use std::hash::{BuildHasher, Hasher};
type Result<T> = std::result::Result<T, String>;
pub fn parse(value: &str) -> Result<[u8; 4]> {
    let value = value.trim().trim_matches('\'');
    if value.len() > 256 || value.is_empty() || value.contains('\0') {
        return Err("invalid color".into());
    }
    let (rgb, opacity) = value
        .split_once('@')
        .map_or((value, None), |(c, a)| (c, Some(a)));
    let lower = rgb.to_ascii_lowercase();
    let mut result = if lower == "random" || lower == "bikeshed" {
        let mut random = std::collections::hash_map::RandomState::new().build_hasher();
        random.write(value.as_bytes());
        (random.finish() as u32).to_be_bytes()
    } else if let Some((_, color)) = crate::owned_color_names::NAMED
        .iter()
        .find(|(name, _)| *name == lower)
    {
        [color[0], color[1], color[2], 255]
    } else {
        let hex = rgb
            .strip_prefix('#')
            .or_else(|| rgb.strip_prefix("0x"))
            .or_else(|| rgb.strip_prefix("0X"))
            .unwrap_or(rgb);
        if !matches!(hex.len(), 6 | 8) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("color requires a named color or RRGGBB[AA] hex".into());
        }
        let v = u32::from_str_radix(hex, 16).map_err(|_| "invalid hex color")?;
        if hex.len() == 6 {
            [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
        } else {
            v.to_be_bytes()
        }
    };
    if let Some(text) = opacity {
        result[3] = if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            u8::from_str_radix(hex, 16).map_err(|_| "invalid hex opacity")?
        } else {
            let alpha = text.parse::<f64>().map_err(|_| "invalid color opacity")?;
            if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                return Err("color opacity must be from 0 to 1".into());
            }
            (alpha * 255.) as u8
        };
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_hex_and_alpha_forms_share_one_parser() {
        assert_eq!(parse("AliceBlue").unwrap(), [240, 248, 255, 255]);
        assert_eq!(parse("DarkSlateGray@0x40").unwrap(), [47, 79, 79, 64]);
        assert_eq!(parse("#11223344").unwrap(), [17, 34, 51, 68]);
        assert_eq!(parse("0X11223344@.5").unwrap(), [17, 34, 51, 127]);
        assert_eq!(parse("blue@0").unwrap(), [0, 0, 255, 0]);
        assert_eq!(parse("green").unwrap(), [0, 128, 0, 255]);
        for (name, rgb) in crate::owned_color_names::NAMED {
            let v = parse(name).unwrap();
            assert_eq!(&v[..3], rgb);
        }
    }
    #[test]
    fn malformed_colors_and_opacity_are_refused() {
        for value in [
            "",
            "unknown",
            "12345",
            "1234567",
            "white@NaN",
            "red@-1",
            "red@1.1",
            "red@0x100",
            "red@0x",
            "red@0@1",
            "red\0",
        ] {
            assert!(parse(value).is_err(), "{value:?}");
        }
    }
    #[test]
    fn random_color_forms_still_obey_explicit_alpha() {
        for name in ["random@0x7f", "bikeshed@.5"] {
            assert_eq!(parse(name).unwrap()[3], 127);
        }
    }
}
