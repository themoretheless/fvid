pub fn video_element(
    width: u32,
    height: u32,
    metadata: Option<&VideoMetadata>,
    rotation: u16,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(invalid("empty Matroska video dimensions"));
    }
    let mut data = [
        uint(0xb0, u64::from(width))?,
        uint(0xba, u64::from(height))?,
    ]
    .concat();
    if let Some(m) = metadata {
        let [left, top, right, bottom] = m.crop;
        let w = width
            .checked_sub(left)
            .and_then(|w| w.checked_sub(right))
            .filter(|w| *w > 0);
        let h = height
            .checked_sub(top)
            .and_then(|h| h.checked_sub(bottom))
            .filter(|h| *h > 0);
        let (Some(w), Some(h)) = (w, h) else {
            return Err(invalid("Matroska crop leaves no picture"));
        };
        let (num, den) = m.pixel_aspect;
        if num == 0 || den == 0 {
            return Err(invalid("zero Matroska pixel aspect ratio"));
        }
        for (id, value) in [
            (0x54cc, left),
            (0x54bb, top),
            (0x54dd, right),
            (0x54aa, bottom),
        ] {
            if value != 0 {
                data.extend(uint(id, u64::from(value))?);
            }
        }
        // DisplayUnit=3 states the exact display aspect, avoiding integer pixel
        // rounding for ratios such as 16:15 or 64:45.
        if num != den {
            data.extend(uint(0x54b2, 3)?);
            data.extend(uint(0x54b0, u64::from(w) * u64::from(num))?);
            data.extend(uint(0x54ba, u64::from(h) * u64::from(den))?);
        }
        let mut colour = Vec::new();
        if let Some(c) = m.colour {
            for (id, value) in [
                (0x55b1, c.matrix),
                (0x55ba, c.transfer),
                (0x55bb, c.primaries),
            ] {
                colour.extend(uint(id, u64::from(value))?);
            }
            colour.extend(uint(0x55b9, if c.full_range { 2 } else { 1 })?);
        }
        for (id, value) in [
            (0x55bc, m.hdr.light.max_cll),
            (0x55bd, m.hdr.light.max_fall),
        ] {
            if !value.is_finite()
                || value < 0.0
                || value.fract() != 0.0
                || f64::from(value) >= u64::MAX as f64
            {
                return Err(invalid(
                    "Matroska content light must be a nonnegative integer",
                ));
            }
            if value > 0.0 {
                colour.extend(uint(id, value as u64)?);
            }
        }
        if let Some(master) = m.hdr.mastering {
            let mut values = Vec::new();
            for (id, value) in [
                (0x55d1, master.red.x),
                (0x55d2, master.red.y),
                (0x55d3, master.green.x),
                (0x55d4, master.green.y),
                (0x55d5, master.blue.x),
                (0x55d6, master.blue.y),
                (0x55d7, master.white.x),
                (0x55d8, master.white.y),
            ] {
                if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                    return Err(invalid("invalid Matroska mastering chromaticity"));
                }
                values.extend(element(id, &value.to_be_bytes())?);
            }
            if master.min_luminance > master.max_luminance {
                return Err(invalid("invalid Matroska mastering luminance range"));
            }
            for (id, value) in [
                (0x55d9, master.max_luminance),
                (0x55da, master.min_luminance),
            ] {
                if !value.is_finite() || value < 0.0 {
                    return Err(invalid("invalid Matroska mastering luminance"));
                }
                values.extend(element(id, &f64::from(value).to_be_bytes())?);
            }
            colour.extend(element(0x55d0, &values)?);
        }
        if !colour.is_empty() {
            data.extend(element(0x55b0, &colour)?);
        }
    }
    let roll: f64 = match rotation {
        0 => 0.0,
        90 => -90.0,
        180 => 180.0,
        270 => 90.0,
        _ => return Err(invalid("Matroska rotation must be 0, 90, 180, or 270")),
    };
    if rotation != 0 {
        data.extend(element(
            0x7670,
            &[uint(0x7671, 0)?, element(0x7675, &roll.to_be_bytes())?].concat(),
        )?);
    }
    element(0xe0, &data)
}
