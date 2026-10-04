//! Owned box/grid regions, geometric expressions and ordered sample compositing.
use crate::owned_frame::GeometryFrame;
type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Box,
    Grid,
}
#[derive(Debug)]
pub struct Draw {
    kind: Kind,
    expressions: [crate::owned_expression::Expression; 5],
    rgba: [u8; 4],
    invert: bool,
    replace: bool,
    enable: crate::owned_timeline::Timeline,
}
impl Draw {
    pub fn box_filter(args: &str) -> Result<Self> {
        Self::parse(Kind::Box, args)
    }
    pub fn grid_filter(args: &str) -> Result<Self> {
        Self::parse(Kind::Grid, args)
    }
    pub fn parse(kind: Kind, args: &str) -> Result<Self> {
        if args.len() > 4096 || args.contains('\0') {
            return Err("invalid drawing options".into());
        }
        let mut values = [
            "0".to_string(),
            "0".into(),
            "0".into(),
            "0".into(),
            if matches!(kind, Kind::Box) {
                "3".into()
            } else {
                "1".into()
            },
        ];
        let mut rgba = [0, 0, 0, 255];
        let mut invert = false;
        let mut replace = false;
        let mut enable = Default::default();
        let mut position = 0;
        for entry in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = entry.split_once('=') {
                pair
            } else {
                let key = *["x", "y", "w", "h", "color", "t", "replace"]
                    .get(position)
                    .ok_or("too many drawing options")?;
                position += 1;
                (key, entry)
            };
            let value = value.trim().trim_matches('\'');
            match key.trim() {
                "x" => values[0] = value.into(),
                "y" => values[1] = value.into(),
                "w" | "width" => values[2] = value.into(),
                "h" | "height" => values[3] = value.into(),
                "t" | "thickness" => values[4] = value.into(),
                "c" | "color" => {
                    invert = value == "invert";
                    if !invert {
                        rgba = crate::owned_rgba::parse(value)?;
                    }
                }
                "replace" => {
                    replace = match value {
                        "1" | "true" | "yes" | "on" => true,
                        "0" | "false" | "no" | "off" => false,
                        _ => return Err("invalid drawing replace flag".into()),
                    }
                }
                "enable" => {
                    enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?
                }
                "box_source" => {
                    return Err("drawing detection-box metadata is not yet owned".into());
                }
                _ => return Err("unknown drawing option".into()),
            }
        }
        let expressions = values
            .map(|v| crate::owned_expression::Expression::parse(&v))
            .into_iter()
            .collect::<Result<Vec<_>>>()?
            .try_into()
            .map_err(|_| "invalid drawing expression count")?;
        let out = Self {
            kind,
            expressions,
            rgba,
            invert,
            replace,
            enable,
        };
        let vars = Self::variables(16, 12, [2, 2], 1.);
        for expr in &out.expressions {
            expr.evaluate(&vars)?;
        }
        Ok(out)
    }
    fn variables(w: usize, h: usize, sub: [usize; 2], sar: f64) -> [(&'static str, f64); 14] {
        [
            ("dar", w as f64 / h as f64 * sar),
            ("hsub", sub[0].ilog2() as f64),
            ("vsub", sub[1].ilog2() as f64),
            ("in_h", h as f64),
            ("ih", h as f64),
            ("in_w", w as f64),
            ("iw", w as f64),
            ("sar", sar),
            ("x", f64::NAN),
            ("y", f64::NAN),
            ("w", f64::NAN),
            ("h", f64::NAN),
            ("t", f64::NAN),
            ("fill", f64::NAN),
        ]
    }
    fn region(&self, w: usize, h: usize, sub: [usize; 2], sar: f64) -> Result<[i64; 5]> {
        if !sar.is_finite() || sar <= 0. || w > i32::MAX as usize || h > i32::MAX as usize {
            return Err("invalid drawing geometry/aspect".into());
        }
        let mut vars = Self::variables(w, h, sub, sar);
        let mut result = [0; 5];
        for _ in 0..6 {
            for i in 0..5 {
                vars[13].1 = match i {
                    0 => w as f64,
                    1 => h as f64,
                    2 => w as f64 - result[0] as f64,
                    3 => h as f64 - result[1] as f64,
                    _ => i32::MAX as f64,
                };
                let value = self.expressions[i].evaluate(&vars)?;
                vars[8 + i].1 = value;
                // Early passes may depend on a size expression evaluated later in this pass.
                result[i] =
                    if value.is_finite() && value >= i32::MIN as f64 && value <= i32::MAX as f64 {
                        value as i64
                    } else {
                        i32::MIN as i64
                    };
            }
        }
        for i in 0..5 {
            let v = vars[8 + i].1;
            if !v.is_finite() || v < i32::MIN as f64 || v > i32::MAX as f64 {
                return Err(
                    "drawing expression is nonfinite or outside integer coordinates".into(),
                );
            }
        }
        if result[2] <= 0 {
            result[2] = w as i64;
        }
        if result[3] <= 0 {
            result[3] = h as i64;
        }
        Ok(result)
    }
    fn selected(&self, r: [i64; 5], x: usize, y: usize) -> bool {
        let [left, top, w, h, t] = r;
        let x = x as i64;
        let y = y as i64;
        match self.kind {
            Kind::Box => {
                x >= left
                    && y >= top
                    && x < left + w
                    && y < top + h
                    && (x - left < t || left + w - 1 - x < t || y - top < t || top + h - 1 - y < t)
            }
            Kind::Grid => (x - left).rem_euclid(w) < t || (y - top).rem_euclid(h) < t,
        }
    }
    pub fn apply(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        self.apply_with_aspect(frame, depth, 1., n, t)
    }
    pub fn apply_with_aspect(
        &self,
        frame: &mut GeometryFrame,
        depth: u8,
        sar: f64,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        let sub = frame.subsampling.unwrap_or([1, 1]);
        let region = self.region(frame.width, frame.height, sub, sar)?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        if frame.subsampling.is_some() {
            self.draw_planar(
                &mut frame.data,
                frame.width,
                frame.height,
                sub,
                depth,
                false,
                region,
            );
        } else {
            self.draw_rgb(&mut frame.data, frame.width, frame.height, depth, 3, region);
        }
        Ok(())
    }
    pub fn apply_yuva(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        sub: [usize; 2],
        depth: u8,
        sar: f64,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if sub.iter().any(|v| !matches!(v, 1 | 2 | 4))
            || w == 0
            || h == 0
            || !(8..=16).contains(&depth)
        {
            return Err("invalid drawing YUVA layout".into());
        }
        let size = w
            .checked_mul(h)
            .and_then(|v| v.checked_mul(2))
            .and_then(|v| {
                w.div_ceil(sub[0])
                    .checked_mul(h.div_ceil(sub[1]))
                    .and_then(|c| c.checked_mul(2))
                    .and_then(|c| c.checked_add(v))
            })
            .and_then(|v| v.checked_mul(bytes));
        if size != Some(data.len())
            || depth > 8
                && data
                    .chunks_exact(2)
                    .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("invalid drawing YUVA storage/precision".into());
        }
        let region = self.region(w, h, sub, sar)?;
        if self.enable.enabled(n, t, w, h)? {
            self.draw_planar(data, w, h, sub, depth, true, region);
        }
        Ok(())
    }
    pub fn apply_rgba(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        self.apply_rgb(data, w, h, 8, 4, 1., n, t)
    }
    /// Canonical RGB/RGBA samples, including precise 9..16-bit standalone surfaces.
    pub fn apply_rgb(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        channels: usize,
        sar: f64,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        let bytes = if depth == 8 { 1 } else { 2 };
        if w == 0
            || h == 0
            || !matches!(channels, 3 | 4)
            || !(8..=16).contains(&depth)
            || w.checked_mul(h)
                .and_then(|v| v.checked_mul(channels))
                .and_then(|v| v.checked_mul(bytes))
                != Some(data.len())
            || depth > 8
                && data
                    .chunks_exact(2)
                    .any(|v| u16::from_le_bytes([v[0], v[1]]) as u32 >= 1u32 << depth)
        {
            return Err("invalid drawing RGB/RGBA storage or precision".into());
        }
        let region = self.region(w, h, [1, 1], sar)?;
        if self.enable.enabled(n, t, w, h)? {
            self.draw_rgb(data, w, h, depth, channels, region);
        }
        Ok(())
    }
    fn yuv(&self) -> [u16; 4] {
        let [r, g, b, a] = self.rgba.map(i64::from);
        let fix = |v: f64| (v * 1024. + 0.5) as i64;
        [
            ((fix(0.299 * 219. / 255.) * r
                + fix(0.587 * 219. / 255.) * g
                + fix(0.114 * 219. / 255.) * b
                + 512
                + (16 << 10))
                >> 10) as u16,
            (((-fix(0.16874 * 224. / 255.) * r - fix(0.33126 * 224. / 255.) * g
                + fix(0.5 * 224. / 255.) * b
                + 511)
                >> 10)
                + 128) as u16,
            (((fix(0.5 * 224. / 255.) * r
                - fix(0.41869 * 224. / 255.) * g
                - fix(0.08131 * 224. / 255.) * b
                + 511)
                >> 10)
                + 128) as u16,
            a as u16,
        ]
    }
    fn draw_planar(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        sub: [usize; 2],
        depth: u8,
        alpha: bool,
        r: [i64; 5],
    ) {
        let bytes = if depth == 8 { 1 } else { 2 };
        let cw = w.div_ceil(sub[0]);
        let ch = h.div_ceil(sub[1]);
        let offsets = [
            0,
            w * h * bytes,
            (w * h + cw * ch) * bytes,
            (w * h + 2 * cw * ch) * bytes,
        ];
        let mut color = self.yuv().map(|v| v << (depth - 8));
        let max = (1u32 << depth) - 1;
        color[3] = ((self.rgba[3] as u32 * max + 127) / 255) as u16;
        let blend = self.rgba[3] as f64 / 255.;
        for y in 0..h {
            for x in 0..w {
                if !self.selected(r, x, y) {
                    continue;
                }
                for p in 0..if alpha && self.replace { 4 } else { 3 } {
                    let at = offsets[p]
                        + if p == 1 || p == 2 {
                            ((y / sub[1]) * cw + x / sub[0]) * bytes
                        } else {
                            (y * w + x) * bytes
                        };
                    let old = if bytes == 1 {
                        data[at] as u16
                    } else {
                        u16::from_le_bytes([data[at], data[at + 1]])
                    };
                    let value = if self.invert {
                        if p != 0 {
                            continue;
                        }
                        max as u16 - old
                    } else if alpha && self.replace {
                        color[p]
                    } else {
                        ((1. - blend) * old as f64 + blend * color[p] as f64) as u16
                    };
                    if bytes == 1 {
                        data[at] = value as u8;
                    } else {
                        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
    }
    fn draw_rgb(
        &self,
        data: &mut [u8],
        w: usize,
        h: usize,
        depth: u8,
        channels: usize,
        r: [i64; 5],
    ) {
        let bytes = if depth == 8 { 1 } else { 2 };
        let max = (1u32 << depth) - 1;
        let alpha = self.rgba[3] as f32 / 255.;
        for y in 0..h {
            for x in 0..w {
                if !self.selected(r, x, y) {
                    continue;
                }
                for p in 0..if channels == 4 && self.replace && !self.invert {
                    4
                } else {
                    3
                } {
                    let at = ((y * w + x) * channels + p) * bytes;
                    let old = if bytes == 1 {
                        data[at] as u16
                    } else {
                        u16::from_le_bytes([data[at], data[at + 1]])
                    };
                    let color = ((self.rgba[p] as u32 * max + 127) / 255) as u16;
                    let value = if self.invert {
                        max as u16 - old
                    } else if channels == 4 && self.replace {
                        color
                    } else {
                        ((1. - alpha) * old as f32 + alpha * color as f32) as u16
                    };
                    if bytes == 1 {
                        data[at] = value as u8;
                    } else {
                        data[at..at + 2].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
        }
    }
}
