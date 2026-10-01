#[test]
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
#[ignore = "requires physical Metal and VideoToolbox"]
fn odd_native_crop_matches_independent_float_reference() {
    use crate::playback_native::{NativeReader, RawFrame, surface_to_packed};
    let (device, queue) = headless_features(
        wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
            | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
    )
    .unwrap();
    let shader = ColorShader::new(
        "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
    )
    .unwrap();
    for bytes in [
        include_bytes!("../tests/fixtures/display/par-2x1.mp4").as_slice(),
        include_bytes!("../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
    ] {
        let mut reader = NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
        assert!(reader.enable_shared_surfaces().unwrap());
        let RawFrame::Surface { surface, colour } = reader.read_frame_raw().unwrap().unwrap()
        else {
            panic!("native surface required")
        };
        let source = surface_to_packed(&surface, colour).unwrap();
        let depth = source.depth;
        let scale = f64::from(1u32 << (depth - 8));
        let mut renderer =
            MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722).unwrap();
        for rotation in [0, 90, 180, 270] {
            let rotated = source.rotated(rotation).unwrap();
            let sw = rotated.frame.width;
            let sh = rotated.frame.height;
            let raw = rotated.plane_data().unwrap();
            let planes: Vec<Vec<f64>> = raw
                .iter()
                .map(|p| {
                    if depth == 8 {
                        p.iter().map(|&v| f64::from(v)).collect()
                    } else {
                        p.chunks_exact(2)
                            .map(|v| f64::from(u16::from_le_bytes(v.try_into().unwrap())) / scale)
                            .collect()
                    }
                })
                .collect();
            for [x, y, w, h] in [[1, 1, sw - 3, sh - 3], [1, 2, sw - 4, sh - 4]] {
                for [ow, oh] in [[w, h], [w / 2 + 1, h / 2 + 1], [w * 2 + 1, h * 2 + 1]] {
                    renderer
                        .set_window([
                            x as f32 / sw as f32,
                            y as f32 / sh as f32,
                            (x + w) as f32 / sw as f32,
                            (y + h) as f32 / sh as f32,
                        ])
                        .unwrap();
                    let actual = renderer
                        .render_surface_sized(
                            &device,
                            &queue,
                            &surface,
                            colour,
                            0,
                            None,
                            rotation,
                            [ow, oh],
                        )
                        .unwrap()
                        .download()
                        .unwrap();
                    // Independent CPU bilinear interpolation in upright component
                    // coordinates: keep fractional chroma phase at odd crop edges.
                    let sample = |plane: usize, ox: usize, oy: usize| {
                        let divisor = if plane == 0 { 1.0 } else { 2.0 };
                        let pw = if plane == 0 { sw } else { sw / 2 };
                        let ph = if plane == 0 { sh } else { sh / 2 };
                        let at = |origin: usize, length: usize, out: usize, index: usize| {
                            let low = origin as f64 / divisor;
                            let high = ((origin + length) as f64 / divisor - 1.0).max(low);
                            ((origin as f64 + (index as f64 + 0.5) * length as f64 / out as f64)
                                / divisor
                                - 0.5)
                                .clamp(low, high)
                        };
                        let sx = at(x, w, ow, ox);
                        let sy = at(y, h, oh, oy);
                        let ix = sx.floor() as usize;
                        let iy = sy.floor() as usize;
                        let fx = sx - sx.floor();
                        let fy = sy - sy.floor();
                        let read =
                            |a: usize, b: usize| planes[plane][b.min(ph - 1) * pw + a.min(pw - 1)];
                        let a = read(ix, iy) * (1.0 - fx) + read(ix + 1, iy) * fx;
                        let b = read(ix, iy + 1) * (1.0 - fx) + read(ix + 1, iy + 1) * fx;
                        a * (1.0 - fy) + b * fy
                    };
                    let rgb = |ox: usize, oy: usize| {
                        let maximum = ((1u32 << depth) - 1) as f64 / scale;
                        let yy = if colour.full {
                            sample(0, ox, oy) / maximum
                        } else {
                            (sample(0, ox, oy) - 16.0) / 219.0
                        };
                        let crange = if colour.full { maximum } else { 224.0 };
                        let cb = (sample(1, ox, oy) - 128.0) / crange;
                        let cr = (sample(2, ox, oy) - 128.0) / crange;
                        let r = yy + 2.0 * (1.0 - colour.kr) * cr;
                        let b = yy + 2.0 * (1.0 - colour.kb) * cb;
                        let g =
                            (yy - colour.kr * r - colour.kb * b) / (1.0 - colour.kr - colour.kb);
                        [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
                    };
                    let output_codes = |rgb: [f64; 3]| {
                        let y = rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
                        let cb = (rgb[2] - y) / (2.0 * (1.0 - 0.0722));
                        let cr = (rgb[0] - y) / (2.0 * (1.0 - 0.2126));
                        let max = ((1u32 << depth) - 1) as f64;
                        [
                            (16.0 + y * 219.0) * scale,
                            (128.0 + cb * 224.0) * scale,
                            (128.0 + cr * 224.0) * scale,
                        ]
                        .map(|v| v.clamp(0.0, max).round() as u16)
                    };
                    let mut expected = [Vec::new(), Vec::new(), Vec::new()];
                    for row in 0..oh {
                        for col in 0..ow {
                            expected[0].push(output_codes(rgb(col, row))[0]);
                        }
                    }
                    for row in 0..oh.div_ceil(2) {
                        for col in 0..ow.div_ceil(2) {
                            let mut average = [0.0; 3];
                            for dy in 0..2 {
                                for dx in 0..2 {
                                    let pixel =
                                        rgb((col * 2 + dx).min(ow - 1), (row * 2 + dy).min(oh - 1));
                                    for c in 0..3 {
                                        average[c] += pixel[c] * 0.25;
                                    }
                                }
                            }
                            let codes = output_codes(average);
                            expected[1].push(codes[1]);
                            expected[2].push(codes[2]);
                        }
                    }
                    for (index, data) in [&actual.y, &actual.cb, &actual.cr].into_iter().enumerate()
                    {
                        let codes: Vec<u16> = if depth == 8 {
                            data.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            data.chunks_exact(2)
                                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                                .collect()
                        };
                        assert_eq!(codes.len(), expected[index].len());
                        let worst = codes
                            .into_iter()
                            .zip(&expected[index])
                            .map(|(a, &b)| a.abs_diff(b))
                            .max()
                            .unwrap();
                        assert!(
                            worst <= 1,
                            "depth={depth} rotation={rotation} crop={x},{y},{w},{h} output={ow},{oh} plane={index} worst={worst}"
                        );
                    }
                }
            }
        }
    }
}
