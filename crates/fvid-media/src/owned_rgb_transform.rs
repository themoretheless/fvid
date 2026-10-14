//! Packed RGB transforms preserve coded sample depth and channel order.
use crate::owned_frame::{GeometryFrame, buffer};
use fvid_media_info::{DecodeTransform, TransposeMode};
type Result<T> = std::result::Result<T, String>;

pub(crate) fn process(
    frame: &GeometryFrame,
    depth: u8,
    request: &DecodeTransform,
    n: u64,
    t: Option<f64>,
    curves: Option<&crate::owned_curves::Curves>,
) -> Result<Option<GeometryFrame>> {
    let mut remaining = request.clone();
    remaining.crop = None;
    remaining.horizontal_flip = false;
    remaining.vertical_flip = false;
    remaining.transpose = None;
    remaining.rotate = None;
    remaining.pad = None;
    remaining.scale = None;
    remaining.negate = None;
    remaining.curves = None;
    remaining.colorbalance = None;
    remaining.colorlevels = None;
    remaining.colorchannelmixer = None;
    if remaining != DecodeTransform::default() {
        return Ok(None);
    }
    if frame.subsampling.is_some() || !(8..=16).contains(&depth) {
        return Err("invalid RGB transform layout or depth".into());
    }
    let pixel = if depth == 8 { 3 } else { 6 };
    let storage = |w: usize, h: usize| {
        w.checked_mul(h)
            .and_then(|n| n.checked_mul(pixel))
            .ok_or_else(|| "RGB transform storage overflow".to_string())
    };
    if frame.width == 0
        || frame.height == 0
        || storage(frame.width, frame.height)? != frame.data.len()
    {
        return Err("invalid RGB transform storage".into());
    }
    if depth > 8
        && frame
            .data
            .chunks_exact(2)
            .any(|s| u32::from(u16::from_le_bytes([s[0], s[1]])) >= 1u32 << depth)
    {
        return Err("RGB transform sample exceeds coded depth".into());
    }
    let map = |source: &GeometryFrame,
               w: usize,
               h: usize,
               position: &dyn Fn(usize, usize) -> Option<(usize, usize)>|
     -> Result<GeometryFrame> {
        if w == 0 || h == 0 {
            return Err("empty RGB transform geometry".into());
        }
        let mut data = buffer(storage(w, h)?)?;
        for y in 0..h {
            for x in 0..w {
                if let Some((sx, sy)) = position(x, y) {
                    if sx >= source.width || sy >= source.height {
                        return Err("RGB transform coordinate outside source".into());
                    }
                    let from = (sy * source.width + sx) * pixel;
                    let to = (y * w + x) * pixel;
                    data[to..to + pixel].copy_from_slice(&source.data[from..from + pixel]);
                }
            }
        }
        Ok(GeometryFrame {
            width: w,
            height: h,
            subsampling: None,
            data,
        })
    };
    let mut current = if let Some(crop) = request.crop {
        if crop
            .x
            .checked_add(crop.width)
            .is_none_or(|n| n > frame.width)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|n| n > frame.height)
        {
            return Err("RGB crop exceeds source geometry".into());
        }
        map(frame, crop.width, crop.height, &|x, y| {
            Some((x + crop.x, y + crop.y))
        })?
    } else {
        map(frame, frame.width, frame.height, &|x, y| Some((x, y)))?
    };
    if request.horizontal_flip || request.vertical_flip {
        current = map(&current, current.width, current.height, &|x, y| {
            Some((
                if request.horizontal_flip {
                    current.width - 1 - x
                } else {
                    x
                },
                if request.vertical_flip {
                    current.height - 1 - y
                } else {
                    y
                },
            ))
        })?;
    }
    if let Some(mode) = request.transpose {
        current = map(&current, current.height, current.width, &|x, y| {
            Some(match mode {
                TransposeMode::Clock => (y, current.height - 1 - x),
                TransposeMode::CClock => (current.width - 1 - y, x),
                TransposeMode::ClockFlip => (current.width - 1 - y, current.height - 1 - x),
                TransposeMode::CClockFlip => (y, x),
            })
        })?;
    }
    if let Some(angle) = request.rotate {
        current = crate::owned_rotate::rotate(&current, angle.degrees, depth, true)?;
    }
    if let Some(pad) = request.pad {
        let (w, h, ox, oy) = (
            pad.width as usize,
            pad.height as usize,
            pad.x as usize,
            pad.y as usize,
        );
        if ox.checked_add(current.width).is_none_or(|n| n > w)
            || oy.checked_add(current.height).is_none_or(|n| n > h)
        {
            return Err("RGB pad does not contain source geometry".into());
        }
        current = map(&current, w, h, &|x, y| {
            (x >= ox && y >= oy && x - ox < current.width && y - oy < current.height)
                .then(|| (x - ox, y - oy))
        })?;
    }
    if let Some(scale) = request.scale {
        let (w, h) = (scale.width as usize, scale.height as usize);
        current = map(&current, w, h, &|x, y| {
            Some((
                (x as u128 * current.width as u128 / w as u128) as usize,
                (y as u128 * current.height as u128 / h as u128) as usize,
            ))
        })?;
    }
    if let Some(args) = request.negate.as_deref() {
        crate::owned_negate::Negate::parse(args)?.apply(&mut current.data, depth)?;
    }
    if let Some(args) = request.curves.as_deref() {
        let parsed;
        let filter = if let Some(filter) = curves {
            filter
        } else {
            parsed = crate::owned_curves::Curves::parse(args)?;
            &parsed
        };
        filter.apply_rgb_clock(
            &mut current.data,
            depth,
            3,
            current.width,
            current.height,
            n,
            t,
        )?;
    }
    if let Some(args) = request.colorbalance.as_deref() {
        crate::owned_colorbalance::ColorBalance::parse(args)?.apply_rgb(
            &mut current.data,
            depth,
            3,
        )?;
    }
    if let Some(args) = request.colorlevels.as_deref() {
        crate::owned_colorlevels::ColorLevels::parse(args)?.apply_rgb(
            &mut current.data,
            depth,
            3,
        )?;
    }
    if let Some(args) = request.colorchannelmixer.as_deref() {
        crate::owned_colorchannelmixer::ColorChannelMixer::parse(args)?.apply_rgb(
            &mut current.data,
            depth,
            3,
        )?;
    }
    Ok(Some(current))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(
        frame: &GeometryFrame,
        depth: u8,
        request: &DecodeTransform,
    ) -> Result<Option<GeometryFrame>> {
        super::process(frame, depth, request, 0, None, None)
    }
    #[test]
    fn all_transposes_keep_three_distinct_channels_at_every_depth() {
        for depth in [8, 10, 12, 16] {
            let maximum = (1u32 << depth) - 1;
            let mut data = Vec::new();
            for i in 0..6u32 {
                for value in [i, maximum - i, maximum / 2 + i] {
                    if depth == 8 {
                        data.push(value as u8);
                    } else {
                        data.extend_from_slice(&(value as u16).to_le_bytes());
                    }
                }
            }
            let source = GeometryFrame {
                width: 3,
                height: 2,
                subsampling: None,
                data,
            };
            for (mode, order) in [
                (TransposeMode::Clock, [3, 0, 4, 1, 5, 2]),
                (TransposeMode::CClock, [2, 5, 1, 4, 0, 3]),
                (TransposeMode::ClockFlip, [5, 2, 4, 1, 3, 0]),
                (TransposeMode::CClockFlip, [0, 3, 1, 4, 2, 5]),
            ] {
                let transformed = process(
                    &source,
                    depth,
                    &DecodeTransform {
                        transpose: Some(mode),
                        ..Default::default()
                    },
                )
                .unwrap()
                .unwrap();
                let pixel = if depth == 8 { 3 } else { 6 };
                let expected: Vec<_> = order
                    .into_iter()
                    .flat_map(|i| source.data[i * pixel..(i + 1) * pixel].iter().copied())
                    .collect();
                assert_eq!((transformed.width, transformed.height), (2, 3));
                assert_eq!(transformed.data, expected);
            }
        }
    }
    #[test]
    fn invalid_geometry_and_samples_fail_before_output() {
        let mut source = GeometryFrame {
            width: 1,
            height: 1,
            subsampling: None,
            data: vec![0; 6],
        };
        assert!(
            process(
                &source,
                10,
                &DecodeTransform {
                    scale: Some(fvid_media_info::ScaleSize {
                        width: 0,
                        height: 1
                    }),
                    ..Default::default()
                }
            )
            .is_err()
        );
        source.data[1] = 4;
        assert_eq!(
            process(&source, 10, &Default::default()).unwrap_err(),
            "RGB transform sample exceeds coded depth"
        );
        source.data.pop();
        assert_eq!(
            process(&source, 10, &Default::default()).unwrap_err(),
            "invalid RGB transform storage"
        );
    }
}
