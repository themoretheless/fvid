//! Owned RGB tone curves with natural and monotone cubic interpolation.
use std::{cell::RefCell, io::Read, path::PathBuf};
type Result<T> = std::result::Result<T, String>;
type Points = Vec<(f64, f64)>;
#[derive(Debug)]
pub struct Curves {
    points: [Points; 4],
    pchip: bool,
    clocked: bool,
    plot: Option<PathBuf>,
    enable: crate::owned_timeline::Timeline,
    cache: RefCell<Option<Tables>>,
}
#[derive(Debug)]
struct Tables {
    depth: u8,
    graphs: [Vec<u16>; 4],
    plotted: bool,
}
impl Curves {
    pub fn parse(args: &str) -> Result<Self> {
        if args.len() > 65536 || args.contains('\0') {
            return Err("invalid curves options".into());
        }
        let names = [
            "preset", "master", "red", "green", "blue", "all", "psfile", "plot", "interp",
        ];
        let mut selected: [Option<Points>; 4] = Default::default();
        let mut all = None;
        let mut preset = "none";
        let mut psfile = None;
        let mut plot = None;
        let mut pchip = false;
        let mut position = 0;
        let mut clocked = false;
        let mut enable = crate::owned_timeline::Timeline::default();
        for option in args.split(':').filter(|_| !args.is_empty()) {
            let (key, value) = if let Some(pair) = option.split_once('=') {
                pair
            } else {
                let key = *names.get(position).ok_or("too many curves options")?;
                position += 1;
                (key, option)
            };
            let value = unquote(value.trim())?;
            match key.trim() {
                "preset" => {
                    preset = value;
                }
                "master" | "m" => selected[3] = Some(parse_points(value)?),
                "red" | "r" => selected[0] = Some(parse_points(value)?),
                "green" | "g" => selected[1] = Some(parse_points(value)?),
                "blue" | "b" => selected[2] = Some(parse_points(value)?),
                "all" => all = Some(parse_points(value)?),
                "psfile" => psfile = Some(PathBuf::from(value)),
                "plot" => plot = Some(PathBuf::from(value)),
                "interp" => {
                    pchip = match value {
                        "natural" | "0" => false,
                        "pchip" | "1" => true,
                        _ => return Err("invalid curves interpolation".into()),
                    }
                }
                "enable" => {
                    clocked = true;
                    enable = crate::owned_timeline::Timeline::grayworld(&format!("enable={value}"))?
                }
                _ => return Err("unknown curves option".into()),
            }
        }
        if let Some(all) = all {
            for channel in &mut selected[..3] {
                if channel.is_none() {
                    *channel = Some(all.clone());
                }
            }
        }
        if let Some(path) = psfile {
            let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut bytes = Vec::new();
            file.take(1_048_577)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if bytes.len() > 1_048_576 {
                return Err("ACV file is too large".into());
            }
            let points = parse_acv(&bytes)?;
            for (channel, points) in selected.iter_mut().zip(points) {
                if channel.is_none() && !points.is_empty() {
                    *channel = Some(points);
                }
            }
        }
        let defaults = preset_points(preset)?;
        for (channel, text) in selected.iter_mut().zip(defaults) {
            if channel.is_none() {
                *channel = Some(parse_points(text)?);
            }
        }
        Ok(Self {
            points: selected.map(Option::unwrap),
            pchip,
            clocked,
            plot,
            enable,
            cache: RefCell::new(None),
        })
    }
    pub fn apply_yuv(
        &self,
        frame: &mut crate::owned_frame::GeometryFrame,
        depth: u8,
        full: bool,
        matrix: crate::owned_yuv_rgb::Matrix,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        crate::owned_overlay::validate_frame(frame, depth)?;
        if !self.enable.enabled(n, t, frame.width, frame.height)? {
            return Ok(());
        }
        crate::owned_yuv_rgb::filter_rgb16_sampled(
            frame,
            depth,
            full,
            matrix,
            crate::owned_yuv_rgb::ChromaSampling::Point,
            |rgb| self.apply_samples(rgb, 16, 3),
        )
    }
    /// Interleaved RGB/RGBA; little-endian samples above eight bits, alpha unchanged.
    pub fn apply_rgb(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if self.clocked {
            return Err("curves timeline requires an explicit frame clock".into());
        }
        self.apply_samples(data, depth, channels)
    }
    pub fn apply_rgb_clock(
        &self,
        data: &mut [u8],
        depth: u8,
        channels: usize,
        width: usize,
        height: usize,
        n: u64,
        t: Option<f64>,
    ) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid curves RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        if width == 0
            || height == 0
            || width
                .checked_mul(height)
                .and_then(|v| v.checked_mul(channels * bytes))
                != Some(data.len())
            || bytes == 2
                && data
                    .chunks_exact(2)
                    .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) >= (1u32 << depth))
        {
            return Err("invalid curves clocked sample storage".into());
        }
        if self.enable.enabled(n, t, width, height)? {
            self.apply_samples(data, depth, channels)
        } else {
            Ok(())
        }
    }
    fn apply_samples(&self, data: &mut [u8], depth: u8, channels: usize) -> Result<()> {
        if !(8..=16).contains(&depth) || !matches!(channels, 3 | 4) {
            return Err("invalid curves RGB format".into());
        }
        let bytes = if depth == 8 { 1 } else { 2 };
        let maximum = (1u32 << depth) - 1;
        if data.len() % (channels * bytes) != 0
            || bytes == 2
                && data
                    .chunks_exact(2)
                    .any(|p| u32::from(u16::from_le_bytes([p[0], p[1]])) > maximum)
        {
            return Err("invalid curves sample storage".into());
        }
        let mut cache = self
            .cache
            .try_borrow_mut()
            .map_err(|_| "curves tables already borrowed")?;
        if cache.as_ref().is_none_or(|v| v.depth != depth) {
            let mut graphs: [Vec<u16>; 4] = Default::default();
            for (graph, points) in graphs.iter_mut().zip(&self.points) {
                *graph = table(points, depth, self.pchip)?;
            }
            // Compose the master lookup after the channel lookup.
            for channel in 0..3 {
                for index in 0..graphs[channel].len() {
                    graphs[channel][index] = graphs[3][usize::from(graphs[channel][index])];
                }
            }
            *cache = Some(Tables {
                depth,
                graphs,
                plotted: false,
            });
        }
        let cache = cache.as_mut().unwrap();
        if !cache.plotted {
            if let Some(path) = &self.plot {
                write_plot(path, &cache.graphs, &self.points)?;
            }
            cache.plotted = true;
        }
        for pixel in data.chunks_exact_mut(channels * bytes) {
            for channel in 0..3 {
                let index = if bytes == 1 {
                    usize::from(pixel[channel])
                } else {
                    usize::from(u16::from_le_bytes([
                        pixel[2 * channel],
                        pixel[2 * channel + 1],
                    ]))
                };
                let value = cache.graphs[channel][index];
                if bytes == 1 {
                    pixel[channel] = value as u8;
                } else {
                    pixel[2 * channel..2 * channel + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
        }
        Ok(())
    }
}
fn unquote(value: &str) -> Result<&str> {
    if value.starts_with('\'') || value.starts_with('"') {
        let quote = value.chars().next().unwrap();
        value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
            .ok_or("unclosed curves quote".into())
    } else {
        Ok(value)
    }
}
fn parse_points(text: &str) -> Result<Points> {
    let mut points = Vec::new();
    for pair in text.split_whitespace() {
        if points.len() == 4096 {
            return Err("too many curves points".into());
        }
        let (x, y) = pair.split_once('/').ok_or("curves points require x/y")?;
        let x = x.parse::<f64>().map_err(|_| "invalid curves coordinate")?;
        let y = y.parse::<f64>().map_err(|_| "invalid curves coordinate")?;
        if !x.is_finite()
            || !y.is_finite()
            || !(0.0..=1.0).contains(&x)
            || !(0.0..=1.0).contains(&y)
            || points.last().is_some_and(|&(previous, _)| x <= previous)
        {
            return Err("curves points must be finite, in range and increasing".into());
        }
        points
            .try_reserve(1)
            .map_err(|_| "curves point allocation failed")?;
        points.push((x, y));
    }
    Ok(points)
}
pub fn parse_acv(bytes: &[u8]) -> Result<[Points; 4]> {
    let mut position = 0;
    let mut word = || -> Result<u16> {
        let pair = bytes
            .get(position..position + 2)
            .ok_or("truncated ACV file")?;
        position += 2;
        Ok(u16::from_be_bytes([pair[0], pair[1]]))
    };
    let version = word()?;
    if !matches!(version, 1 | 4) {
        return Err("unsupported ACV version".into());
    }
    let count = word()?;
    let mut result: [Points; 4] = Default::default();
    for channel in [3, 0, 1, 2].into_iter().take(usize::from(count)) {
        let size = usize::from(word()?);
        if size > 4096 {
            return Err("too many ACV points".into());
        }
        let mut text = String::new();
        for _ in 0..size {
            let y = word()?;
            let x = word()?;
            if x > 255 || y > 255 {
                return Err("ACV coordinate exceeds 255".into());
            }
            text.push_str(&format!(
                "{:.6}/{:.6} ",
                f64::from(x) / 255.,
                f64::from(y) / 255.
            ));
        }
        result[channel] = parse_points(&text)?;
    }
    Ok(result)
}
fn zeros<T: Default + Clone>(size: usize) -> Result<Vec<T>> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(size)
        .map_err(|_| "curves allocation failed")?;
    values.resize(size, T::default());
    Ok(values)
}
fn table(points: &[(f64, f64)], depth: u8, pchip: bool) -> Result<Vec<u16>> {
    let size = 1usize << depth;
    let scale = (size - 1) as f64;
    let n = points.len();
    let mut lut = zeros::<u16>(size)?;
    if n == 0 {
        for (index, value) in lut.iter_mut().enumerate() {
            *value = index as u16;
        }
        return Ok(lut);
    }
    let clip = |v: f64| v.clamp(0., scale) as u16;
    if n == 1 {
        lut.fill(clip(points[0].1 * scale));
        return Ok(lut);
    }
    if points
        .windows(2)
        .any(|v| (v[0].0 * scale) as usize >= (v[1].0 * scale) as usize)
    {
        return Err("curves points too close for sample depth".into());
    }
    let mut h = zeros::<f64>(n - 1)?;
    let mut slope = zeros::<f64>(n - 1)?;
    let mut deriv = zeros::<f64>(n)?;
    for i in 0..n - 1 {
        h[i] = points[i + 1].0 - points[i].0;
        slope[i] = (points[i + 1].1 - points[i].1) / h[i];
    }
    if pchip {
        // Work in sample units, as used by the LUT's Hermite interpolation.
        for i in 0..n - 1 {
            h[i] = (points[i + 1].0 * scale) - (points[i].0 * scale);
            slope[i] = (points[i + 1].1 * scale - points[i].1 * scale) / h[i];
        }
        if n == 2 {
            let m = slope[0];
            let b = points[0].1 * scale - points[0].0 * scale * m;
            for (i, v) in lut.iter_mut().enumerate() {
                *v = clip(i as f64 * m + b);
            }
            return Ok(lut);
        }
        for i in 1..n - 1 {
            let a = slope[i - 1];
            let b = slope[i];
            if a != 0. && b != 0. && a.signum() == b.signum() {
                let w1 = 2. * h[i] + h[i - 1];
                let w2 = h[i] + 2. * h[i - 1];
                deriv[i] = (w1 + w2) / (w1 / a + w2 / b);
            }
        }
        deriv[0] = edge(h[0], h[1], slope[0], slope[1]);
        deriv[n - 1] = edge(h[n - 2], h[n - 3], slope[n - 2], slope[n - 3]);
        let mut x = 0;
        while (x as f64) < points[0].0 * scale {
            lut[x] = clip(points[0].1 * scale);
            x += 1;
        }
        for i in 0..n - 1 {
            while (x as f64) < points[i + 1].0 * scale {
                let t = (x as f64 - points[i].0 * scale) / h[i];
                lut[x] = clip(
                    hermite(1. - t, points[i].1 * scale, -h[i] * deriv[i])
                        + hermite(t, points[i + 1].1 * scale, h[i] * deriv[i + 1]),
                );
                x += 1;
            }
        }
        lut[x..].fill(clip(points[n - 1].1 * scale));
    } else {
        let mut upper = zeros::<f64>(n)?;
        for i in 1..n - 1 {
            let k = 1. / (2. * (h[i - 1] + h[i]) - h[i - 1] * upper[i - 1]);
            upper[i] = h[i] * k;
            deriv[i] = (6. * (slope[i] - slope[i - 1]) - h[i - 1] * deriv[i - 1]) * k;
        }
        for i in (0..n - 1).rev() {
            deriv[i] -= upper[i] * deriv[i + 1];
        }
        lut[..(points[0].0 * scale) as usize].fill(clip(points[0].1 * scale));
        for i in 0..n - 1 {
            let b = slope[i] - h[i] * deriv[i] / 2. - h[i] * (deriv[i + 1] - deriv[i]) / 6.;
            let c = deriv[i] / 2.;
            let d = (deriv[i + 1] - deriv[i]) / (6. * h[i]);
            let start = (points[i].0 * scale) as usize;
            let end = (points[i + 1].0 * scale) as usize;
            for x in start..=end {
                let t = (x - start) as f64 / scale;
                lut[x] = clip((points[i].1 + b * t + c * t * t + d * t * t * t) * scale);
            }
        }
        lut[(points[n - 1].0 * scale) as usize..].fill(clip(points[n - 1].1 * scale));
    }
    Ok(lut)
}
fn edge(h0: f64, h1: f64, m0: f64, m1: f64) -> f64 {
    let d = ((2. * h0 + h1) * m0 - h0 * m1) / (h0 + h1);
    if sign(d) != sign(m0) {
        0.
    } else if sign(m0) != sign(m1) && d.abs() > 3. * m0.abs() {
        3. * m0
    } else {
        d
    }
}
fn hermite(x: f64, value: f64, derivative: f64) -> f64 {
    let x2 = x * x;
    let x3 = x2 * x;
    value.mul_add(3f64.mul_add(x2, -2. * x3), derivative * (x3 - x2))
}
fn write_plot(path: &std::path::Path, graphs: &[Vec<u16>; 4], points: &[Points; 4]) -> Result<()> {
    use std::io::Write;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| e.to_string())?);
    writeln!(out,"set size square\nset grid\nplot '-' using 1:2 with lines title 'red', '-' using 1:2 with lines title 'green', '-' using 1:2 with lines title 'blue', '-' using 1:2 with lines title 'master'").map_err(|e|e.to_string())?;
    let scale = (graphs[0].len() - 1) as f64;
    for graph in graphs {
        for (i, &v) in graph.iter().enumerate() {
            writeln!(out, "{:.6} {:.6}", i as f64 / scale, f64::from(v) / scale)
                .map_err(|e| e.to_string())?;
        }
        writeln!(out, "e").map_err(|e| e.to_string())?;
    }
    for (channel, points) in points.iter().enumerate() {
        for &(x, y) in points {
            writeln!(out, "# knot {channel} {x} {y}").map_err(|e| e.to_string())?;
        }
    }
    out.flush().map_err(|e| e.to_string())
}
fn preset_points(name: &str) -> Result<[&'static str; 4]> {
    Ok(match name {
        "none" | "0" => [""; 4],
        "color_negative" | "1" => [
            "0.129/1 0.466/0.498 0.725/0",
            "0.109/1 0.301/0.498 0.517/0",
            "0.098/1 0.235/0.498 0.423/0",
            "",
        ],
        "cross_process" | "2" => [
            "0/0 0.25/0.156 0.501/0.501 0.686/0.745 1/1",
            "0/0 0.25/0.188 0.38/0.501 0.745/0.815 1/0.815",
            "0/0 0.231/0.094 0.709/0.874 1/1",
            "",
        ],
        "darker" | "3" => ["", "", "", "0/0 0.5/0.4 1/1"],
        "increase_contrast" | "4" => ["", "", "", "0/0 0.149/0.066 0.831/0.905 0.905/0.98 1/1"],
        "lighter" | "5" => ["", "", "", "0/0 0.4/0.5 1/1"],
        "linear_contrast" | "6" => ["", "", "", "0/0 0.305/0.286 0.694/0.713 1/1"],
        "medium_contrast" | "7" => ["", "", "", "0/0 0.286/0.219 0.639/0.643 1/1"],
        "negative" | "8" => ["", "", "", "0/1 1/0"],
        "strong_contrast" | "9" => ["", "", "", "0/0 0.301/0.196 0.592/0.6 0.686/0.737 1/1"],
        "vintage" | "10" => [
            "0/0.11 0.42/0.51 1/0.95",
            "0/0 0.50/0.48 1/1",
            "0/0.22 0.49/0.44 1/0.8",
            "",
        ],
        _ => return Err("unknown curves preset".into()),
    })
}

fn sign(x: f64) -> i8 {
    if x > 0. {
        1
    } else if x < 0. {
        -1
    } else {
        0
    }
}
