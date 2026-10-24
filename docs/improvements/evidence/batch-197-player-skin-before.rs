//! Player chrome and timeline skins. File skins use the same renderer as built-ins.
use eframe::egui::{Color32, Painter, Pos2, Rect, Stroke};

#[derive(Clone, Debug)]
pub(super) struct Skin {
    pub name: String,
    pub diagram: bool,
    pub text: Color32,
    pub played: Color32,
    pub buffer: Color32,
    pub swap: Color32,
    pub track: Color32,
    pub panel: Color32,
}
impl Default for Skin {
    fn default() -> Self {
        Self::classic()
    }
}
impl Skin {
    pub fn classic() -> Self {
        Self {
            name: "Classic".into(),
            diagram: false,
            text: Color32::from_rgb(238, 235, 228),
            played: Color32::from_rgb(242, 107, 29),
            buffer: Color32::from_rgb(65, 205, 210),
            swap: Color32::from_rgb(99, 91, 181),
            track: Color32::from_rgba_premultiplied(43, 42, 41, 46),
            panel: Color32::from_rgba_premultiplied(6, 6, 6, 128),
        }
    }
    pub fn diagram() -> Self {
        Self {
            name: "Diagram".into(),
            diagram: true,
            text: Color32::WHITE,
            played: Color32::WHITE,
            buffer: Color32::WHITE,
            swap: Color32::WHITE,
            track: Color32::from_gray(170),
            panel: Color32::from_gray(40),
        }
    }
    pub fn load(value: &str) -> crate::Result<Self> {
        match value {
            "classic" => return Ok(Self::classic()),
            "diagram" => return Ok(Self::diagram()),
            _ => {}
        }
        let data = std::fs::read_to_string(value)?;
        let json: serde_json::Value = serde_json::from_str(&data)
            .map_err(|e| crate::invalid(&format!("invalid skin: {e}")))?;
        let object = json
            .as_object()
            .ok_or_else(|| crate::invalid("skin must be a JSON object"))?;
        let mut skin = match object
            .get("style")
            .and_then(|v| v.as_str())
            .unwrap_or("diagram")
        {
            "classic" => Self::classic(),
            "diagram" => Self::diagram(),
            _ => return Err(crate::invalid("skin style must be classic or diagram")),
        };
        for (key, value) in object {
            if key == "style" {
                continue;
            }
            if key == "name" {
                skin.name = value
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| crate::invalid("skin name must be nonempty text"))?
                    .to_owned();
                continue;
            }
            let target = match key.as_str() {
                "text" => &mut skin.text,
                "played" => &mut skin.played,
                "buffer" => &mut skin.buffer,
                "swap" => &mut skin.swap,
                "track" => &mut skin.track,
                "panel" => &mut skin.panel,
                _ => return Err(crate::invalid(&format!("unknown skin field {key}"))),
            };
            let hex = value
                .as_str()
                .and_then(|s| s.strip_prefix('#'))
                .filter(|s| s.len() == 6 && s.is_ascii())
                .ok_or_else(|| crate::invalid("skin colours must be #RRGGBB"))?;
            let rgb =
                u32::from_str_radix(hex, 16).map_err(|_| crate::invalid("invalid skin colour"))?;
            *target = Color32::from_rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        }
        Ok(skin)
    }
}

/// Patterns use screen-space cells, so resizing never changes their density.
pub(super) fn pattern(painter: &Painter, rect: Rect, color: Color32, checker: bool) {
    let painter = painter.with_clip_rect(rect.intersect(painter.clip_rect()));
    let step = if checker { 4.0 } else { 6.0 };
    let mut y = rect.top();
    let mut row = 0;
    while y < rect.bottom() {
        let mut x = rect.left();
        let mut col = 0;
        while x < rect.right() {
            if checker {
                if (row + col) % 2 == 0 {
                    painter.rect_filled(
                        Rect::from_min_size(Pos2::new(x, y), eframe::egui::Vec2::splat(step)),
                        0.0,
                        color,
                    );
                }
            } else {
                painter.circle_filled(Pos2::new(x + 2.0, y + 2.0), 1.0, color);
            }
            x += step;
            col += 1;
        }
        y += step;
        row += 1;
    }
    painter.line_segment(
        [rect.left_top(), rect.left_bottom()],
        Stroke::new(1.0, color),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builtins_and_file_validation() {
        assert!(Skin::load("diagram").unwrap().diagram);
        assert!(!Skin::load("classic").unwrap().diagram);
        let path = std::env::temp_dir().join(format!("fvid-skin-{}.json", std::process::id()));
        std::fs::write(
            &path,
            r##"{"name":"Mint","style":"diagram","played":"#00ffaa"}"##,
        )
        .unwrap();
        let skin = Skin::load(path.to_str().unwrap()).unwrap();
        assert_eq!(skin.played, Color32::from_rgb(0, 255, 170));
        std::fs::write(&path, r##"{"played":"#xxffaa"}"##).unwrap();
        assert!(Skin::load(path.to_str().unwrap()).is_err());
        std::fs::write(&path, r##"{"typo":true}"##).unwrap();
        assert!(Skin::load(path.to_str().unwrap()).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
