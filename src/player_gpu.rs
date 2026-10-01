//! Video presentation on the GPU: the decoded Y, Cb and Cr planes are uploaded
//! as byte textures (R8 for 8-bit, RG8 for little-endian wider samples) and
//! converted to RGB at source precision in a fragment shader (Metal on
//! macOS through wgpu), so no CPU pass touches the pixels after decoding. The
//! shader holds the colour decision too when the grade is one lookup: the table
//! the CPU baked rides as a texture — the 3D grid of nodes, or the three byte
//! tables for a plan that keeps its channels apart — and every sample reads its
//! codes out of that same table.
use crate::color::{Grade, Interpolation, Lut3d, ShaderLook};
use crate::playback_native::{AvcColour, PackedPlanar, Planar8};
use eframe::{egui_wgpu, wgpu};
use std::sync::Arc;

const SHADER: &str = r#"
struct Params {
    y_offset: f32,
    y_gain: f32,
    c_offset: f32,
    c_gain: f32,
    kr: f32,
    kb: f32,
    srgb: f32,
    sample_scale: f32,
    // The part of the planes a crop keeps, as texture coordinates: the corner
    // it starts at, then the corner it ends at. The whole picture is (0,0)-(1,1).
    window: vec4<f32>,
    // VLC's picture settings as the five numbers its adjust filter computes:
    // the luma becomes gamma⁻¹(clip(lum + contrast·y)) on stored samples and
    // the chroma pair rotates by the hue and scales by the saturation around
    // grey. The identity bundle — 1, 0, 1, 1, 0 — leaves both as they stand.
    adjust_a: vec4<f32>, // contrast, lum, inverse gamma, cos·saturation
    adjust_b: vec4<f32>, // sin·saturation, then whether the bundle moves at all
    // The grade the CPU baked, as one lookup into the table it itself reads.
    // `grid.x` is a grid's edge length, `grid.y` how to read between its nodes —
    // 0 for nearest, 1 for trilinear, 2 for tetrahedral — and `grid.z` says
    // which table: 0 is the 3D grid of nodes bound at 5, 1 is the byte tables
    // bound at 6. `grid.w` turns the lookup off, so a frame with nothing to
    // grade pays nothing for either binding.
    grid: vec4<f32>,
    look: vec4<f32>,
    look_min: vec4<f32>,
    look_max: vec4<f32>,
    encoder_size: vec4<f32>,
};
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_cb: texture_2d<f32>;
@group(0) @binding(2) var plane_cr: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> params: Params;
// The grade's nodes, one texel per node: the table lists red fastest, so node
// (r, g, b) is texel (r, g, b) and the walk below is `Lut3d::sample`'s.
@group(0) @binding(5) var grade_grid: texture_3d<f32>;
// A plan that maps each channel on its own is three byte tables, not a grid, and
// that is what the CPU paints from. Texel `code` of this one row holds the three
// tables' answers for a channel coded `code`, each in its own component.
@group(0) @binding(7) var look_grid: texture_3d<f32>;
@group(0) @binding(6) var grade_tables: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    let p = corners[index];
    var out: VertexOut;
    out.position = vec4(p, 0.0, 1.0);
    // The quad spans the rect it is drawn into, so the crop is a span of
    // texture coordinates rather than a change of where the quad lands.
    let t = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    let uv = mix(params.window.xy, params.window.zw, t);
    out.uv = uv;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}

// Interpolate normalized byte texels in float32, preserving precision when
// an RGB8 source is scaled into a 10-bit encoder surface.
fn sample_rgb(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(textureDimensions(plane_y, 0));
    var at = clamp(uv * vec2<f32>(size) - vec2(0.5), vec2(0.0), vec2<f32>(size - vec2(1)));
    if params.encoder_size.x > 0.0 {
        let edge = vec2<f32>(size - vec2(1));
        let low = clamp(params.window.xy * vec2<f32>(size), vec2(0.0), edge);
        let high = max(low, clamp(params.window.zw * vec2<f32>(size) - vec2(1.0), vec2(0.0), edge));
        at = clamp(at, low, high);
    }
    let lo = vec2<i32>(floor(at));
    let hi = min(lo + vec2(1), size - vec2(1));
    let fraction = fract(at);
    let a = textureLoad(plane_y, lo, 0).rgb;
    let b = textureLoad(plane_y, vec2(hi.x, lo.y), 0).rgb;
    let c = textureLoad(plane_y, vec2(lo.x, hi.y), 0).rgb;
    let d = textureLoad(plane_y, hi, 0).rgb;
    return mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
}

fn grid_node(grid: texture_3d<f32>, at: vec3<i32>) -> vec3<f32> {
    return textureLoad(grid, at, 0).rgb;
}

/// The component axis `i` names. A vector cannot be indexed by a runtime value,
/// so the tetrahedral walk below asks rather than subscript.
fn axis_of(d: vec3<f32>, i: i32) -> f32 {
    if i == 0 {
        return d.x;
    }
    if i == 1 {
        return d.y;
    }
    return d.z;
}

/// The unit step along axis `i`: 0 is red, 1 green, 2 blue, the same order the
/// CPU's table indexes its nodes in.
fn along_axis(i: i32) -> vec3<i32> {
    if i == 0 {
        return vec3(1, 0, 0);
    }
    if i == 1 {
        return vec3(0, 1, 0);
    }
    return vec3(0, 0, 1);
}

/// The axis of the largest offset, the lowest axis keeping a tie — the sort's
/// first pick. Walking up with a strict `>` leaves the earliest maximum.
fn tallest_axis(d: vec3<f32>) -> i32 {
    var best = i32(0);
    for (var i = i32(1); i < i32(3); i = i + 1) {
        if axis_of(d, i) > axis_of(d, best) {
            best = i;
        }
    }
    return best;
}

/// The axis of the smallest offset, the highest axis keeping a tie — the same
/// sort's last pick. Walking down with a strict `<` leaves the latest minimum.
fn shortest_axis(d: vec3<f32>) -> i32 {
    var best = i32(2);
    for (var i = i32(1); i >= i32(0); i = i - 1) {
        if axis_of(d, i) < axis_of(d, best) {
            best = i;
        }
    }
    return best;
}

/// Read the three byte tables, which is what `Grade::paint` does: the code a
/// channel carries indexes its own table, and the tables were baked along the
/// grey axis — valid only because `Grade::new` measured that the plan keeps its
/// channels apart before it let this route exist.
fn table_read(rgb: vec3<f32>) -> vec3<f32> {
    let at = rgb * vec3(255.0);
    let r = textureLoad(grade_tables, vec2<i32>(i32(round(at.x)), 0), 0).r;
    let g = textureLoad(grade_tables, vec2<i32>(i32(round(at.y)), 0), 0).g;
    let b = textureLoad(grade_tables, vec2<i32>(i32(round(at.z)), 0), 0).b;
    return vec3(r, g, b);
}

/// Read the baked grid, the way `Lut3d::sample` reads the same table: the input
/// is a display code in [0, 1], the grid's domain is [0, 1] because the plan
/// baked it, and the three modes are the CPU's three — a snap to the nearest
/// node, a box blend of eight, and a walk of four nodes along the ordered
/// offsets.
fn grade_read(rgb: vec3<f32>) -> vec3<f32> {
    if params.grid.z > 0.5 { return table_read(rgb); }
    let converted = read_grid(rgb, grade_grid, params.grid);
    if params.look.w < 0.5 { return converted; }
    let normalized = clamp((converted - params.look_min.xyz) / max(params.look_max.xyz - params.look_min.xyz, vec3(0.000001)), vec3(0.0), vec3(1.0));
    if params.look.z > 0.5 {
        let position = normalized * (params.look.x - 1.0);
        return vec3(look_channel(position.x, 0), look_channel(position.y, 1), look_channel(position.z, 2));
    }
    return read_grid(normalized, look_grid, params.look);
}
fn look_node(index: i32) -> vec3<f32> {
    let size = vec3<i32>(textureDimensions(look_grid, 0));
    return textureLoad(look_grid, vec3(index % size.x, (index / size.x) % size.y, index / (size.x * size.y)), 0).rgb;
}
fn look_channel(position: f32, channel: i32) -> f32 {
    let lower = i32(floor(position));
    let upper = min(lower + 1, i32(params.look.x) - 1);
    let weight = position - f32(lower);
    return axis_of(look_node(lower), channel) * (1.0 - weight) + axis_of(look_node(upper), channel) * weight;
}
fn read_grid(rgb: vec3<f32>, grid: texture_3d<f32>, options: vec4<f32>) -> vec3<f32> {
    let last = vec3<i32>(i32(options.x) - 1);
    let p = rgb * vec3(f32(last.x));
    let origin = vec3<i32>(floor(p));
    let offsets = p - vec3<f32>(origin);
    if options.y < 0.5 {
        let near = select(vec3<i32>(0), vec3<i32>(1), offsets > vec3(0.5));
        return grid_node(grid, min(origin + near, last));
    }
    if options.y < 1.5 {
        let r1 = min(origin + vec3(1, 0, 0), last);
        let g1 = min(origin + vec3(0, 1, 0), last);
        let b1 = min(origin + vec3(0, 0, 1), last);
        let c000 = grid_node(grid, origin);
        let c100 = grid_node(grid, r1);
        let c010 = grid_node(grid, g1);
        let c110 = grid_node(grid, min(origin + vec3(1, 1, 0), last));
        let c001 = grid_node(grid, b1);
        let c101 = grid_node(grid, min(r1 + vec3(0, 0, 1), last));
        let c011 = grid_node(grid, min(g1 + vec3(0, 0, 1), last));
        let c111 = grid_node(grid, min(r1 + vec3(0, 1, 1), last));
        let x00 = mix(c000, c100, vec3(offsets.x));
        let x10 = mix(c010, c110, vec3(offsets.x));
        let x01 = mix(c001, c101, vec3(offsets.x));
        let x11 = mix(c011, c111, vec3(offsets.x));
        let y0 = mix(x00, x10, vec3(offsets.y));
        let y1 = mix(x01, x11, vec3(offsets.y));
        return mix(y0, y1, vec3(offsets.z));
    }
    // Order the axes by offset and walk the four nodes of that tetrahedron:
    // origin, plus the tallest axis, plus the middle one, plus all three. The
    // middle is the axis neither of the two walks named, because the three axes
    // are 0, 1 and 2 and the pair can never land on one axis twice.
    let a = tallest_axis(offsets);
    let c = shortest_axis(offsets);
    let b = 3 - a - c;
    let e1 = along_axis(a);
    let e2 = e1 + along_axis(b);
    let n0 = grid_node(grid, origin);
    let n1 = grid_node(grid, min(origin + e1, last));
    let n2 = grid_node(grid, min(origin + e2, last));
    let n3 = grid_node(grid, min(origin + vec3(1, 1, 1), last));
    let sa = axis_of(offsets, a);
    let sb = axis_of(offsets, b);
    let sc = axis_of(offsets, c);
    return n0 * (1.0 - sa) + n1 * (sa - sb) + n2 * (sb - sc) + n3 * sc;
}

/// The five numbers of the bundle run over display codes, which is what the
/// CPU draw path does to a picture that has already left the planes behind:
/// BT.601 limited range solved back out of the bytes, the luma line and its
/// gamma curve, the chroma turned around grey, and the same matrix forward
/// again. `adjust_rgb` is the other half of this and both are read by the same
/// test, because a viewer's slider has to move the shown picture the same way
/// whichever route drew it.
fn adjust_codes(rgb: vec3<f32>) -> vec3<f32> {
    // The same landing on a byte the lookup in front of it makes: the CPU route
    // hands its sliders a frame of stored codes, not the grid's float answer.
    let stored = round(rgb * vec3(255.0));
    let solved = (0.299 * stored.x + 0.587 * stored.y + 0.114 * stored.z) / 1.164 + 16.0;
    let cb = (-0.168736 * stored.x - 0.331264 * stored.y + 0.5 * stored.z) * (224.0 / 255.0);
    let cr = (0.5 * stored.x - 0.418688 * stored.y - 0.081312 * stored.z) * (224.0 / 255.0);
    let luma = clamp(params.adjust_a.x * solved + params.adjust_a.y, 0.0, 255.0);
    let turned = pow(luma / 255.0, params.adjust_a.z) * 255.0;
    // Both lines read the pair as it was solved, which is the rotation VLC's
    // filter works out.
    let cb_adj = params.adjust_a.w * cb + params.adjust_b.x * cr;
    let cr_adj = params.adjust_a.w * cr - params.adjust_b.x * cb;
    let y = 1.164 * (turned - 16.0);
    let r = y + 1.596 * cr_adj;
    let g = y - 0.391 * cb_adj - 0.813 * cr_adj;
    let b = y + 2.013 * cb_adj;
    let shown = clamp(round(vec3(r, g, b)), vec3(0.0), vec3(255.0));
    return shown / vec3(255.0);
}

// Plane sampling returns source codes scaled into eight-bit code units.
// Integer reconstruction before interpolation preserves source low bits.
fn stored(sampled: vec4<f32>) -> f32 { return sampled.r; }
fn decoded_texel(sampled: vec4<f32>) -> vec4<f32> {
    if params.adjust_b.z > 1.5 {
        // Recover exact MSB-aligned integer codes before interpolation.
        return round(sampled * (65535.0 / 64.0)) * 0.25;
    }
    return vec4<f32>(dot(round(sampled.rg * 255.0), vec2(1.0, 256.0)) * params.sample_scale, 0.0, 0.0, 0.0);
}

fn source_uv(uv: vec2<f32>) -> vec2<f32> {
    if params.adjust_b.w == 1.0 { return vec2(uv.y, 1.0 - uv.x); }
    if params.adjust_b.w == 2.0 { return vec2(1.0 - uv.x, 1.0 - uv.y); }
    if params.adjust_b.w == 3.0 { return vec2(1.0 - uv.y, uv.x); }
    return uv;
}
fn source_texel(plane: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(plane, 0));
    var point = p;
    if params.adjust_b.w == 1.0 { point = vec2(p.y, size.y - 1 - p.x); }
    if params.adjust_b.w == 2.0 { point = size - vec2(1) - p; }
    if params.adjust_b.w == 3.0 { point = vec2(size.x - 1 - p.y, p.x); }
    return decoded_texel(textureLoad(plane, point, 0));
}

// Wide stored samples must interpolate in float32. Hardware filtering of
// split low/high-byte channels can round each component independently and
// amplify that error at byte boundaries. Native P010 uses this same rule.
fn sample_plane(plane: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    var size = vec2<i32>(textureDimensions(plane, 0));
    if params.adjust_b.w == 1.0 || params.adjust_b.w == 3.0 { size = size.yx; }
    var bounded_uv = uv;
    if params.encoder_size.x > 0.0 {
        let half_pixel = vec2(0.5) / vec2<f32>(size);
        let low = min(params.window.xy + half_pixel, params.window.zw);
        let high = max(low, params.window.zw - half_pixel);
        bounded_uv = clamp(uv, low, high);
    }
    if params.sample_scale == 0.0 && params.adjust_b.z < 1.5 {
        return textureSample(plane, plane_sampler, source_uv(bounded_uv)) * 255.0;
    }
    let position = bounded_uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(position));
    let fraction = fract(position);
    let last = size - vec2<i32>(1);
    let a = source_texel(plane, clamp(base, vec2<i32>(0), last));
    let b = source_texel(plane, clamp(base + vec2<i32>(1, 0), vec2<i32>(0), last));
    let c = source_texel(plane, clamp(base + vec2<i32>(0, 1), vec2<i32>(0), last));
    let d = source_texel(plane, clamp(base + vec2<i32>(1, 1), vec2<i32>(0), last));
    return mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    let graded = params.grid.w > 0.5;
    // VLC's picture settings work on the stored 8-bit samples, so with no grade
    // to run first they are applied here: the luma line, clamped like the
    // filter's tables, then its gamma curve, and the chroma turned around grey
    // — all before any range maths, the way the filter sees the frame it is
    // given. Under a grade there is no stored sample left to step over: the
    // grade is the colour management and the sliders belong on the codes it
    // leaves, which is the only picture a brightness slider can promise to
    // brighten. That order is what the CPU route has always drawn, so the two
    // agree by construction rather than by approximation.
    var y_stored = stored(sample_plane(plane_y, in.uv));
    var cb_c = stored(sample_plane(plane_cb, in.uv)) - params.c_offset;
    var cr_c = stored(sample_plane(plane_cr, in.uv)) - params.c_offset;
    if params.adjust_b.z > 0.5 {
        let uv = sample_plane(plane_cb, in.uv);
        cb_c = uv.r - params.c_offset;
        cr_c = uv.g - params.c_offset;
    }
    if !graded && params.adjust_b.y > 0.5 {
        let luma = clamp(params.adjust_a.x * y_stored + params.adjust_a.y, 0.0, 255.0);
        y_stored = pow(luma / 255.0, params.adjust_a.z) * 255.0;
        let cb_turned = params.adjust_a.w * cb_c + params.adjust_b.x * cr_c;
        let cr_turned = params.adjust_a.w * cr_c - params.adjust_b.x * cb_c;
        cb_c = cb_turned;
        cr_c = cr_turned;
    }
    let y = (y_stored - params.y_offset) * params.y_gain;
    let cb = cb_c * params.c_gain;
    let cr = cr_c * params.c_gain;
    let r = y + 2.0 * (1.0 - params.kr) * cr;
    let b = y + 2.0 * (1.0 - params.kb) * cb;
    let g = (y - params.kr * r - params.kb * b) / (1.0 - params.kr - params.kb);
    var rgb = clamp(vec3(r, g, b), vec3(0.0), vec3(1.0));
    if params.adjust_b.z == 3.0 {
        rgb = sample_rgb(in.uv);
        if params.adjust_b.y > 0.5 {
            rgb = clamp(adjust_codes(rgb), vec3(0.0), vec3(1.0));
        }
    }
    if graded {
        // The CPU route indexes its grid by the byte it stores, so land on that
        // byte first: the two routes then read the same nodes, and what is left
        // between them is arithmetic rather than a different colour decision.
        let code = round(rgb * 255.0) / 255.0;
        rgb = clamp(grade_read(code), vec3(0.0), vec3(1.0));
        if params.adjust_b.y > 0.5 {
            rgb = clamp(adjust_codes(rgb), vec3(0.0), vec3(1.0));
        }
    }
    if params.srgb > 0.5 {
        // The swapchain encodes to sRGB on write; hand it linear light.
        rgb = to_linear(rgb);
    }
    return vec4(rgb, 1.0);
}
"#;

/// The settings bundle that leaves every sample as it stands, mirroring VLC's
/// dialog defaults with its switch off: unity multipliers and no turn. The
/// player's sliders are folded into this shape before either draw path sees
/// them.
pub const IDENTITY_ADJUST: [f32; 5] = [1.0, 0.0, 1.0, 1.0, 0.0];

/// Display grading/shader output converted into native encoder surfaces.
/// Output supports explicit dimensions and quarter-turn rotation; no framebuffer sRGB encoding.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub struct MetalEncoderRenderer {
    gpu: VideoGpu,
    chroma: wgpu::RenderPipeline,
    depth: u8,
    full_range: bool,
    window: [f32; 4],
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
impl MetalEncoderRenderer {
    pub fn new(
        device: &wgpu::Device,
        shader: &ColorShader,
        depth: u8,
        full_range: bool,
        kr: f32,
        kb: f32,
    ) -> crate::Result<Self> {
        if device.adapter_info().backend != wgpu::Backend::Metal {
            return Err(crate::invalid("native encoder rendering requires Metal"));
        }
        if depth == 10
            && !device.features().contains(
                wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            )
        {
            return Err(crate::invalid(
                "P010 rendering requires normalized16 and adapter-specific formats",
            ));
        }
        let source = shader
            .encoder_source(depth, full_range, kr, kb)?
            .replace("fn encoder_y(", "fn fs(");
        let formats = if depth == 8 {
            [wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm]
        } else {
            [
                wgpu::TextureFormat::R16Unorm,
                wgpu::TextureFormat::Rg16Unorm,
            ]
        };
        let gpu = build_with_source(device, formats[0], &source);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("encoder chroma"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&gpu.layout)],
            immediate_size: 0,
        });
        let chroma = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("encoder chroma"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("encoder_uv"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: formats[1],
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Self {
            gpu,
            chroma,
            depth,
            full_range,
            window: [0.0, 0.0, 1.0, 1.0],
        })
    }
    /// Crop in upright normalized source coordinates before output scaling.
    pub fn set_window(&mut self, window: [f32; 4]) -> crate::Result<()> {
        if window
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
            || window[0] >= window[2]
            || window[1] >= window[3]
        {
            return Err(crate::invalid("invalid encoder crop window"));
        }
        self.window = window;
        Ok(())
    }
    pub fn render_rgb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        self.render_rgb_sized(device, queue, rgb, size, size)
    }
    /// Resample RGB input into the requested native encoder geometry on the GPU.
    pub fn render_rgb_sized(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
        output_size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        self.gpu
            .upload_rgb(device, queue, rgb, size, 0, self.window, IDENTITY_ADJUST)?;
        self.render(device, queue, output_size)
    }
    /// Upload packed input once, then grade/shade/resample into a native surface.
    pub fn render_planar8_sized(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        grade: Option<&Arc<Grade>>,
        output_size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        self.gpu.upload(
            device,
            queue,
            frame,
            serial,
            self.window,
            IDENTITY_ADJUST,
            grade,
        );
        self.render(device, queue, output_size)
    }
    /// Upload packed input once, then grade/shade/resample into a native surface.
    pub fn render_packed_sized(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &PackedPlanar,
        serial: u64,
        grade: Option<&Arc<Grade>>,
        output_size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        self.gpu.upload_packed(
            device,
            queue,
            frame,
            serial,
            self.window,
            IDENTITY_ADJUST,
            grade,
        )?;
        self.render(device, queue, output_size)
    }
    pub fn render_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &fvid_vt::Surface,
        colour: AvcColour,
        serial: u64,
        grade: Option<&Arc<Grade>>,
    ) -> crate::Result<fvid_vt::Surface> {
        self.render_surface_rotated(device, queue, surface, colour, serial, grade, 0)
    }
    pub fn render_surface_rotated(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &fvid_vt::Surface,
        colour: AvcColour,
        serial: u64,
        grade: Option<&Arc<Grade>>,
        rotation: u16,
    ) -> crate::Result<fvid_vt::Surface> {
        let size = if matches!(rotation, 90 | 270) {
            [surface.height(), surface.width()]
        } else {
            [surface.width(), surface.height()]
        };
        self.render_surface_sized(
            device, queue, surface, colour, serial, grade, rotation, size,
        )
    }
    /// Rotate and resample native input into the requested encoder geometry.
    pub fn render_surface_sized(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface: &fvid_vt::Surface,
        colour: AvcColour,
        serial: u64,
        grade: Option<&Arc<Grade>>,
        rotation: u16,
        output_size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        self.gpu.import_surface_rotated(
            device,
            queue,
            surface,
            colour,
            serial,
            self.window,
            IDENTITY_ADJUST,
            grade,
            rotation,
        )?;
        self.render(device, queue, output_size)
    }
    fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: [usize; 2],
    ) -> crate::Result<fvid_vt::Surface> {
        let limit = device.limits().max_texture_dimension_2d as usize;
        if size.iter().any(|&n| n == 0 || n > limit) {
            return Err(crate::invalid("invalid native encoder output dimensions"));
        }
        let geometry: Vec<u8> = [size[0] as f32, size[1] as f32, 0.0, 0.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        queue.write_buffer(&self.gpu.uniform, 144, &geometry);
        let group = self
            .gpu
            .bind_group
            .as_ref()
            .ok_or_else(|| crate::invalid("encoder source is not bound"))?;
        fvid_vt::Surface::metal_render(
            device,
            queue,
            size[0],
            size[1],
            self.depth,
            self.full_range,
            |pass, index| {
                pass.set_pipeline(if index == 0 {
                    &self.gpu.pipeline
                } else {
                    &self.chroma
                });
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..6, 0..1);
            },
        )
        .map_err(|e| crate::Error::Gpu(e.to_string()))
    }
}

/// GPU objects shared by every frame, stored in egui's callback resources.
pub struct VideoGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    srgb: bool,
    planes: Option<Planes>,
    surface_mode: f32,
    rgb_source: Option<Arc<Vec<u8>>>,
    rotation: u16,
    /// The grade the shader reads, which travels with the bind group below.
    grid: Grid,
    bind_group: Option<wgpu::BindGroup>,
    /// The plane geometry and grade the current bind group was built for: a
    /// change to either needs a new one, and nothing else does.
    bound: Option<([usize; 2], [usize; 2], usize)>,
    serial: u64,
    /// The crop the uniform currently holds, kept so a crop changed while the
    /// same frame stays on screen still reaches the shader.
    window: [f32; 4],
    /// The settings bundle the uniform currently holds, kept for the same
    /// reason: a slider moved over a held frame reaches the shader too.
    adjust: [f32; 5],
}
struct Planes {
    size: [usize; 2],
    chroma: [usize; 2],
    depth: u8,
    textures: [wgpu::Texture; 3],
    views: [wgpu::TextureView; 3],
}

/// The tables bound for a graded plane picture, and the grade that filled them:
/// either the 3D grid of nodes at binding 5, or the three byte tables at binding
/// 6, whichever the grade itself reads.
///
/// A grade with nothing to look up still needs both bound — a two-node grid and
/// an empty row of bytes the shader never reads, because `grid.w` says the
/// lookup is off.
struct Grid {
    /// The grade these textures mirror, kept so the frame after this one can be
    /// told it reads the same colour without comparing a megabyte of nodes. A
    /// grade whose tables the shader cannot carry is `None` here and stays on
    /// the route that takes both steps on the CPU.
    source: Option<Arc<Grade>>,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// The byte tables' one row of 256 texels. Fixed in size, so it is made once
    /// and only ever rewritten, and the bind group that holds it stays good.
    look_texture: wgpu::Texture,
    look_view: wgpu::TextureView,
    look_size: usize,
    look_linear: bool,
    look_params: [f32; 12],
    table_texture: wgpu::Texture,
    table_view: wgpu::TextureView,
    /// Edge length the grid texture was made for, and a count of how often one
    /// has replaced another: the bind group holds the view, so a new texture
    /// needs a new bind group and nothing else does.
    size: usize,
    epoch: usize,
    /// Edge length, read mode, which table and the on switch, as the uniform
    /// carries them.
    params: [f32; 4],
}

/// The two-node grid bound when a picture has nothing to grade.
const GRID_OFF: [f32; 4] = [2.0, 0.0, 0.0, 0.0];

/// The number the shader names the byte tables with in `grid.z`.
const TABLES: f32 = 1.0;

/// The number the shader matches a read mode against: 0 snaps to a node, 1
/// blends the eight around it, 2 walks the four of a tetrahedron.
fn grid_mode(interpolation: Interpolation) -> f32 {
    match interpolation {
        Interpolation::Nearest => 0.0,
        Interpolation::Trilinear => 1.0,
        Interpolation::Tetrahedral => 2.0,
    }
}

/// A display-code RGB shader applied after colour management and adjustment,
/// before the framebuffer's sRGB conversion. Implement
/// `fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32>`.
#[derive(Clone, Debug)]
pub struct ColorShader {
    compiled: String,
}
impl ColorShader {
    pub fn new(source: &str) -> crate::Result<Self> {
        if source.len() > 64 * 1024 {
            return Err(crate::invalid("shader source exceeds 64 KiB"));
        }
        let compiled = format!("{}\n{}", SHADER.replace(
            "    if params.srgb > 0.5 {",
            "    rgb = clamp(process_color(rgb, in.uv), vec3(0.0), vec3(1.0));\n    if params.srgb > 0.5 {"
        ), source);
        let module = naga::front::wgsl::parse_str(&compiled)
            .map_err(|error| crate::Error::Invalid(error.emit_to_string(&compiled)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| crate::Error::Invalid(error.emit_to_string(&compiled)))?;
        if module.entry_points.len() != 2 || module.global_variables.len() != 8 {
            return Err(crate::invalid(
                "display shader must only add functions and constants",
            ));
        }
        Ok(Self { compiled })
    }
    pub fn compiled_source(&self) -> &str {
        &self.compiled
    }
    /// Encoder fragment entries for explicit output dimensions and quarter-turn rotation. The
    /// renderer must set `params.srgb` to zero (encoder input is display codes).
    /// Chroma averages four RGB samples before 4:2:0 subsampling.
    pub fn encoder_source(
        &self,
        depth: u8,
        full_range: bool,
        kr: f32,
        kb: f32,
    ) -> crate::Result<String> {
        if ![8, 10].contains(&depth)
            || !kr.is_finite()
            || !kb.is_finite()
            || kr <= 0.0
            || kb <= 0.0
            || kr + kb >= 1.0
        {
            return Err(crate::invalid("invalid encoder depth or YUV coefficients"));
        }
        let (max, offset, range, chroma_range, pack) = if depth == 8 {
            (
                255.0,
                if full_range { 0.0 } else { 16.0 },
                if full_range { 255.0 } else { 219.0 },
                if full_range { 255.0 } else { 224.0 },
                1.0 / 255.0,
            )
        } else {
            (
                1023.0,
                if full_range { 0.0 } else { 64.0 },
                if full_range { 1023.0 } else { 876.0 },
                if full_range { 1023.0 } else { 896.0 },
                64.0 / 65535.0,
            )
        };
        let center = if depth == 8 { 128.0 } else { 512.0 };
        let mut source = self.compiled.replace(
            "@fragment\nfn fs(in: VertexOut) -> @location(0) vec4<f32>",
            "fn display_codes(in: VertexOut) -> vec4<f32>",
        );
        source.push_str(&format!(
            r#"
fn encoder_yuv(rgb: vec3<f32>) -> vec3<f32> {{
    let y = dot(rgb, vec3<f32>({kr},{kg},{kb}));
    return vec3(y, (rgb.b-y)/(2.0*(1.0-{kb})), (rgb.r-y)/(2.0*(1.0-{kr})));
}}
@fragment fn encoder_y(in: VertexOut) -> @location(0) vec4<f32> {{
    let y = encoder_yuv(display_codes(in).rgb).x;
    let code = round(clamp({offset}+y*{range},0.0,{max}));
    return vec4(code*{pack},0.0,0.0,1.0);
}}
@fragment fn encoder_uv(in: VertexOut) -> @location(0) vec4<f32> {{
    let size = params.encoder_size.xy;
    let delta = vec2<f32>(1.0) / size;
    let origin = (floor(in.position.xy)*2.0+vec2(0.5))/size;
    let last = (size-vec2(0.5))/size;
    var point = in;
    point.uv = mix(params.window.xy,params.window.zw,min(origin,last));
    var rgb = display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + vec2(delta.x,0.0),last));
    rgb += display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + vec2(0.0,delta.y),last));
    rgb += display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + delta,last));
    rgb += display_codes(point).rgb;
    let uv = encoder_yuv(rgb*0.25).yz;
    let code = round(clamp(vec2({center})+uv*{chroma_range},vec2(0.0),vec2({max})));
    return vec4(code*{pack},0.0,1.0);
}}
"#,
            kg = 1.0 - kr - kb
        ));
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|e| crate::invalid(&e.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|e| crate::invalid(&e.emit_to_string(&source)))?;
        Ok(source)
    }
}

/// Explicit GPU renderer selection. Software adapters are excluded and the
/// requested API/device cannot silently fall back to another API or the CPU.
pub fn configuration(
    backend: crate::Backend,
    ordinal: usize,
) -> crate::Result<egui_wgpu::WgpuConfiguration> {
    use crate::Backend;
    let backends = match backend {
        Backend::Auto => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        Backend::Metal => wgpu::Backends::METAL,
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Gl => wgpu::Backends::GL,
        _ => {
            return Err(crate::invalid(
                "player renderer requires auto, metal, vulkan, dx12 or gl; CUDA is a processing/codec backend",
            ));
        }
    };
    if backend == Backend::Auto && ordinal != 0 {
        return Err(crate::invalid(
            "--device requires an explicit player backend when not zero",
        ));
    }
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends;
    #[cfg(target_os = "macos")]
    {
        let descriptor = Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = descriptor(adapter);
            if adapter.get_info().backend == wgpu::Backend::Metal {
                descriptor.required_features |=
                    adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
            }
            descriptor
        });
    }
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        let mut available: Vec<_> = adapters
            .iter()
            .filter(|adapter| {
                adapter.get_info().device_type != wgpu::DeviceType::Cpu
                    && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
            })
            .collect();
        available.sort_by_key(|adapter| {
            let info = adapter.get_info();
            (info.name, info.vendor, info.device)
        });
        available
            .get(ordinal)
            .map(|adapter| (*adapter).clone())
            .ok_or_else(|| {
                format!("player backend {backend} hardware device {ordinal} unavailable")
            })
    }));
    Ok(egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    })
}

pub fn install_with_shader(
    state: &egui_wgpu::RenderState,
    shader: Option<&ColorShader>,
) -> crate::Result<()> {
    let validation = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = state.device.push_error_scope(wgpu::ErrorFilter::Internal);
    let oom = state
        .device
        .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let gpu = build_with_source(
        &state.device,
        state.target_format,
        shader.map_or(SHADER, ColorShader::compiled_source),
    );
    for scope in [oom, internal, validation] {
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(crate::Error::Gpu(error.to_string()));
        }
    }
    state.renderer.write().callback_resources.insert(gpu);
    Ok(())
}

/// Create the pipeline once and register it with the renderer.
pub fn install(state: &egui_wgpu::RenderState) {
    let gpu = build(&state.device, state.target_format);
    state.renderer.write().callback_resources.insert(gpu);
}

/// The objects one video draw needs for one target format: the pipeline, its
/// bind group layout, the sampler that magnifies the picture, and the parameter
/// buffer [`upload`](VideoGpu::upload) fills. `install` asks for the window's
/// format; a test asks for a plain 8-bit one so the picture the shader paints
/// can be read back and set against `planar8_to_rgb`, the conversion the player
/// falls back to as soon as a frame has to be graded.
fn build(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> VideoGpu {
    build_with_source(device, target_format, SHADER)
}
fn build_with_source(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    source: &str,
) -> VideoGpu {
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fvid video"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let texture = |binding| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("fvid video"),
        entries: &[
            texture(0),
            texture(1),
            texture(2),
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT | wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Read with `textureLoad`, one texel per node, so the grid needs no
            // filtering and the shader's own weights are the CPU's weights.
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            // The byte tables' row, likewise loaded rather than sampled. An
            // 8-unorm texture is filterable by definition, so it asks for no
            // more than that.
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 7,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("fvid video"),
        bind_group_layouts: &[Some(&layout)],
        ..Default::default()
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fvid video"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("fvid video"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fvid video params"),
        // The colour conversion's scalars padded to a vec4, then the crop
        // window, picture settings, both LUTs, authored domain and encoder geometry: 160 bytes laid
        // out as the shader's Params declares.
        size: 160,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    VideoGpu {
        pipeline,
        layout,
        sampler,
        uniform,
        srgb: target_format.is_srgb(),
        planes: None,
        surface_mode: 0.0,
        rgb_source: None,
        rotation: 0,
        grid: Grid::empty(device),
        bind_group: None,
        bound: None,
        serial: u64::MAX,
        window: [0.0; 4],
        adjust: IDENTITY_ADJUST,
    }
}

impl Grid {
    /// True when the bound texture already mirrors exactly this grade, which is
    /// what lets a repaint of the same picture skip both the table copy and the
    /// uniform that names it.
    fn holds(&self, grade: Option<&Arc<Grade>>) -> bool {
        match (&self.source, grade) {
            (Some(held), Some(want)) => Arc::ptr_eq(held, want),
            (None, None) => true,
            _ => false,
        }
    }

    /// The grid bound while a picture has nothing to look up, with the shader's
    /// switch set so it never reads either table.
    fn empty(device: &wgpu::Device) -> Self {
        let (texture, view) = make_grid(device, 2);
        let (table_texture, table_view) = make_tables(device);
        let (look_texture, look_view) = make_grid(device, 2);
        Self {
            source: None,
            texture,
            view,
            look_texture,
            look_view,
            look_size: 2,
            look_linear: false,
            look_params: [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0],
            table_texture,
            table_view,
            size: 2,
            params: GRID_OFF,
            epoch: 0,
        }
    }

    /// Point the bound tables at `grade`'s own lookup. A grid of the same edge
    /// length is written into the texture that is already there, so the bind
    /// group that holds it stays good and only a different size — a new
    /// `--grid`, say, or a grade that reads bytes rather than nodes — costs a
    /// new texture and a rebinding.
    fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grade: Option<&Arc<Grade>>) {
        if self.holds(grade) {
            return;
        }
        let look = grade.and_then(|grade| grade.shader_look()).or_else(|| {
            grade.and_then(|g| match g.shader_stages() {
                crate::color::ShaderStages::Chain {
                    conversion: crate::color::Lut::Three(cube),
                    look: Some(_),
                } if g.is_gpu_grade() => Some(ShaderLook::Grid(cube.clone())),
                _ => None,
            })
        });
        let second = grade.and_then(|g| match g.shader_stages() {
            crate::color::ShaderStages::Chain {
                look: Some(crate::color::Lut::Three(cube)),
                ..
            } => Some(cube),
            _ => None,
        });
        self.look_params = [2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0, 0.0];
        if let Some(cube) = second {
            if cube.size != self.look_size || self.look_linear {
                (self.look_texture, self.look_view) = make_grid(device, cube.size);
                self.look_size = cube.size;
                self.look_linear = false;
                self.epoch += 1;
            }
            upload_grid(queue, &self.look_texture, cube.size, &cube.data);
            self.look_params = [
                cube.size as f32,
                grid_mode(grade.unwrap().interpolation()),
                0.0,
                1.0,
                cube.domain_min[0],
                cube.domain_min[1],
                cube.domain_min[2],
                0.0,
                cube.domain_max[0],
                cube.domain_max[1],
                cube.domain_max[2],
                0.0,
            ];
        }
        if let Some(crate::color::ShaderStages::Chain {
            look: Some(crate::color::Lut::One(line)),
            ..
        }) = grade
            .filter(|g| g.is_gpu_grade())
            .map(|g| g.shader_stages())
        {
            let size = line.len();
            if self.look_size != size || !self.look_linear {
                (self.look_texture, self.look_view) = make_line(device, size);
                self.look_size = size;
                self.look_linear = true;
                self.epoch += 1;
            }
            let extent = self.look_texture.size();
            let row = (extent.width as usize * 16).next_multiple_of(256);
            let mut bytes =
                vec![0u8; row * extent.height as usize * extent.depth_or_array_layers as usize];
            for i in 0..size {
                let offset = (i / extent.width as usize) * row + (i % extent.width as usize) * 16;
                for channel in 0..3 {
                    bytes[offset + channel * 4..offset + channel * 4 + 4]
                        .copy_from_slice(&line.data[channel][i].to_le_bytes());
                }
            }
            queue.write_texture(
                self.look_texture.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row as u32),
                    rows_per_image: Some(extent.height),
                },
                extent,
            );
            self.look_params = [
                size as f32,
                0.0,
                1.0,
                1.0,
                line.domain_min[0],
                line.domain_min[1],
                line.domain_min[2],
                0.0,
                line.domain_max[0],
                line.domain_max[1],
                line.domain_max[2],
                0.0,
            ];
        }
        let grid = match &look {
            Some(ShaderLook::Grid(cube)) => Some(cube),
            _ => None,
        };
        let size = grid.map_or(2, |cube| cube.size);
        if size != self.size {
            let (texture, view) = make_grid(device, size);
            self.texture = texture;
            self.view = view;
            self.size = size;
            self.epoch += 1;
        }
        let nodes: Vec<[f32; 3]> =
            grid.map_or_else(|| Lut3d::identity(2).data, |cube| cube.data.clone());
        upload_grid(queue, &self.texture, size, &nodes);
        if let Some(ShaderLook::Tables(tables)) = &look {
            // One texel per input code, holding each channel's own answer in its
            // own component, so the shader reads the exact bytes `paint` writes.
            let mut tabled = [0u8; 4 * 256];
            for code in 0..256 {
                for channel in 0..3 {
                    tabled[code * 4 + channel] = tables[channel][code];
                }
                tabled[code * 4 + 3] = 255;
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.table_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &tabled,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * 256),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 256,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.source = grade.cloned();
        // A picture with no table to read leaves the switch off, so it pays for
        // the bindings and not for the lookup.
        self.params = match (&grade, &look) {
            (Some(grade), Some(ShaderLook::Grid(cube))) => {
                [cube.size as f32, grid_mode(grade.interpolation()), 0.0, 1.0]
            }
            (Some(grade), Some(ShaderLook::Tables(_))) => {
                [1.0, grid_mode(grade.interpolation()), TABLES, 1.0]
            }
            _ => GRID_OFF,
        };
    }
}

fn make_line(device: &wgpu::Device, size: usize) -> (wgpu::Texture, wgpu::TextureView) {
    let width = size.min(128);
    let height = size.div_ceil(width).min(128);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid float 1D look"),
        size: wgpu::Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: size.div_ceil(width * height) as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&Default::default());
    (texture, view)
}

fn upload_grid(queue: &wgpu::Queue, texture: &wgpu::Texture, size: usize, nodes: &[[f32; 3]]) {
    // One `rgba32float` texel per node, in the table's own order: red
    // fastest, so the node (r, g, b) lands at texel (r, g, b), which is how
    // the shader indexes it. `write_texture` copies through a staging buffer
    // whose rows are 256-byte aligned, so a node row is padded out to that.
    const TEXEL: usize = 4 * std::mem::size_of::<f32>();
    let packed = size * TEXEL;
    let row = packed.next_multiple_of(256);
    let mut bytes = vec![0u8; row * size * size];
    for (index, node) in nodes.iter().enumerate() {
        let from = index * TEXEL;
        let to = (from / packed) * row + from % packed;
        for (channel, value) in [node[0], node[1], node[2], 1.0].iter().enumerate() {
            let at = to + channel * std::mem::size_of::<f32>();
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row as u32),
            rows_per_image: Some(size as u32),
        },
        wgpu::Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: size as u32,
        },
    );
}

/// A 3D texture with one `rgba32float` texel per node of a `size`³ grid.
fn make_grid(device: &wgpu::Device, size: usize) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid grade grid"),
        size: wgpu::Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: size as u32,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D3,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// A 256×1 row of `rgba8unorm`, one texel per input code of the three byte
/// tables. The bytes the CPU stores are the bytes the shader loads, so neither
/// route rounds a second time.
fn make_tables(device: &wgpu::Device) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("fvid grade tables"),
        size: wgpu::Extent3d {
            width: 256,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

struct FramePlanes<'a> {
    data: [&'a [u8]; 3],
    size: [usize; 2],
    chroma: [usize; 2],
    depth: u8,
    colour: AvcColour,
}

impl VideoGpu {
    fn upload_shared_rgb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &Arc<Vec<u8>>,
        size: [usize; 2],
        window: [f32; 4],
        adjust: [f32; 5],
    ) -> crate::Result<bool> {
        if self.surface_mode == 3.0
            && self.planes.as_ref().is_some_and(|p| p.size == size)
            && self
                .rgb_source
                .as_ref()
                .is_some_and(|source| Arc::ptr_eq(source, rgb))
        {
            if self.window != window || self.adjust != adjust {
                self.window = window;
                self.adjust = adjust;
                self.write_params(queue, 8, AvcColour::default(), window, adjust);
            }
            return Ok(false);
        }
        self.upload_rgb(device, queue, rgb, size, 0, window, adjust)?;
        self.rgb_source = Some(rgb.clone());
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
    ) {
        self.upload_planes(
            device,
            queue,
            FramePlanes {
                data: [&frame.y, &frame.cb, &frame.cr],
                size: [frame.width, frame.height],
                chroma: [frame.chroma_width, frame.chroma_height],
                depth: 8,
                colour: frame.colour,
            },
            serial,
            window,
            adjust,
            grade,
        );
    }
    #[allow(clippy::too_many_arguments)]
    fn upload_packed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &PackedPlanar,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
    ) -> crate::Result<()> {
        let data = frame.plane_data()?;
        let [sx, sy] = frame
            .frame
            .subsampling
            .expect("validated planar subsampling");
        self.upload_planes(
            device,
            queue,
            FramePlanes {
                data,
                size: [frame.frame.width, frame.frame.height],
                chroma: [
                    frame.frame.width.div_ceil(sx),
                    frame.frame.height.div_ceil(sy),
                ],
                depth: frame.depth,
                colour: frame.colour,
            },
            serial,
            window,
            adjust,
            grade,
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn upload_rgb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
    ) -> crate::Result<()> {
        let len = size[0].checked_mul(size[1]).and_then(|n| n.checked_mul(3));
        if size.contains(&0)
            || len != Some(rgb.len())
            || size
                .iter()
                .any(|&dimension| dimension > device.limits().max_texture_dimension_2d as usize)
        {
            return Err(crate::invalid("invalid RGB display frame"));
        }
        if self.surface_mode != 3.0 || self.planes.as_ref().is_none_or(|p| p.size != size) {
            let mut planes = self.create_planes(device, size, [1, 1], 8);
            planes.textures[0] = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fvid RGB display"),
                size: wgpu::Extent3d {
                    width: size[0] as u32,
                    height: size[1] as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            planes.views[0] = planes.textures[0].create_view(&Default::default());
            self.planes = Some(planes);
            self.bound = None;
        }
        self.surface_mode = 3.0;
        self.rgb_source = None;
        self.rotation = 0;
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        self.grid.set(device, queue, None);
        self.bind_planes(device, size, [1, 1]);
        let rgba: Vec<u8> = rgb
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        queue.write_texture(
            self.planes.as_ref().unwrap().textures[0].as_image_copy(),
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size[0] as u32 * 4),
                rows_per_image: Some(size[1] as u32),
            },
            wgpu::Extent3d {
                width: size[0] as u32,
                height: size[1] as u32,
                depth_or_array_layers: 1,
            },
        );
        self.write_params(queue, 8, AvcColour::default(), window, adjust);
        Ok(())
    }
    fn upload_planes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: FramePlanes<'_>,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
    ) {
        if self.surface_mode == 0.0
            && self.serial == serial
            && self.window == window
            && self.adjust == adjust
            && self.grid.holds(grade)
        {
            return;
        }
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        let size = frame.size;
        let chroma = frame.chroma;
        let bytes_per_sample = if frame.depth == 8 { 1 } else { 2 };
        if self.planes.as_ref().is_none_or(|p| {
            p.size != size
                || p.chroma != chroma
                || p.depth != frame.depth
                || self.surface_mode != 0.0
        }) {
            self.planes = Some(self.create_planes(device, size, chroma, frame.depth));
            self.bound = None;
        }
        self.surface_mode = 0.0;
        self.rgb_source = None;
        self.rotation = 0;
        self.grid.set(device, queue, grade);
        self.bind_planes(device, size, chroma);
        let planes = self.planes.as_ref().unwrap();
        for (index, (data, [w, h])) in [
            (frame.data[0], size),
            (frame.data[1], chroma),
            (frame.data[2], chroma),
        ]
        .into_iter()
        .enumerate()
        {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &planes.textures[index],
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(w as u32 * bytes_per_sample),
                    rows_per_image: Some(h as u32),
                },
                wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.write_params(queue, frame.depth, frame.colour, window, adjust);
    }
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    fn import_surface(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &fvid_vt::Surface,
        colour: AvcColour,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
    ) -> crate::Result<()> {
        self.import_surface_rotated(
            device, queue, frame, colour, serial, window, adjust, grade, 0,
        )
    }
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[allow(clippy::too_many_arguments)]
    fn import_surface_rotated(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &fvid_vt::Surface,
        mut colour: AvcColour,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
    ) -> crate::Result<()> {
        if ![0, 90, 180, 270].contains(&rotation) {
            return Err(crate::invalid("invalid surface rotation"));
        }
        let native_mode = if frame.depth() == 8 { 1.0 } else { 2.0 };
        if self.surface_mode == native_mode
            && self.rotation == rotation
            && self.serial == serial
            && self.window == window
            && self.adjust == adjust
            && self.grid.holds(grade)
        {
            return Ok(());
        }
        if self.surface_mode != native_mode || self.serial != serial {
            let imported = frame
                .import_metal(device)
                .map_err(|e| crate::Error::Gpu(e.to_string()))?;
            let textures = [imported.y, imported.uv.clone(), imported.uv];
            let views = std::array::from_fn(|i| textures[i].create_view(&Default::default()));
            self.planes = Some(Planes {
                size: [frame.width(), frame.height()],
                chroma: [frame.width().div_ceil(2), frame.height().div_ceil(2)],
                depth: frame.depth(),
                textures,
                views,
            });
            self.bound = None;
        }
        self.rotation = rotation;
        self.surface_mode = if frame.depth() == 8 { 1.0 } else { 2.0 };
        self.rgb_source = None;
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        self.grid.set(device, queue, grade);
        self.bind_planes(
            device,
            [frame.width(), frame.height()],
            [frame.width().div_ceil(2), frame.height().div_ceil(2)],
        );
        colour.full = frame.full_range();
        self.write_params(queue, frame.depth(), colour, window, adjust);
        Ok(())
    }
    fn bind_planes(&mut self, device: &wgpu::Device, size: [usize; 2], chroma: [usize; 2]) {
        let bound = (size, chroma, self.grid.epoch);
        if self.bound != Some(bound) {
            let planes = self.planes.as_ref().unwrap();
            let views = &planes.views;
            let grid = &self.grid;
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fvid video"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&views[0]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&views[1]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&views[2]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::TextureView(&grid.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: wgpu::BindingResource::TextureView(&grid.table_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: wgpu::BindingResource::TextureView(&grid.look_view),
                    },
                ],
            }));
            self.bound = Some(bound);
        }
    }
    fn write_params(
        &self,
        queue: &wgpu::Queue,
        depth: u8,
        colour: AvcColour,
        window: [f32; 4],
        adjust: [f32; 5],
    ) {
        // Limited range maps 16..235 (luma) and 16..240 (chroma) onto 0..1.
        let scale = (1u32 << (depth - 8)) as f32;
        let full_range = ((1u32 << depth) - 1) as f32 / scale;
        let (y_offset, y_range, c_range) = if colour.full {
            (0.0, full_range, full_range)
        } else {
            (16.0, 219.0, 224.0)
        };
        // The settings ride as the shader's two vectors: the luma trio with
        // the cosine of the turn in the fourth slot, the sine and a switch in
        // the other, the rest of the space padding to the vector width. The
        // switch says the bundle is not the identity, which is when the shader
        // steps the shown picture through the matrix round trip at all — the
        // draw path hands over the frame untouched for that one bundle, and a
        // round trip costs a step of rounding even when nothing moves. The grid
        // vector is the texture's own shape, filled in when it was last
        // written: edge length, how to read between nodes, and whether to.
        let params: [f32; 24] = [
            y_offset,
            1.0 / y_range,
            128.0,
            1.0 / c_range,
            colour.kr as f32,
            colour.kb as f32,
            if self.srgb { 1.0 } else { 0.0 },
            if depth > 8 { 1.0 / scale } else { 0.0 },
            window[0],
            window[1],
            window[2],
            window[3],
            adjust[0],
            adjust[1],
            adjust[2],
            adjust[3],
            adjust[4],
            f32::from(adjust != IDENTITY_ADJUST),
            self.surface_mode,
            f32::from(self.rotation / 90),
            self.grid.params[0],
            self.grid.params[1],
            self.grid.params[2],
            self.grid.params[3],
        ];
        let bytes: Vec<u8> = params
            .iter()
            .chain(self.grid.look_params.iter())
            .chain([0.0f32; 4].iter())
            .flat_map(|v| v.to_le_bytes())
            .collect();
        queue.write_buffer(&self.uniform, 0, &bytes);
    }
    fn create_planes(
        &self,
        device: &wgpu::Device,
        size: [usize; 2],
        chroma: [usize; 2],
        depth: u8,
    ) -> Planes {
        let make = |label: &str, [w, h]: [usize; 2]| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: if depth == 8 {
                    wgpu::TextureFormat::R8Unorm
                } else {
                    wgpu::TextureFormat::Rg8Unorm
                },
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let textures = [
            make("fvid plane y", size),
            make("fvid plane cb", chroma),
            make("fvid plane cr", chroma),
        ];
        let views: [wgpu::TextureView; 3] = [
            textures[0].create_view(&wgpu::TextureViewDescriptor::default()),
            textures[1].create_view(&wgpu::TextureViewDescriptor::default()),
            textures[2].create_view(&wgpu::TextureViewDescriptor::default()),
        ];
        Planes {
            depth,
            size,
            chroma,
            textures,
            views,
        }
    }
}

/// Paint callback drawing one planar frame into its rect, sampling only the
/// part of it a crop leaves, and looking the frame's codes up in `grade` when
/// the colour it states is not the colour the panel shows.
pub struct VideoCallback {
    pub frame: Arc<Planar8>,
    pub serial: u64,
    pub window: [f32; 4],
    pub adjust: [f32; 5],
    /// The grade this picture's codes are shown through, which is the one the
    /// frame kept its planes for: a grade whose grid the shader cannot carry
    /// was applied on the way here, and arrives as packed RGB with this `None`.
    pub grade: Option<Arc<Grade>>,
}

/// Display already graded RGB codes through the same custom shader as planar frames.
pub struct RgbVideoCallback {
    pub frame: Arc<Vec<u8>>,
    pub size: [usize; 2],
    pub window: [f32; 4],
    pub adjust: [f32; 5],
}
impl egui_wgpu::CallbackTrait for RgbVideoCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<VideoGpu>() {
            if let Err(error) = gpu.upload_shared_rgb(
                device,
                queue,
                &self.frame,
                self.size,
                self.window,
                self.adjust,
            ) {
                eprintln!("RGB display: {error}");
                gpu.bind_group = None;
            }
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(gpu) = resources.get::<VideoGpu>() {
            if let Some(group) = &gpu.bind_group {
                pass.set_pipeline(&gpu.pipeline);
                pass.set_bind_group(0, group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
    }
}
impl egui_wgpu::CallbackTrait for VideoCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<VideoGpu>() {
            gpu.upload(
                device,
                queue,
                &self.frame,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
            );
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<VideoGpu>() else {
            return;
        };
        let Some(bind_group) = &gpu.bind_group else {
            return;
        };
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

/// Paint source-precision planar samples using the same pipeline and colour tables.
pub struct PackedVideoCallback {
    pub frame: Arc<PackedPlanar>,
    pub serial: u64,
    pub window: [f32; 4],
    pub adjust: [f32; 5],
    pub grade: Option<Arc<Grade>>,
}
impl egui_wgpu::CallbackTrait for PackedVideoCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<VideoGpu>() {
            if let Err(error) = gpu.upload_packed(
                device,
                queue,
                &self.frame,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
            ) {
                gpu.bind_group = None;
                eprintln!("{error}");
            }
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(gpu) = resources.get::<VideoGpu>() {
            if let Some(bind_group) = &gpu.bind_group {
                pass.set_pipeline(&gpu.pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
    }
}

/// Render a retained decoder surface directly, keeping pixels on the GPU.
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub struct SurfaceVideoCallback {
    pub frame: fvid_vt::Surface,
    pub rotation: u16,
    pub colour: AvcColour,
    pub serial: u64,
    pub window: [f32; 4],
    pub adjust: [f32; 5],
    pub grade: Option<Arc<Grade>>,
}
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
impl egui_wgpu::CallbackTrait for SurfaceVideoCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(gpu) = resources.get_mut::<VideoGpu>() {
            if let Err(error) = gpu.import_surface_rotated(
                device,
                queue,
                &self.frame,
                self.colour,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
                self.rotation,
            ) {
                gpu.bind_group = None;
                gpu.bound = None;
                gpu.serial = u64::MAX;
                eprintln!("{error}");
            }
        }
        Vec::new()
    }
    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(gpu) = resources.get::<VideoGpu>() {
            if let Some(bind_group) = &gpu.bind_group {
                pass.set_pipeline(&gpu.pipeline);
                pass.set_bind_group(0, bind_group, &[]);
                pass.draw(0..6, 0..1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn hardware_export_copies_mp4_audio_with_gapless_timing() {
        use crate::container::{matroska_write, mp4::Mp4Reader, webm::WebmReader};
        use crate::hardware_export::*;
        let (device, queue) = headless().unwrap();
        let surface = fvid_vt::Surface::metal_black(&device, &queue, 32, 32, 8, false).unwrap();
        let destination = std::env::temp_dir().join(format!(
            "fvid-audio-export-{}-{}.mkv",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(destination.clone());
        for bytes in [
            include_bytes!("../tests/fixtures/audio/two-audio.mp4").as_slice(),
            include_bytes!("../tests/fixtures/audio/aac-native-edit.m4a").as_slice(),
        ] {
            let input = Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
            let audio_indices: Vec<_> = input
                .tracks()
                .iter()
                .enumerate()
                .filter(|(_, t)| t.handler == *b"soun")
                .map(|(i, _)| i)
                .collect();
            assert!(!audio_indices.is_empty());
            let frames = (0..3).map(|i| {
                Ok(HardwareFrame {
                    surface: surface.clone(),
                    pts: EncodedTime {
                        value: i,
                        timescale: 25,
                    },
                    duration: EncodedTime {
                        value: 1,
                        timescale: 25,
                    },
                    force_keyframe: false,
                })
            });
            write_video_file_with_mp4_audio(
                &destination,
                frames,
                EncoderCodec::H264,
                1_000_000,
                TrackOptions::default(),
                input,
            )
            .unwrap();
            let mut output = WebmReader::open(
                std::fs::File::open(&destination).unwrap(),
                Default::default(),
            )
            .unwrap();
            output.scan_all().unwrap();
            assert_eq!(output.tracks.len(), audio_indices.len() + 1);
            for (additional, source_index) in audio_indices.into_iter().enumerate() {
                let mut source =
                    Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
                let mut expected = std::io::Cursor::new(Vec::new());
                matroska_write::write_mp4_aac(&mut source, source_index, &mut expected, None, None)
                    .unwrap();
                expected.set_position(0);
                let mut reference = WebmReader::open(expected, Default::default()).unwrap();
                reference.scan_all().unwrap();
                let track = additional + 1;
                assert_eq!(
                    output.tracks[track].codec_private,
                    reference.tracks[0].codec_private
                );
                assert_eq!(
                    output.tracks[track].codec_delay_ns,
                    reference.tracks[0].codec_delay_ns
                );
                let actual: Vec<_> = output
                    .packets
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.track == output.tracks[track].number)
                    .map(|(i, p)| (i, p.pts_ns, p.duration_ns, p.discard_padding_ns))
                    .collect();
                assert_eq!(actual.len(), reference.packets.len());
                for ((i, pts, duration, padding), j) in
                    actual.into_iter().zip(0..reference.packets.len())
                {
                    assert_eq!(
                        (pts, duration, padding),
                        (
                            reference.packets[j].pts_ns,
                            reference.packets[j].duration_ns,
                            reference.packets[j].discard_padding_ns
                        )
                    );
                    assert_eq!(
                        output.read_packet(i).unwrap(),
                        reference.read_packet(j).unwrap()
                    );
                }
                for interval in [
                    None,
                    Some((
                        std::time::Duration::from_millis(10),
                        std::time::Duration::from_millis(20),
                    )),
                ] {
                    let source =
                        Mp4Reader::open(std::io::Cursor::new(bytes), Default::default()).unwrap();
                    let exported = WebmReader::open(
                        std::fs::File::open(&destination).unwrap(),
                        Default::default(),
                    )
                    .unwrap();
                    let mut source_pcm = Vec::new();
                    let mut exported_pcm = Vec::new();
                    let mut control = crate::native_media::DecodeProgress::new(None, None).unwrap();
                    let source_stats = crate::native_media::decode_mp4_audio_reader_controlled(
                        source,
                        &mut source_pcm,
                        interval,
                        Some(source_index),
                        &mut control,
                    )
                    .unwrap();
                    let mut control = crate::native_media::DecodeProgress::new(None, None).unwrap();
                    let exported_stats =
                        crate::native_media::decode_matroska_audio_reader_controlled(
                            exported,
                            &mut exported_pcm,
                            interval,
                            Some(track),
                            &mut control,
                        )
                        .unwrap();
                    assert!(source_stats.sample_frames > 0);
                    assert_eq!(source_stats.sample_frames, exported_stats.sample_frames);
                    assert_eq!(
                        source_pcm, exported_pcm,
                        "PCM differs for source track {source_index}, interval {interval:?}"
                    );
                }
            }
            std::fs::remove_file(&destination).unwrap();
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn hardware_video_mux_interleaves_subtitles_and_preserves_tail() {
        use crate::container::matroska_write::Encoding;
        use crate::hardware_export::*;
        let (device, queue) = headless().unwrap();
        let surface = fvid_vt::Surface::metal_black(&device, &queue, 32, 32, 8, false).unwrap();
        let frames = (0..3).map(|i| {
            Ok(HardwareFrame {
                surface: surface.clone(),
                pts: EncodedTime {
                    value: i,
                    timescale: 25,
                },
                duration: EncodedTime {
                    value: 1,
                    timescale: 25,
                },
                force_keyframe: false,
            })
        });
        let payload = b"0,0,Default,,0,0,0,,caption";
        let mut audio_source = crate::container::mp4::Mp4Reader::open(
            std::io::Cursor::new(
                include_bytes!("../tests/fixtures/audio/two-audio.mp4").as_slice(),
            ),
            crate::container::mp4::Limits::default(),
        )
        .unwrap();
        let audio_index = audio_source
            .tracks()
            .iter()
            .position(|t| t.codec == *b"mp4a")
            .unwrap();
        let audio = &audio_source.tracks()[audio_index];
        let asc = crate::codec::config::aac_specific_config(&audio.configuration)
            .unwrap()
            .to_vec();
        let sample_rate = audio.sample_rate;
        let channels = audio.channels;
        let mut audio_payload = Vec::new();
        audio_source
            .read_packet(audio_index, 0, &mut audio_payload)
            .unwrap();
        let times = [20_000_000, 60_000_000, 300_000_000];
        let packets = times
            .into_iter()
            .map(|pts_ns| {
                Ok(AdditionalPacket {
                    track: 0,
                    pts_ns,
                    duration_ns: 100_000_000,
                    key_frame: true,
                    data: payload.to_vec(),
                    options: PacketOptions::default(),
                })
            })
            .chain(std::iter::once(Ok(AdditionalPacket {
                track: 1,
                pts_ns: 400_000_000,
                duration_ns: 21_333_333,
                key_frame: true,
                data: audio_payload.clone(),
                options: PacketOptions {
                    discard_padding_ns: 1_000_000,
                    invisible: false,
                },
            })));
        let track = AdditionalTrack {
            encoding: Encoding::Ass {
                configuration: b"[Script Info]\nScriptType: v4.00+\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize\nStyle: Default,Arial,20\n[Events]\n",
            },
            name: "captions",
            language: "eng",
        };
        let mut output = std::io::Cursor::new(Vec::new());
        let stats = write_video_with_tracks(
            &mut output,
            frames,
            EncoderCodec::H264,
            1_000_000,
            TrackOptions::default(),
            vec![
                track,
                AdditionalTrack {
                    encoding: Encoding::Aac {
                        configuration: &asc,
                        sample_rate,
                        channels,
                    },
                    name: "audio",
                    language: "eng",
                },
            ],
            vec![TrackOptions::default(), TrackOptions::default()],
            packets,
        )
        .unwrap();
        assert_eq!(stats.frames, 3);
        output.set_position(0);
        let mut reader = crate::container::webm::WebmReader::open(
            output,
            crate::container::webm::Limits::default(),
        )
        .unwrap();
        assert_eq!(reader.tracks.len(), 3);
        assert_eq!(reader.tracks[1].codec, "S_TEXT/ASS");
        reader.scan_all().unwrap();
        assert_eq!(reader.tracks[2].codec, "A_AAC");
        assert_eq!(reader.tracks[2].codec_private, asc);
        let (audio_packet_index, audio_packet) = reader
            .packets
            .iter()
            .enumerate()
            .find(|(_, p)| p.track == reader.tracks[2].number)
            .unwrap();
        assert_eq!(audio_packet.pts_ns, 400_000_000);
        assert_eq!(audio_packet.duration_ns, Some(21_333_333));
        assert_eq!(audio_packet.discard_padding_ns, 1_000_000);
        assert_eq!(
            reader.read_packet(audio_packet_index).unwrap(),
            audio_payload
        );
        let subtitles: Vec<_> = reader
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.track == reader.tracks[1].number)
            .map(|(i, p)| (i, p.pts_ns, p.duration_ns))
            .collect();
        assert_eq!(subtitles.len(), 3);
        for ((index, pts, duration), expected) in subtitles.into_iter().zip(times) {
            assert_eq!(pts, expected as i64);
            assert_eq!(duration, Some(100_000_000));
            assert_eq!(reader.read_packet(index).unwrap(), payload);
        }
        assert!(
            reader
                .packets
                .windows(2)
                .all(|p| p[0].pts_ns <= p[1].pts_ns)
        );
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn native_crop_scaling_matches_cropped_rotated_packed_source() {
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
            let mut reader =
                NativeReader::new(std::io::Cursor::new(bytes), 64 * 1024 * 1024).unwrap();
            assert!(reader.enable_shared_surfaces().unwrap());
            let RawFrame::Surface { surface, colour } = reader.read_frame_raw().unwrap().unwrap()
            else {
                panic!("native surface required")
            };
            let source = surface_to_packed(&surface, colour).unwrap();
            let depth = source.depth;
            let sample_bytes = if depth == 8 { 1 } else { 2 };
            let mut renderer =
                MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722).unwrap();
            for rotation in [0, 90, 180, 270] {
                let rotated = source.rotated(rotation).unwrap();
                let sw = rotated.frame.width;
                let sh = rotated.frame.height;
                let [x, y, w, h] = [2, 2, sw - 4, sh - 4];
                let planes = rotated.plane_data().unwrap();
                let mut data = Vec::new();
                for (index, plane) in planes.into_iter().enumerate() {
                    let divisor = if index == 0 { 1 } else { 2 };
                    let stride = sw / divisor;
                    for row in y / divisor..(y + h) / divisor {
                        let begin = (row * stride + x / divisor) * sample_bytes;
                        data.extend_from_slice(&plane[begin..begin + w / divisor * sample_bytes]);
                    }
                }
                let cropped = PackedPlanar::new(
                    crate::native_geometry::GeometryFrame {
                        width: w,
                        height: h,
                        subsampling: Some([2, 2]),
                        data,
                    },
                    depth,
                    colour,
                )
                .unwrap();
                let output = [w * 2 + 2, h * 2 + 2];
                renderer.set_window([0.0, 0.0, 1.0, 1.0]).unwrap();
                let expected = renderer
                    .render_packed_sized(&device, &queue, &cropped, 0, None, output)
                    .unwrap()
                    .download()
                    .unwrap();
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
                        &device, &queue, &surface, colour, 0, None, rotation, output,
                    )
                    .unwrap()
                    .download()
                    .unwrap();
                for (a, b) in [&actual.y, &actual.cb, &actual.cr].into_iter().zip([
                    &expected.y,
                    &expected.cb,
                    &expected.cr,
                ]) {
                    let codes = |p: &[u8]| -> Vec<u16> {
                        if depth == 8 {
                            p.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            p.chunks_exact(2)
                                .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
                                .collect()
                        }
                    };
                    let worst = codes(a)
                        .into_iter()
                        .zip(codes(b))
                        .map(|(a, b)| a.abs_diff(b))
                        .max()
                        .unwrap();
                    assert!(
                        worst <= 1,
                        "depth={depth} rotation={rotation} worst={worst}"
                    );
                }
            }
        }
    }
    include!("player_gpu_odd_crop_tests.rs");
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_crop_matches_independent_rgb_crop_in_all_planes() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new(
            "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
        )
        .unwrap();
        let source: Vec<u8> = (0..11 * 9)
            .flat_map(|i| {
                [
                    (i * 73 % 256) as u8,
                    (i * 31 % 256) as u8,
                    (i * 113 % 256) as u8,
                ]
            })
            .collect();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for [x, y, w, h] in [[2, 2, 7, 5], [1, 1, 8, 6], [0, 0, 11, 9]] {
                    let cropped: Vec<u8> = (y..y + h)
                        .flat_map(|row| {
                            source[(row * 11 + x) * 3..(row * 11 + x + w) * 3]
                                .iter()
                                .copied()
                        })
                        .collect();
                    for output_size in [
                        [w, h],
                        [w * 2 + 1, h * 2 + 1],
                        [w.div_ceil(2), h.div_ceil(2)],
                    ] {
                        renderer.set_window([0.0, 0.0, 1.0, 1.0]).unwrap();
                        let expected = renderer
                            .render_rgb_sized(&device, &queue, &cropped, [w, h], output_size)
                            .unwrap()
                            .download()
                            .unwrap();
                        renderer
                            .set_window([
                                x as f32 / 11.0,
                                y as f32 / 9.0,
                                (x + w) as f32 / 11.0,
                                (y + h) as f32 / 9.0,
                            ])
                            .unwrap();
                        let actual = renderer
                            .render_rgb_sized(&device, &queue, &source, [11, 9], output_size)
                            .unwrap()
                            .download()
                            .unwrap();
                        for (a, b) in [&actual.y, &actual.cb, &actual.cr].into_iter().zip([
                            &expected.y,
                            &expected.cb,
                            &expected.cr,
                        ]) {
                            let codes = |p: &[u8]| -> Vec<u16> {
                                if depth == 8 {
                                    p.iter().map(|&v| u16::from(v)).collect()
                                } else {
                                    p.chunks_exact(2)
                                        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                        .collect()
                                }
                            };
                            assert!(
                                codes(a)
                                    .into_iter()
                                    .zip(codes(b))
                                    .all(|(a, b)| a.abs_diff(b) <= 1),
                                "crop {x}:{y}:{w}:{h} output={output_size:?} depth={depth} full={full}"
                            );
                        }
                    }
                }
                assert!(renderer.set_window([0.0, 0.0, f32::NAN, 1.0]).is_err());
                assert!(renderer.set_window([0.5, 0.0, 0.5, 1.0]).is_err());
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn native_shader_export_stream_encodes_muxes_and_reopens() {
        use crate::hardware_export::{
            EncodedTime, EncoderCodec, HardwareFrame, ShaderFrame, TrackOptions,
        };
        use crate::playback_native::{NativeReader, RawFrame};
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let mut reader = NativeReader::new(
            std::io::Cursor::new(
                include_bytes!("../tests/fixtures/display/par-2x1.mp4").as_slice(),
            ),
            64 * 1024 * 1024,
        )
        .unwrap();
        assert!(reader.enable_shared_surfaces().unwrap());
        let RawFrame::Surface { surface, colour } = reader.read_frame_raw().unwrap().unwrap()
        else {
            panic!("native decoder surface required")
        };
        drop(reader);
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        for (depth, codec) in [(8, EncoderCodec::H264), (10, EncoderCodec::Hevc)] {
            let mut renderer =
                MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722).unwrap();
            let pts = [0, 1, 4];
            let duration = [1, 3, 2];
            let frames = (0..3).map(|i| {
                Ok(ShaderFrame {
                    frame: HardwareFrame {
                        surface: surface.clone(),
                        pts: EncodedTime {
                            value: pts[i],
                            timescale: 25,
                        },
                        duration: EncodedTime {
                            value: duration[i],
                            timescale: 25,
                        },
                        force_keyframe: i == 2,
                    },
                    colour,
                    grade: None,
                    rotation: 90,
                })
            });
            let mut output = std::io::Cursor::new(Vec::new());
            let stats = crate::hardware_export::write_shader_video(
                &mut output,
                frames,
                &mut renderer,
                &device,
                &queue,
                [32, 48],
                codec,
                2_000_000,
                TrackOptions::default(),
            )
            .unwrap();
            assert_eq!(stats.frames, 3);
            assert!(stats.compressed_bytes > 0);
            output.set_position(0);
            let mut reopened = NativeReader::new(output, 64 * 1024 * 1024).unwrap();
            for expected in pts {
                let _frame = reopened.read_frame_raw().unwrap().unwrap();
                assert_eq!(reopened.dimensions(), [32, 48]);
                let (ticks, scale) = reopened.current_pts().unwrap();
                assert_eq!(ticks as f64 / scale as f64, expected as f64 / 25.0);
            }
            assert!(reopened.read_frame_raw().unwrap().is_none());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_rgb_scaling_matches_independent_bilinear_source_sampling() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new(
            "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }",
        )
        .unwrap();
        let rgb: Vec<u8> = (0..7 * 5)
            .flat_map(|i| {
                [
                    (i * 73 % 256) as u8,
                    (i * 31 % 256) as u8,
                    (i * 113 % 256) as u8,
                ]
            })
            .collect();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for [w, h] in [[3, 2], [4, 3], [12, 8], [11, 9], [7, 5]] {
                    let sample = |x: usize, y: usize| {
                        let sx = ((x as f64 + 0.5) * 7.0 / w as f64 - 0.5).clamp(0.0, 6.0);
                        let sy = ((y as f64 + 0.5) * 5.0 / h as f64 - 0.5).clamp(0.0, 4.0);
                        let ix = sx.floor() as usize;
                        let iy = sy.floor() as usize;
                        let fx = sx - ix as f64;
                        let fy = sy - iy as f64;
                        std::array::from_fn::<_, 3, _>(|c| {
                            let read = |x, y| f64::from(rgb[(y * 7 + x) * 3 + c]) / 255.0;
                            let top = read(ix, iy) * (1.0 - fx) + read((ix + 1).min(6), iy) * fx;
                            let bottom = read(ix, (iy + 1).min(4)) * (1.0 - fx)
                                + read((ix + 1).min(6), (iy + 1).min(4)) * fx;
                            top * (1.0 - fy) + bottom * fy
                        })
                    };
                    let planes = renderer
                        .render_rgb_sized(&device, &queue, &rgb, [7, 5], [w, h])
                        .unwrap()
                        .download()
                        .unwrap();
                    for (channel, plane) in
                        [&planes.y, &planes.cb, &planes.cr].into_iter().enumerate()
                    {
                        let pw = if channel == 0 { w } else { w.div_ceil(2) };
                        let codes: Vec<u16> = if depth == 8 {
                            plane.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            plane
                                .chunks_exact(2)
                                .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                .collect()
                        };
                        for (i, code) in codes.into_iter().enumerate() {
                            let x = i % pw;
                            let y = i / pw;
                            let color = if channel == 0 {
                                sample(x, y)
                            } else {
                                let mut sum = [0.0; 3];
                                for dy in 0..2 {
                                    for dx in 0..2 {
                                        let value = sample(
                                            (x * 2 + dx).min(w - 1),
                                            (y * 2 + dy).min(h - 1),
                                        );
                                        for c in 0..3 {
                                            sum[c] += value[c] * 0.25;
                                        }
                                    }
                                }
                                sum
                            };
                            let luma = color[0] * 0.2126 + color[1] * 0.7152 + color[2] * 0.0722;
                            let value = match channel {
                                0 => luma,
                                1 => (color[2] - luma) / (2.0 * (1.0 - 0.0722)),
                                _ => (color[0] - luma) / (2.0 * (1.0 - 0.2126)),
                            };
                            let factor = if depth == 8 { 1.0 } else { 4.0 };
                            let max = if depth == 8 { 255.0 } else { 1023.0 };
                            let expected = if channel == 0 {
                                (if full { 0.0 } else { 16.0 * factor })
                                    + value * (if full { max } else { 219.0 * factor })
                            } else {
                                128.0 * factor + value * (if full { max } else { 224.0 * factor })
                            };
                            assert!(
                                (f64::from(code) - expected.clamp(0.0, max).round()).abs() <= 1.0,
                                "{w}x{h} depth={depth} full={full} channel={channel} at={x},{y}: {code} vs {expected}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_scales_native_output_and_aligns_chroma_to_output_pixels() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x,uv.y,step(0.7,uv.x+uv.y)); }").unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let input =
                    fvid_vt::Surface::metal_black(&device, &queue, 7, 5, depth, full).unwrap();
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                for size in [[3, 2], [4, 3], [12, 8], [11, 9]] {
                    let reference = renderer
                        .render_rgb(&device, &queue, &vec![0; size[0] * size[1] * 3], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    let rgb = renderer
                        .render_rgb_sized(&device, &queue, &[0; 7 * 5 * 3], [7, 5], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    assert_eq!(rgb.y, reference.y);
                    assert_eq!(rgb.cb, reference.cb);
                    assert_eq!(rgb.cr, reference.cr);
                    for rotation in [0, 90, 180, 270] {
                        let output = renderer
                            .render_surface_sized(
                                &device,
                                &queue,
                                &input,
                                AvcColour::default(),
                                0,
                                None,
                                rotation,
                                size,
                            )
                            .unwrap();
                        assert_eq!([output.width(), output.height()], size);
                        let actual = output.download().unwrap();
                        assert_eq!(actual.y, reference.y);
                        assert_eq!(actual.cb, reference.cb);
                        assert_eq!(actual.cr, reference.cr);
                    }
                }
                assert!(
                    renderer
                        .render_rgb_sized(&device, &queue, &[0; 3], [1, 1], [0, 1])
                        .is_err()
                );
                assert!(
                    renderer
                        .render_rgb_sized(&device, &queue, &[0; 3], [1, 1], [usize::MAX, 1])
                        .is_err()
                );
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_rotates_output_geometry_and_chroma_sampling() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x,uv.y,step(0.7,uv.x+uv.y)); }").unwrap();
        for [w, h] in [[8, 6], [7, 5]] {
            for depth in [8, 10] {
                let input =
                    fvid_vt::Surface::metal_black(&device, &queue, w, h, depth, false).unwrap();
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, false, 0.2126, 0.0722)
                        .unwrap();
                for rotation in [0, 90, 180, 270] {
                    let size = if rotation == 90 || rotation == 270 {
                        [h, w]
                    } else {
                        [w, h]
                    };
                    let reference = renderer
                        .render_rgb(&device, &queue, &vec![0; size[0] * size[1] * 3], size)
                        .unwrap()
                        .download()
                        .unwrap();
                    let output = renderer
                        .render_surface_rotated(
                            &device,
                            &queue,
                            &input,
                            AvcColour::default(),
                            0,
                            None,
                            rotation,
                        )
                        .unwrap();
                    assert_eq!([output.width(), output.height()], size);
                    let actual = output.download().unwrap();
                    assert_eq!(actual.y, reference.y, "Y rotation {rotation}");
                    assert_eq!(actual.cb, reference.cb, "Cb rotation {rotation}");
                    assert_eq!(actual.cr, reference.cr, "Cr rotation {rotation}");
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn encoder_chroma_averages_aligned_pixels_including_odd_edges() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(uv.x*uv.x, uv.y, step(0.7,uv.x+uv.y)); }").unwrap();
        for [w, h] in [[8usize, 6], [7, 5]] {
            for depth in [8, 10] {
                for full in [false, true] {
                    let mut renderer =
                        MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                            .unwrap();
                    let planes = renderer
                        .render_rgb(&device, &queue, &vec![0; w * h * 3], [w, h])
                        .unwrap()
                        .download()
                        .unwrap();
                    let rgb = |x: usize, y: usize| {
                        let u = (x.min(w - 1) as f64 + 0.5) / w as f64;
                        let v = (y.min(h - 1) as f64 + 0.5) / h as f64;
                        [u * u, v, if u + v >= 0.7 { 1.0 } else { 0.0 }]
                    };
                    let factor = if depth == 8 { 1.0 } else { 4.0 };
                    let max = if depth == 8 { 255.0 } else { 1023.0 };
                    let yuv = |rgb: [f64; 3]| {
                        let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
                        [
                            y,
                            (rgb[2] - y) / (2.0 * (1.0 - 0.0722)),
                            (rgb[0] - y) / (2.0 * (1.0 - 0.2126)),
                        ]
                    };
                    for (channel, data) in
                        [&planes.y, &planes.cb, &planes.cr].into_iter().enumerate()
                    {
                        let codes: Vec<u16> = if depth == 8 {
                            data.iter().map(|&v| u16::from(v)).collect()
                        } else {
                            data.chunks_exact(2)
                                .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                                .collect()
                        };
                        let width = if channel == 0 { w } else { w.div_ceil(2) };
                        for (i, code) in codes.into_iter().enumerate() {
                            let (x, y) = (i % width, i / width);
                            let value = if channel == 0 {
                                yuv(rgb(x, y))[0]
                            } else {
                                let mut average = [0.0; 3];
                                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                                    for (sum, v) in
                                        average.iter_mut().zip(rgb(x * 2 + dx, y * 2 + dy))
                                    {
                                        *sum += v / 4.0;
                                    }
                                }
                                yuv(average)[channel]
                            };
                            let expected = if channel == 0 {
                                (if full { 0.0 } else { 16.0 * factor })
                                    + value * (if full { max } else { 219.0 * factor })
                            } else {
                                128.0 * factor + value * (if full { max } else { 224.0 * factor })
                            };
                            assert!(
                                (f64::from(code) - expected.clamp(0.0, max).round()).abs() <= 1.0,
                                "{w}x{h} depth={depth} full={full} plane={channel} at={x},{y}: {code} vs {expected}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal"]
    fn display_shader_converts_to_native_encoder_yuv() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let mut renderer =
                    MetalEncoderRenderer::new(&device, &shader, depth, full, 0.2126, 0.0722)
                        .unwrap();
                let surface = renderer
                    .render_rgb(&device, &queue, &vec![80; 64 * 64 * 3], [64, 64])
                    .unwrap();
                let planes = surface.download().unwrap();
                let native =
                    fvid_vt::Surface::metal_black(&device, &queue, 64, 64, depth, full).unwrap();
                let native_output = renderer
                    .render_surface(&device, &queue, &native, AvcColour::default(), 1, None)
                    .unwrap();
                let native_planes = native_output.download().unwrap();
                assert_eq!(native_planes.y, planes.y);
                assert_eq!(native_planes.cb, planes.cb);
                assert_eq!(native_planes.cr, planes.cr);
                let codec = if depth == 8 {
                    fvid_vt::EncoderCodec::H264
                } else {
                    fvid_vt::EncoderCodec::Hevc
                };
                let mut encoder = fvid_vt::Encoder::new_with_depth(codec, 64, 64, depth).unwrap();
                assert!(
                    !encoder
                        .encode(&native_output, 0, 1, 25)
                        .unwrap()
                        .data
                        .is_empty()
                );
                let y: f64 = 0.2126 * 0.2 + 0.7152 * 0.4 + 0.0722 * 0.6;
                let cb = (0.6 - y) / (2.0 * (1.0 - 0.0722));
                let cr = (0.2 - y) / (2.0 * (1.0 - 0.2126));
                let factor = if depth == 8 { 1.0 } else { 4.0 };
                let max = if depth == 8 { 255.0 } else { 1023.0 };
                let y_range = if full { max } else { 219.0 * factor };
                let c_range = if full { max } else { 224.0 * factor };
                let expected: [f64; 3] = [
                    ((if full { 0.0 } else { 16.0 * factor }) + y * y_range).round(),
                    (128.0 * factor + cb * c_range).round(),
                    (128.0 * factor + cr * c_range).round(),
                ];
                for (plane, expected) in [&planes.y, &planes.cb, &planes.cr]
                    .into_iter()
                    .zip(expected)
                {
                    let codes: Vec<u16> = if depth == 8 {
                        plane.iter().map(|&v| u16::from(v)).collect()
                    } else {
                        plane
                            .chunks_exact(2)
                            .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                            .collect()
                    };
                    assert!(
                        codes
                            .iter()
                            .all(|&code| (f64::from(code) - expected).abs() <= 1.0),
                        "depth={depth} full={full} expected={expected} actual={:?}",
                        &codes[..8]
                    );
                }
            }
        }
    }
    #[test]
    fn encoder_shader_validates_nv12_p010_and_ranges() {
        let shader = ColorShader::new(include_str!("../shaders/grayscale.wgsl")).unwrap();
        for depth in [8, 10] {
            for full in [false, true] {
                let source = shader.encoder_source(depth, full, 0.2126, 0.0722).unwrap();
                let module = naga::front::wgsl::parse_str(&source).unwrap();
                assert_eq!(module.entry_points.len(), 3);
                assert!(module.entry_points.iter().any(|e| e.name == "encoder_y"));
                assert!(module.entry_points.iter().any(|e| e.name == "encoder_uv"));
            }
        }
        assert!(shader.encoder_source(12, false, 0.2126, 0.0722).is_err());
        assert!(shader.encoder_source(8, false, f32::NAN, 0.0722).is_err());
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn shader_renders_into_native_encoder_surface() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .unwrap();
        for depth in [8, 10] {
            let (y, uv, scale) = if depth == 8 {
                (83u16, 147u16, 1.0 / 255.0)
            } else {
                (337, 593, 64.0 / 65535.0)
            };
            let source = format!(
                "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{ let p = array<vec2<f32>,3>(vec2(-1.0,-1.0),vec2(3.0,-1.0),vec2(-1.0,3.0)); return vec4(p[i],0.0,1.0); }} @fragment fn y() -> @location(0) vec4<f32> {{ return vec4({},0.0,0.0,1.0); }} @fragment fn uv() -> @location(0) vec4<f32> {{ return vec4({},{},0.0,1.0); }}",
                f64::from(y) * scale,
                f64::from(uv) * scale,
                f64::from(uv) * scale
            );
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: None,
                source: wgpu::ShaderSource::Wgsl(source.into()),
            });
            let formats = if depth == 8 {
                [wgpu::TextureFormat::R8Unorm, wgpu::TextureFormat::Rg8Unorm]
            } else {
                [
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                ]
            };
            let pipelines: Vec<_> = formats
                .into_iter()
                .zip(["y", "uv"])
                .map(|(format, entry)| {
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: None,
                        layout: None,
                        vertex: wgpu::VertexState {
                            module: &module,
                            entry_point: Some("vs"),
                            compilation_options: Default::default(),
                            buffers: &[],
                        },
                        primitive: Default::default(),
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &module,
                            entry_point: Some(entry),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format,
                                blend: None,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        multiview_mask: None,
                        cache: None,
                    })
                })
                .collect();
            let surface = fvid_vt::Surface::metal_render(
                &device,
                &queue,
                64,
                64,
                depth,
                false,
                |pass, index| {
                    pass.set_pipeline(&pipelines[index]);
                    pass.draw(0..3, 0..1);
                },
            )
            .unwrap();
            let pixels = surface.download().unwrap();
            let codes = |data: &[u8]| -> Vec<u16> {
                if depth == 8 {
                    data.iter().map(|&v| u16::from(v)).collect()
                } else {
                    data.chunks_exact(2)
                        .map(|p| u16::from_le_bytes(p.try_into().unwrap()))
                        .collect()
                }
            };
            assert!(codes(&pixels.y).iter().all(|&v| v == y));
            let partial = fvid_vt::Surface::metal_render(
                &device,
                &queue,
                64,
                64,
                depth,
                false,
                |pass, index| {
                    let side = if index == 0 { 64 } else { 32 };
                    pass.set_scissor_rect(0, 0, side / 2, side);
                    pass.set_pipeline(&pipelines[index]);
                    pass.draw(0..3, 0..1);
                },
            )
            .unwrap()
            .download()
            .unwrap();
            assert!(codes(&partial.y).iter().enumerate().all(|(i, &v)| v
                == if i % 64 < 32 {
                    y
                } else if depth == 8 {
                    16
                } else {
                    64
                }));
            for plane in [&partial.cb, &partial.cr] {
                assert!(codes(plane).iter().enumerate().all(|(i, &v)| v
                    == if i % 32 < 16 {
                        uv
                    } else if depth == 8 {
                        128
                    } else {
                        512
                    }));
            }
            assert!(
                codes(&pixels.cb)
                    .iter()
                    .chain(codes(&pixels.cr).iter())
                    .all(|&v| v == uv)
            );
            let codec = if depth == 8 {
                fvid_vt::EncoderCodec::H264
            } else {
                fvid_vt::EncoderCodec::Hevc
            };
            let mut encoder = fvid_vt::Encoder::new_with_depth(codec, 64, 64, depth).unwrap();
            assert!(!encoder.encode(&surface, 0, 1, 25).unwrap().data.is_empty());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and Main10 VideoToolbox"]
    fn metal_p010_output_initializes_exact_codes_and_encodes_main10() {
        let (device, queue) = headless_features(
            wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
        )
        .expect("physical normalized16 GPU required");
        assert_eq!(device.adapter_info().backend, wgpu::Backend::Metal);
        for full in [false, true] {
            let surface = fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 10, full).unwrap();
            let pixels = surface.download().unwrap();
            assert_eq!(pixels.depth, 10);
            assert!(pixels.y.chunks_exact(2).all(|p| u16::from_le_bytes(p.try_into().unwrap()) == if full {0} else {64}));
            assert!(
                pixels
                    .cb
                    .chunks_exact(2)
                    .chain(pixels.cr.chunks_exact(2))
                    .all(|p| u16::from_le_bytes(p.try_into().unwrap()) == 512)
            );
            let mut encoder =
                fvid_vt::Encoder::new_with_depth(fvid_vt::EncoderCodec::Hevc, 64, 64, 10).unwrap();
            let encoded = encoder.encode(&surface, 0, 1, 25).unwrap();
            let config =
                crate::codec::config::HevcConfig::parse(&encoded.decoder_configuration).unwrap();
            assert_eq!(config.profile, 2);
            assert_eq!(config.bit_depth_luma, 10);
            assert!(!encoded.data.is_empty());
        }
    }
    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical Metal and VideoToolbox"]
    fn metal_output_surface_initializes_native_encoder_planes() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_eq!(device.adapter_info().backend, wgpu::Backend::Metal);
        assert!(fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 10, false).is_err());
        for full in [false, true] {
            let surface = fvid_vt::Surface::metal_black(&device, &queue, 64, 64, 8, full).unwrap();
            let pixels = surface.download().unwrap();
            assert!(pixels.y.iter().all(|&v| v == if full { 0 } else { 16 }));
            assert!(pixels.cb.iter().chain(&pixels.cr).all(|&v| v == 128));
            let mut encoder = fvid_vt::Encoder::new(fvid_vt::EncoderCodec::H264, 64, 64).unwrap();
            assert!(!encoder.encode(&surface, 0, 1, 25).unwrap().data.is_empty());
        }
    }
    #[test]
    #[ignore = "requires physical GPU"]
    fn rgb_frames_run_display_shader_and_gpu_adjustment() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let rgb: Vec<u8> = (0..4096)
            .flat_map(|i| [(i % 256) as u8, (i * 7 % 256) as u8, (i * 13 % 256) as u8])
            .collect();
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let shared = Arc::new(rgb.clone());
        let window = [0.0, 0.0, 1.0, 1.0];
        assert!(
            gpu.upload_shared_rgb(&device, &queue, &shared, [64, 64], window, IDENTITY_ADJUST)
                .unwrap()
        );
        assert!(
            !gpu.upload_shared_rgb(&device, &queue, &shared, [64, 64], window, IDENTITY_ADJUST)
                .unwrap()
        );
        let adjust = [1.2, -12.0, 0.8, 0.7, 0.3];
        assert!(
            !gpu.upload_shared_rgb(&device, &queue, &shared, [64, 64], window, adjust)
                .unwrap()
        );
        let changed = read_picture(&device, &queue, &gpu, 64, 64);
        let mut expected = Vec::new();
        crate::player::adjust_rgb(&rgb, &mut expected, &adjust);
        assert!(
            changed
                .chunks_exact(4)
                .zip(expected.chunks_exact(3))
                .all(|(a, b)| (0..3).all(|c| a[c].abs_diff(b[c]) <= 1))
        );
        let next = Arc::new(vec![90; rgb.len()]);
        assert!(
            gpu.upload_shared_rgb(&device, &queue, &next, [64, 64], window, IDENTITY_ADJUST)
                .unwrap()
        );
        assert!(
            read_picture(&device, &queue, &gpu, 64, 64)
                .chunks_exact(4)
                .all(|p| p == [90, 90, 90, 255])
        );
        for adjust in [IDENTITY_ADJUST, [1.2, -12.0, 0.8, 0.7, 0.3]] {
            gpu.upload_rgb(
                &device,
                &queue,
                &rgb,
                [64, 64],
                1,
                [0.0, 0.0, 1.0, 1.0],
                adjust,
            )
            .unwrap();
            let actual = read_picture(&device, &queue, &gpu, 64, 64);
            let mut expected = Vec::new();
            if adjust == IDENTITY_ADJUST {
                expected = rgb.clone();
            } else {
                crate::player::adjust_rgb(&rgb, &mut expected, &adjust);
            }
            let worst = actual
                .chunks_exact(4)
                .zip(expected.chunks_exact(3))
                .flat_map(|(a, b)| (0..3).map(move |c| a[c].abs_diff(b[c])))
                .max()
                .unwrap();
            assert!(worst <= 1, "GPU RGB adjustment error {worst}");
        }
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
        let mut custom = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        custom
            .upload_rgb(
                &device,
                &queue,
                &rgb,
                [64, 64],
                1,
                [0.0, 0.0, 1.0, 1.0],
                IDENTITY_ADJUST,
            )
            .unwrap();
        assert!(
            read_picture(&device, &queue, &custom, 64, 64)
                .chunks_exact(4)
                .all(|p| p == [51, 102, 153, 255])
        );
        let picture = frame(|_, _| 128, |_, _| 80);
        gpu.upload(
            &device,
            &queue,
            &picture,
            1,
            [0.0, 0.0, 1.0, 1.0],
            IDENTITY_ADJUST,
            None,
        );
        assert_eq!(
            read_picture(&device, &queue, &gpu, 64, 64),
            painted(&device, &queue, &picture, None)
        );
    }
    use super::*;
    use crate::color::hdr::{ColourDescription, HdrMetadata};
    use crate::color::log::Log;
    use crate::color::tonemap::{ContentLight, DisplayTarget};
    use crate::color::{Lut, Settings};

    /// The shader is written by hand next to the byte array that feeds it, so
    /// the one thing a machine can check without a driver is that naga accepts
    /// the WGSL and that the parameter block the upload writes is exactly as
    /// wide as the uniform buffer allocated for it.
    #[test]
    fn the_video_shader_validates_and_the_params_fit_the_uniform() {
        use naga::proc::Layouter;
        use naga::valid::{Capabilities, ValidationFlags, Validator};
        let module = naga::front::wgsl::parse_str(super::SHADER).expect("player shader must parse");
        Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .expect("player shader must validate without optional capabilities");
        let mut layouter = Layouter::default();
        let params = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Params"))
            .map(|(handle, _)| handle)
            .expect("shader declares Params");
        layouter
            .update(module.to_ctx())
            .expect("every shader type must have a layout");
        assert_eq!(
            layouter[params].size, 160,
            "Params must match the 160-byte uniform the upload writes"
        );
    }

    #[test]
    fn display_shader_validates_and_rejects_extra_resources() {
        ColorShader::new(include_str!("../shaders/grayscale.wgsl")).unwrap();
        assert!(
            ColorShader::new("fn process_color(rgb: vec3<f32>) -> vec3<f32> { return rgb; }")
                .is_err()
        );
        assert!(ColorShader::new("var<private> extra: f32; fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb; }").is_err());
        assert!(ColorShader::new(&" ".repeat(65537)).is_err());
        for backend in [crate::Backend::Cpu, crate::Backend::Cuda] {
            assert!(configuration(backend, 0).is_err());
        }
        assert!(configuration(crate::Backend::Auto, 1).is_err());
    }

    #[test]
    #[ignore = "requires a physical GPU; failure to initialize is a failure"]
    fn display_shader_executes_after_colour_conversion() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let picture = frame(|_, _| 96, |_, _| 192);
        let baseline = painted(&device, &queue, &picture, None);
        let shader = ColorShader::new(include_str!("../shaders/grayscale.wgsl")).unwrap();
        let mut gpu = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        let shown = show(&device, &queue, &mut gpu, &picture, None, IDENTITY_ADJUST);
        for (source, output) in baseline.chunks_exact(4).zip(shown.chunks_exact(4)) {
            let expected = (f32::from(source[0]) * 0.2126
                + f32::from(source[1]) * 0.7152
                + f32::from(source[2]) * 0.0722)
                .round() as i32;
            for channel in &output[..3] {
                assert!((i32::from(*channel) - expected).abs() <= 1);
            }
            assert_eq!(output[0], output[1]);
            assert_eq!(output[1], output[2]);
            assert_eq!(output[3], 255);
        }
    }

    #[test]
    #[ignore = "requires a physical GPU; source precision must be tested by readback"]
    fn source_precision_planes_use_gpu_conversion_and_display_shader() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let grade = Arc::new(slog3(33, Interpolation::Tetrahedral));
        let mut serial = 1;
        for depth in [8, 10, 12, 16, 8] {
            for subsampling in [[2, 2], [2, 1], [1, 1], [1, 2]] {
                for full in [false, true] {
                    let (width, height) = (9usize, 7usize);
                    let scale = 1u32 << (depth - 8);
                    let luma = width * height;
                    let chroma = width.div_ceil(subsampling[0]) * height.div_ceil(subsampling[1]);
                    let mut data = Vec::new();
                    for i in 0..luma + chroma * 2 {
                        let value = if i < luma {
                            (16 + (i * 37) % 220) as u32 * scale + scale - 1
                        } else {
                            128 * scale
                        };
                        if depth == 8 {
                            data.push(value as u8);
                        } else {
                            data.extend((value as u16).to_le_bytes());
                        }
                    }
                    let frame = PackedPlanar::new(
                        crate::native_geometry::GeometryFrame {
                            width,
                            height,
                            subsampling: Some(subsampling),
                            data,
                        },
                        depth,
                        AvcColour {
                            kr: 0.2126,
                            kb: 0.0722,
                            full,
                        },
                    )
                    .unwrap();
                    let mut cpu = Vec::new();
                    frame.to_rgb(&mut cpu, width * height * 3).unwrap();
                    gpu.upload_packed(
                        &device,
                        &queue,
                        &frame,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        None,
                    )
                    .unwrap();
                    serial += 1;
                    let shown = read_picture(&device, &queue, &gpu, width, height);
                    assert!(
                        worst_between(&shown, &cpu).into_iter().all(|v| v <= 1),
                        "depth {depth} {subsampling:?} full={full}"
                    );
                    if depth > 8 {
                        let narrowed = frame.to_planar8(width * height * 3).unwrap();
                        let mut quantized = Vec::new();
                        crate::playback_native::planar8_to_rgb(
                            &narrowed,
                            &mut quantized,
                            width * height * 3,
                        )
                        .unwrap();
                        assert!(
                            cpu.iter().zip(&quantized).any(|(a, b)| a != b),
                            "test must distinguish source precision from 8-bit narrowing"
                        );
                        assert!(
                            shown
                                .chunks_exact(4)
                                .zip(quantized.chunks_exact(3))
                                .any(|(a, b)| a[..3] != *b),
                            "GPU must preserve source low bits"
                        );
                    }
                    gpu.upload_packed(
                        &device,
                        &queue,
                        &frame,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        Some(&grade),
                    )
                    .unwrap();
                    serial += 1;
                    let graded = read_picture(&device, &queue, &gpu, width, height);
                    grade.apply(&mut cpu);
                    assert!(
                        worst_between(&graded, &cpu).into_iter().all(|v| v <= 2),
                        "graded depth {depth}"
                    );
                    let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3(0.2,0.4,0.6); }").unwrap();
                    let mut custom = build_with_source(
                        &device,
                        wgpu::TextureFormat::Rgba8Unorm,
                        shader.compiled_source(),
                    );
                    custom
                        .upload_packed(
                            &device,
                            &queue,
                            &frame,
                            serial,
                            [0.0, 0.0, 1.0, 1.0],
                            IDENTITY_ADJUST,
                            Some(&grade),
                        )
                        .unwrap();
                    let pixels = read_picture(&device, &queue, &custom, width, height);
                    assert!(
                        pixels
                            .chunks_exact(4)
                            .all(|pixel| pixel == [51, 102, 153, 255])
                    );
                }
            }
        }
    }

    #[test]
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[ignore = "requires physical VideoToolbox and Metal"]
    fn hardware_surfaces_render_with_colour_and_custom_shader() {
        use crate::codec::config::{AvcConfig, HevcConfig};
        use crate::container::mp4::{Limits, Mp4Reader};
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::METAL,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        assert_ne!(adapter.get_info().device_type, wgpu::DeviceType::Cpu);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::TEXTURE_FORMAT_16BIT_NORM,
            ..Default::default()
        }))
        .unwrap();
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let shader = ColorShader::new("fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return vec3<f32>(0.2, 0.4, 0.6); }").unwrap();
        let mut custom = build_with_source(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            shader.compiled_source(),
        );
        let grade = Arc::new(Grade::new(
            bt709(),
            &HdrMetadata::default(),
            Settings {
                log: Some(Log::SLog3),
                size: 33,
                interpolation: Interpolation::Tetrahedral,
                ..Settings::video(DisplayTarget::sdr(100.0))
            },
            Some(Lut::Three(Lut3d::from_fn(17, |rgb| {
                [rgb[0] * rgb[0], rgb[1], 1.0 - rgb[2]]
            }))),
        ));
        assert!(grade.is_gpu_grade());
        let mut serial = 0;
        for bytes in [
            include_bytes!("../tests/fixtures/display/par-2x1.mp4").as_slice(),
            include_bytes!("../tests/fixtures/hevc/main-ipb.mp4").as_slice(),
            include_bytes!("../tests/fixtures/hevc/main10-ipb.mp4").as_slice(),
        ] {
            let mut reader =
                Mp4Reader::open(std::io::Cursor::new(bytes), Limits::default()).unwrap();
            let track = reader
                .tracks()
                .iter()
                .position(|t| t.handler == *b"vide")
                .unwrap();
            let config = reader.tracks()[track].configuration.clone();
            let count = reader.tracks()[track].samples.len();
            let mut session = if reader.tracks()[track].codec == *b"avc1" {
                let config = AvcConfig::parse(&config).unwrap();
                fvid_vt::Session::new_avc_surface(
                    &config.sps,
                    &config.pps,
                    config.length_size,
                    false,
                )
                .unwrap()
            } else {
                let config = HevcConfig::parse(&config).unwrap();
                let sets = |kind| {
                    config
                        .arrays
                        .iter()
                        .filter(|a| a.nal_type == kind)
                        .flat_map(|a| a.units.iter().copied())
                        .collect::<Vec<_>>()
                };
                fvid_vt::Session::new_hevc_surface(
                    &sets(32),
                    &sets(33),
                    &sets(34),
                    config.length_size,
                    config.bit_depth_luma,
                    false,
                )
                .unwrap()
            };
            let mut packet = Vec::new();
            let mut frames = 0;
            for index in 0..count {
                reader.read_packet(track, index, &mut packet).unwrap();
                let Some(surface) = session.decode_surface(&packet).unwrap() else {
                    continue;
                };
                let planes = surface.download().unwrap();
                let colour = AvcColour {
                    kr: 0.2126,
                    kb: 0.0722,
                    full: planes.full_range,
                };
                let mut data = planes.y;
                data.extend(planes.cb);
                data.extend(planes.cr);
                let packed = PackedPlanar::new(
                    crate::native_geometry::GeometryFrame {
                        width: planes.width,
                        height: planes.height,
                        subsampling: Some([2, 2]),
                        data,
                    },
                    planes.depth,
                    colour,
                )
                .unwrap();
                for rotation in [0, 90, 180, 270] {
                    let turned = packed.rotated(rotation).unwrap();
                    for grade in [None, Some(&grade)] {
                        for window in [[0.0, 0.0, 1.0, 1.0], [0.1, 0.2, 0.8, 0.9]] {
                            for adjust in [IDENTITY_ADJUST, [1.1, 2.0, 0.9, 0.8, 0.1]] {
                                serial += 1;
                                gpu.upload_packed(
                                    &device, &queue, &turned, serial, window, adjust, grade,
                                )
                                .unwrap();
                                let reference = read_picture(
                                    &device,
                                    &queue,
                                    &gpu,
                                    turned.frame.width,
                                    turned.frame.height,
                                );
                                // Same serial intentionally exercises changing storage layout.
                                gpu.import_surface_rotated(
                                    &device, &queue, &surface, colour, serial, window, adjust,
                                    grade, rotation,
                                )
                                .unwrap();
                                let shown = read_picture(
                                    &device,
                                    &queue,
                                    &gpu,
                                    turned.frame.width,
                                    turned.frame.height,
                                );
                                assert!(
                                    shown
                                        .iter()
                                        .zip(&reference)
                                        .all(|(a, b)| a.abs_diff(*b) <= 1),
                                    "native surface conversion differs: depth {}, frame {index}, rotation {rotation}, crop {window:?}, adjust {adjust:?}, graded {}, max {}",
                                    planes.depth,
                                    grade.is_some(),
                                    shown
                                        .iter()
                                        .zip(&reference)
                                        .map(|(a, b)| a.abs_diff(*b))
                                        .max()
                                        .unwrap()
                                );
                                gpu.upload_packed(
                                    &device, &queue, &turned, serial, window, adjust, grade,
                                )
                                .unwrap();
                                assert_eq!(
                                    read_picture(
                                        &device,
                                        &queue,
                                        &gpu,
                                        turned.frame.width,
                                        turned.frame.height
                                    ),
                                    reference
                                );
                            }
                        }
                    }
                }
                serial += 1;
                custom
                    .import_surface(
                        &device,
                        &queue,
                        &surface,
                        colour,
                        serial,
                        [0.0, 0.0, 1.0, 1.0],
                        IDENTITY_ADJUST,
                        None,
                    )
                    .unwrap();
                assert!(
                    read_picture(&device, &queue, &custom, planes.width, planes.height)
                        .chunks_exact(4)
                        .all(|p| p == [51, 102, 153, 255])
                );
                frames += 1;
            }
            assert!(frames >= 10);
        }
    }
    /// A device with no window, so a test can own the texture it paints into.
    /// A machine that reports no adapter has nothing here to measure against.
    fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
        headless_features(wgpu::Features::empty())
    }
    fn headless_features(features: wgpu::Features) -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        }))
        .ok()?;
        Some((device, queue))
    }

    /// The codes one frame's draw leaves behind, on a pipeline of its own.
    fn painted(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
    ) -> Vec<u8> {
        paint_adjust(device, queue, frame, grade, IDENTITY_ADJUST)
    }

    /// …and the same draw with a settings bundle on it, for a test that has to
    /// set a slider's maths against the grade's.
    fn paint_adjust(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
        adjust: [f32; 5],
    ) -> Vec<u8> {
        let mut gpu = build(device, wgpu::TextureFormat::Rgba8Unorm);
        show(device, queue, &mut gpu, frame, grade, adjust)
    }

    /// One draw of `frame` as `gpu` holds it: the product's own `upload` filling
    /// the parameter buffer, the product's own pipeline, into a plain 8-bit
    /// target so the shader's `srgb` branch stays off and the bytes it writes are
    /// the coded RGB the CPU conversion also hands back. The same `gpu` drawn
    /// twice is how a grade changed over a picture that has not moved looks.
    fn show(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &mut VideoGpu,
        frame: &Planar8,
        grade: Option<&Arc<Grade>>,
        adjust: [f32; 5],
    ) -> Vec<u8> {
        let (width, height) = (frame.width, frame.height);
        gpu.upload(device, queue, frame, 0, [0.0, 0.0, 1.0, 1.0], adjust, grade);
        read_picture(device, queue, gpu, width, height)
    }
    fn read_picture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        gpu: &VideoGpu,
        width: usize,
        height: usize,
    ) -> Vec<u8> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fvid test picture"),
            size: wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = gpu
            .bind_group
            .as_ref()
            .expect("an upload binds the planes and the grid");
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fvid test picture"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fvid test picture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&gpu.pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.draw(0..6, 0..1);
        }
        queue.submit([encoder.finish()]);
        let stride = (width * 4).next_multiple_of(256) as u64;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fvid test readback"),
            size: stride * height as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fvid test readback"),
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride as u32),
                    rows_per_image: Some(height as u32),
                },
            },
            wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([encoder.finish()]);
        let (done, waited) = std::sync::mpsc::channel();
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = done.send(result);
            });
        device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            })
            .expect("the readback must finish");
        waited
            .recv()
            .expect("the mapping callback must run")
            .expect("the buffer must map");
        let mut out = vec![0u8; width * height * 4];
        {
            let mapped = readback.slice(..).get_mapped_range().expect("mapped");
            for row in 0..height {
                let from = row * stride as usize;
                let to = (row + 1) * stride as usize;
                let keep = row * width * 4;
                out[keep..keep + width * 4].copy_from_slice(&mapped[from..to][..width * 4]);
            }
        }
        readback.unmap();
        out
    }

    /// A 64 by 64 frame whose luma walks the whole code range, so the picture
    /// holds samples the limited-range mapping clamps at both ends.
    fn frame(cb: impl Fn(usize, usize) -> u8, cr: impl Fn(usize, usize) -> u8) -> Planar8 {
        let (width, height) = (64, 64);
        let y: Vec<u8> = (0..width * height).map(|i| (i % 256) as u8).collect();
        let mut cb_plane = Vec::new();
        let mut cr_plane = Vec::new();
        for row in 0..height / 2 {
            for column in 0..width / 2 {
                cb_plane.push(cb(column, row));
                cr_plane.push(cr(column, row));
            }
        }
        Planar8 {
            width,
            height,
            chroma_width: width / 2,
            chroma_height: height / 2,
            y,
            cb: cb_plane,
            cr: cr_plane,
            colour: crate::playback_native::AvcColour::default(),
        }
    }

    /// The two plane routes measured against each other on a picture whose
    /// chroma is flat: a field with no gradient has nothing for the sampler to
    /// place differently, so the shader and the CPU conversion are left doing
    /// the same arithmetic and have to land on the same bytes. Four corners of
    /// the chroma square stand in for the neutral one, because a chroma of 128
    /// zeroes the very gains a matrix is made of — and a flat field cannot hide
    /// a wrong one: writing `2.4` where the shader scales Cr by `2.0` moves
    /// these bytes and fails the test, while leaving a neutral field alone.
    #[test]
    fn a_frame_with_flat_chroma_pays_the_shader_the_cpus_bytes() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
            let frame = frame(move |_, _| blue, move |_, _| red);
            let painted = painted(&device, &queue, &frame, None);
            let mut cpu = Vec::new();
            crate::playback_native::planar8_to_rgb(&frame, &mut cpu, 64 * 64 * 3)
                .expect("converted");
            for (index, (shown, other)) in painted
                .as_chunks::<4>()
                .0
                .iter()
                .zip(cpu.as_chunks::<3>().0)
                .enumerate()
            {
                for channel in 0..3 {
                    assert_eq!(
                        shown[channel], other[channel],
                        "{blue}/{red} pixel {index} channel {channel}: {shown:?} against {other:?}"
                    );
                }
            }
        }
    }

    /// Where the two routes are known to part, and by how much: the sampler
    /// sits a chroma sample half a luma column and half a row away from where
    /// the CPU applies it, so on a ramp that steps one code per sample the
    /// worst weight error is three quarters of a chroma code. One of those
    /// moves red by 1.80 RGB codes, so the whole difference is predicted to
    /// stay under 1.35 and two codes leave a driver's rounding somewhere to
    /// live; measured here, `[1, 1, 0]` — blue never moves, because its
    /// component is flat in this frame. The bound is what the interpolation
    /// costs, not what the matrix is worth: writing `2.4` for the `2.0` that
    /// scales Cr shows up here as `[11, 6, 0]`.
    #[test]
    fn a_chroma_ramp_divides_the_routes_by_its_interpolation() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let frame = frame(|_, _| 128, |i, _| 96 + i as u8);
        let painted = painted(&device, &queue, &frame, None);
        let mut cpu = Vec::new();
        crate::playback_native::planar8_to_rgb(&frame, &mut cpu, 64 * 64 * 3).expect("converted");
        let mut worst = [0u32; 3];
        for (pair, other) in painted
            .as_chunks::<4>()
            .0
            .iter()
            .zip(cpu.as_chunks::<3>().0)
        {
            for channel in 0..3 {
                let delta = pair[channel] as i32 - other[channel] as i32;
                worst[channel] = worst[channel].max(delta.unsigned_abs());
            }
        }
        assert!(
            worst.iter().all(|d| *d <= 2),
            "chroma interpolation moved the routes by {worst:?}"
        );
    }

    /// A plain BT.709 picture as its file states it.
    fn bt709() -> ColourDescription {
        ColourDescription {
            primaries: 1,
            transfer: 1,
            matrix: 1,
            full_range: false,
        }
    }

    /// A grade that reads the grid for every channel of every pixel: a 1 000-nit
    /// PQ master with highlights rolled onto a 100-nit panel. Its plan moves
    /// between primaries, so no set of byte tables can stand in for it.
    fn hdr10(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(
            ColourDescription {
                transfer: 16,
                primaries: 9,
                ..bt709()
            },
            &HdrMetadata {
                light: ContentLight {
                    max_cll: 1_000.0,
                    max_fall: 400.0,
                },
                ..Default::default()
            },
            settings,
            None,
        )
    }

    /// A grade that separates into byte tables: gamma-2.2 codes over the same
    /// primaries the panel shows, so one curve per channel is the whole plan and
    /// the CPU paints from 768 bytes rather than a grid. The shader has to read
    /// those exact bytes, which is what makes a byte-for-byte match between the
    /// two routes worth having here.
    fn gamma(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(
            ColourDescription {
                transfer: 4,
                ..bt709()
            },
            &HdrMetadata::default(),
            settings,
            None,
        )
    }

    /// A camera-log grade: its plan names no tone map and moves no primaries, so
    /// a cheap look at the plan says it separates into byte tables. It does not —
    /// the curve's `to_linear` is negative below the toe, and the gamut compress
    /// desaturates a negative channel toward luma — so `Grade::new` measures that
    /// and leaves the grade on the grid, which is the route the shader takes.
    fn slog3(size: usize, interpolation: Interpolation) -> Grade {
        let mut settings = Settings::video(DisplayTarget::sdr(100.0));
        settings.log = Some(Log::SLog3);
        settings.size = size;
        settings.interpolation = interpolation;
        Grade::new(bt709(), &HdrMetadata::default(), settings, None)
    }

    /// The bytes the CPU route leaves for the window: planes to packed codes,
    /// then the grade applied to them, which is what makes a plane picture
    /// packed at all.
    fn cpu_graded(frame: &Planar8, grade: &Grade) -> Vec<u8> {
        let mut rgb = Vec::new();
        crate::playback_native::planar8_to_rgb(frame, &mut rgb, frame.width * frame.height * 3)
            .expect("converted");
        grade.apply(&mut rgb);
        rgb
    }

    /// The whole frame's worst move per channel between a picture the shader
    /// graded and the same picture the CPU graded.
    fn worst_between(shader: &[u8], cpu: &[u8]) -> [u32; 3] {
        let mut worst = [0u32; 3];
        for (shown, other) in shader.as_chunks::<4>().0.iter().zip(cpu.as_chunks::<3>().0) {
            for channel in 0..3 {
                let delta = shown[channel] as i32 - other[channel] as i32;
                worst[channel] = worst[channel].max(delta.unsigned_abs());
            }
        }
        worst
    }

    /// The two grading routes on the same table: what a plane picture looks like
    /// when the fragment shader looks its codes up in the baked table, against
    /// what it looks like when the CPU converts it to packed RGB and grades that.
    ///
    /// They have to land on the same bytes, because the choice between them is
    /// made frame by frame by whatever else the window needs — a snapshot takes
    /// the CPU route while the picture on screen takes the shader. Both read the
    /// identical table: the shader is handed whichever of the two the grade itself
    /// reads, a grid's nodes or its byte tables, and the codes it feeds it are the
    /// bytes the CPU indexes by, which is what the `round(rgb · 255)` in front of
    /// the lookup is for. Nothing in the two routes is an approximation of the
    /// other, so this says `assert_eq!` rather than a tolerance: a grid that
    /// arrived with its nodes in another order, a read mode the uniform told the
    /// shader but not the CPU, a byte table row shifted by one texel, a lookup
    /// before the range maths instead of after it — each moves these bytes.
    #[test]
    fn the_shader_and_the_cpu_read_the_same_table_to_the_byte() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for interpolation in Interpolation::ALL {
            for size in [17, 33, 64] {
                // Each grade is named with the table it has to reach the shader
                // by, so the three cannot all quietly take one route and leave the
                // other binding untested.
                for (grade, kind) in [
                    (hdr10(size, interpolation), "grid"),
                    (gamma(size, interpolation), "tables"),
                    (slog3(size, interpolation), "grid"),
                ] {
                    let grade = Arc::new(grade);
                    let look = match grade.shader_look() {
                        Some(ShaderLook::Grid(_)) => "grid",
                        Some(ShaderLook::Tables(_)) => "tables",
                        None => "none",
                    };
                    assert_eq!(look, kind, "a {kind} grade reached the shader as {look}");
                    // Four corners of the chroma square rather than the neutral
                    // one, which is what a wrong matrix hides; see the flat
                    // chroma test above.
                    for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
                        let frame = frame(move |_, _| blue, move |_, _| red);
                        let shader = painted(&device, &queue, &frame, Some(&grade));
                        let cpu = cpu_graded(&frame, &grade);
                        let worst = worst_between(&shader, &cpu);
                        assert_eq!(
                            worst,
                            [0, 0, 0],
                            "{interpolation:?} at a {size} {kind}, chroma {blue}/{red}: the \
                             routes parted by {worst:?}"
                        );
                    }
                }
            }
        }
    }

    /// The table a texture mirrors changes while the picture does not: a caller
    /// moves `--grid` or names another look over a frame that is held on screen.
    /// One pipeline drawn twice is what the window shows then, and the second
    /// answer has to be the second grade — not the first one still sitting in the
    /// texture, nor in a bind group that points at it. A grid of the same edge
    /// length is written into the texture already bound, a different length is a
    /// new texture and a new bind group, a grade that reads bytes rather than
    /// nodes leaves the grid behind altogether, and no grade at all is the switch
    /// turned off, so every way out of here is walked.
    #[test]
    fn a_grade_changed_over_a_held_picture_moves_the_bound_table() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let frame = frame(|_, _| 64, |_, _| 224);
        let steps: [(String, Grade); 7] = [
            (
                "trilinear 33 grid".into(),
                hdr10(33, Interpolation::Trilinear),
            ),
            // Same edge length, other read: the table stays, the uniform moves.
            (
                "tetrahedral 33 grid".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
            (
                "tetrahedral 17 grid".into(),
                hdr10(17, Interpolation::Tetrahedral),
            ),
            // Back to a length a texture of this one has had before, which is a
            // new texture all the same.
            (
                "tetrahedral 33 grid again".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
            ("nearest 64 grid".into(), hdr10(64, Interpolation::Nearest)),
            // A grade with no grid at all: the row of bytes replaces the 64 cubed
            // texture, and the switch names the other binding.
            ("byte tables".into(), gamma(33, Interpolation::Tetrahedral)),
            (
                "tetrahedral 33 grid after bytes".into(),
                hdr10(33, Interpolation::Tetrahedral),
            ),
        ];
        for (index, (what, grade)) in steps.iter().enumerate() {
            let grade = Arc::new(grade.clone());
            let shown = show(
                &device,
                &queue,
                &mut gpu,
                &frame,
                Some(&grade),
                IDENTITY_ADJUST,
            );
            let worst = worst_between(&shown, &cpu_graded(&frame, &grade));
            assert_eq!(worst, [0, 0, 0], "step {index}, {what}: {worst:?}");
        }
        let shown = show(&device, &queue, &mut gpu, &frame, None, IDENTITY_ADJUST);
        let mut plain = Vec::new();
        crate::playback_native::planar8_to_rgb(&frame, &mut plain, 64 * 64 * 3).expect("converted");
        assert_eq!(
            worst_between(&shown, &plain),
            [0, 0, 0],
            "the last grade left the picture after the lookup was turned off"
        );
    }

    /// The colour half of the two routes, with the conversion taken out of it:
    /// the shader's own ungraded bytes are graded on the CPU and set against the
    /// same picture the shader graded, over a chroma ramp that reaches every
    /// code.
    ///
    /// Comparing a graded shader against a graded CPU leaves the planar→RGB step
    /// in the number too, and the two routes are up to two codes apart there
    /// where the chroma ramp crosses them (measured by the chroma ramp test
    /// above). A steep curve multiplies a code of input into tens of output —
    /// the same three grades set against the whole CPU route were measured part
    /// by up to fifty-six codes, every one of them that multiplication and not a
    /// second colour decision. This test isolates the colour, and it agrees byte
    /// for byte at every code the ramp reaches, for a grid read three ways and
    /// for the byte tables.
    #[test]
    fn the_shader_grades_its_own_codes_as_the_cpu_would() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        for interpolation in Interpolation::ALL {
            for size in [17, 33, 64] {
                for grade in [
                    hdr10(size, interpolation),
                    gamma(size, interpolation),
                    slog3(size, interpolation),
                ] {
                    let grade = Arc::new(grade);
                    let frame = frame(|_, _| 128, |i, _| 96 + i as u8);
                    let plain = painted(&device, &queue, &frame, None);
                    let mut want: Vec<u8> = plain
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|pixel| [pixel[0], pixel[1], pixel[2]])
                        .collect();
                    grade.apply(&mut want);
                    let shown = painted(&device, &queue, &frame, Some(&grade));
                    let worst = worst_between(&shown, &want);
                    assert_eq!(
                        worst,
                        [0, 0, 0],
                        "{interpolation:?} at a {size} grid over a chroma ramp: {worst:?}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "requires a physical GPU for float 1D LUT qualification"]
    fn sequential_1d_grading_preserves_float_nodes_and_long_tables() {
        use crate::color::lut::Lut1d;
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let picture = frame(|_, _| 128, |i, _| (16 + i % 220) as u8);
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let mut serial = 1;
        for interpolation in Interpolation::ALL {
            for size in [1, 2, 5, 128, 129, 4096, 16385, 300_000] {
                let data = std::array::from_fn(|channel| {
                    (0..size)
                        .map(|i| {
                            let x = if size == 1 {
                                0.25
                            } else {
                                i as f32 / (size - 1) as f32
                            };
                            match channel {
                                0 => x * x,
                                1 => x * 1.2 - 0.1,
                                _ => 1.0 - x,
                            }
                        })
                        .collect()
                });
                let line = Lut1d {
                    data,
                    domain_min: [-0.2, 0.1, -0.5],
                    domain_max: [1.2, 0.9, 1.5],
                };
                let grade = Arc::new(Grade::new(
                    bt709(),
                    &HdrMetadata::default(),
                    Settings {
                        log: Some(Log::SLog3),
                        gamut: Some(crate::color::Primaries::BT2020),
                        size: 17,
                        interpolation,
                        ..Settings::video(DisplayTarget::sdr(100.0))
                    },
                    Some(Lut::One(line)),
                ));
                assert!(grade.is_gpu_grade());
                assert!(grade.shader_look().is_none());
                gpu.upload(
                    &device,
                    &queue,
                    &picture,
                    serial,
                    [0.0, 0.0, 1.0, 1.0],
                    IDENTITY_ADJUST,
                    None,
                );
                let plain = read_picture(&device, &queue, &gpu, 64, 64);
                let mut expected: Vec<u8> = plain
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect();
                grade.apply(&mut expected);
                gpu.upload(
                    &device,
                    &queue,
                    &picture,
                    serial,
                    [0.0, 0.0, 1.0, 1.0],
                    IDENTITY_ADJUST,
                    Some(&grade),
                );
                let shown = read_picture(&device, &queue, &gpu, 64, 64);
                let worst = worst_between(&shown, &expected);
                assert!(
                    worst.iter().all(|&v| v <= 1),
                    "{interpolation:?}, {size}: {worst:?}"
                );
                assert_ne!(plain, shown);
                serial += 1;
            }
        }
    }

    #[test]
    #[ignore = "requires a physical GPU for sequential LUT qualification"]
    fn sequential_3d_grading_preserves_authored_domains() {
        let (device, queue) = headless().expect("physical GPU required");
        assert_ne!(device.adapter_info().device_type, wgpu::DeviceType::Cpu);
        let frame = frame(|_, _| 128, |i, _| (16 + i % 220) as u8);
        for interpolation in Interpolation::ALL {
            for size in [5, 17] {
                let mut settings = Settings::video(DisplayTarget::sdr(100.0));
                settings.size = 17;
                settings.interpolation = interpolation;
                settings.log = Some(Log::SLog3);
                let mut look = Lut3d::from_fn(size, |rgb| {
                    [rgb[0] * rgb[0], rgb[1] * 1.2 - 0.1, 1.0 - rgb[2]]
                });
                look.domain_min = [-0.2, 0.1, -0.5];
                look.domain_max = [1.2, 0.9, 1.5];
                let grade = Arc::new(Grade::new(
                    bt709(),
                    &HdrMetadata::default(),
                    settings,
                    Some(Lut::Three(look)),
                ));
                assert!(grade.is_gpu_grade());
                assert!(grade.shader_look().is_none());
                let plain = painted(&device, &queue, &frame, None);
                let mut expected: Vec<u8> = plain
                    .chunks_exact(4)
                    .flat_map(|p| p[..3].iter().copied())
                    .collect();
                grade.apply(&mut expected);
                let shown = painted(&device, &queue, &frame, Some(&grade));
                let worst = worst_between(&shown, &expected);
                assert!(
                    worst.iter().all(|&v| v <= 1),
                    "{interpolation:?}, {size}: {worst:?}"
                );
                assert_ne!(shown, plain);
            }
        }
    }

    /// The bytes the CPU route leaves for the window when a picture is graded
    /// and then stepped through VLC's settings: converted, graded, the bundle
    /// over the graded codes, which is the order the draw path has always used.
    fn cpu_graded_and_adjusted(frame: &Planar8, grade: &Grade, bundle: &[f32; 5]) -> Vec<u8> {
        let rgb = cpu_graded(frame, grade);
        if *bundle == IDENTITY_ADJUST {
            return rgb;
        }
        let mut adjusted = Vec::new();
        crate::player::adjust_rgb(&rgb, &mut adjusted, bundle);
        adjusted
    }

    /// VLC's picture settings and a grade, both asked for at once: the order the
    /// window shows them in, and the space the sliders work in.
    ///
    /// The grade is the colour management, so it runs first and the sliders are
    /// stepped over the codes it leaves — the picture the viewer is looking at,
    /// which is the only picture a brightness slider can promise to brighten.
    /// Contrast and gamma over log codes before the curve unwraps them would be
    /// swallowed or doubled by the curve instead, and the CPU draw path has
    /// always graded first, so the shader that keeps a plane picture's planes
    /// has to follow it here. Every step of both routes is the same arithmetic
    /// on the same bytes, including the byte the lookup lands on before the
    /// sliders run, so this is an `assert_eq!` on the whole frame rather than a
    /// tolerance: a slider applied before the grade, a turn that reads its own
    /// answer back, a grid's float answer fed to the matrix instead of its byte,
    /// or the identity bundle stepped through the round trip the CPU skips, each
    /// moves these codes.
    #[test]
    fn a_grade_and_a_slider_reach_the_window_in_the_cpus_order() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        // Every one of the five numbers away from the identity, so no term of
        // either half of the bundle is left unexercised: contrast, brightness
        // through the lum offset, gamma, saturation and a hue turn — and the
        // same bundle turned over, which is the other half of the turn's maths.
        for bundle in [
            [1.3, 14.0, 1.0 / 1.25, 0.6, 0.35],
            [0.75, -22.0, 1.0 / 0.8, 1.7, -0.5],
            IDENTITY_ADJUST,
        ] {
            for interpolation in Interpolation::ALL {
                // One grade read from the grid, one from the byte tables, one
                // whose plan a cheap look calls separable and the measurement
                // sends back to the grid.
                for grade in [
                    hdr10(33, interpolation),
                    gamma(33, interpolation),
                    slog3(33, interpolation),
                ] {
                    let grade = Arc::new(grade);
                    for (blue, red) in [(128u8, 128u8), (100, 180), (64, 224), (192, 32)] {
                        let frame = frame(move |_, _| blue, move |_, _| red);
                        let shader = paint_adjust(&device, &queue, &frame, Some(&grade), bundle);
                        let worst = worst_between(
                            &shader,
                            &cpu_graded_and_adjusted(&frame, &grade, &bundle),
                        );
                        assert_eq!(
                            worst,
                            [0, 0, 0],
                            "bundle {bundle:?}, {interpolation:?}, chroma {blue}/{red}: the \
                             routes parted by {worst:?}"
                        );
                    }
                }
            }
        }
    }

    /// A slider moved over a picture the window is holding, with a grade on it:
    /// the frame, its planes and its table all stay, so the only thing that
    /// changes is the parameter block, and the switch that turns the bundle's
    /// round trip on rides in it. Drawn twice over one pipeline, the second
    /// answer has to be the second bundle's and the third the first one's again,
    /// because a stale switch would leave the picture either graded without the
    /// slider the viewer just moved, or stepped through the matrix when nothing
    /// asked for it.
    #[test]
    fn a_slider_moved_over_a_held_graded_picture_reaches_the_shader() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let mut gpu = build(&device, wgpu::TextureFormat::Rgba8Unorm);
        let frame = frame(|_, _| 64, |_, _| 224);
        let grade = Arc::new(hdr10(33, Interpolation::Tetrahedral));
        let bundle = [1.25, 10.0, 1.0 / 1.2, 0.8, -0.3];
        for step in [IDENTITY_ADJUST, bundle, IDENTITY_ADJUST] {
            let shown = show(&device, &queue, &mut gpu, &frame, Some(&grade), step);
            let worst = worst_between(&shown, &cpu_graded_and_adjusted(&frame, &grade, &step));
            assert_eq!(
                worst,
                [0, 0, 0],
                "step {step:?} over a held picture: {worst:?}"
            );
        }
    }

    /// The same bundle with no grade in front of it, which is the picture
    /// settings' own parity between the routes: the shader turns the planes it
    /// was given, the CPU solves BT.601 back out of the bytes the converter
    /// stored and turns that. Where both routes' samples survive the conversion
    /// the two are the same maths with one stored byte in the middle, so what
    /// separates them is what that rounding costs at either end — measured here,
    /// on four corners of the chroma square and a turn that scales no channel
    /// down, because a wrong turn cannot hide at a step or two.
    ///
    /// The picture is deliberately mild at both ends: a sample outside the coded
    /// range makes the routes incomparable rather than disagreeing, because the
    /// converter clamps it and the packed route then adjusts a luma the file
    /// never carried. The last two frames are exactly that, kept in the test as
    /// the record of it — thirty-two codes for a luma of 0 under a contrast of
    /// 1.3, and fifty-three for chroma at the top of the range — every one of
    /// them the shader's stored sample against the black or white the converter
    /// left behind, which no maths recovers afterwards.
    #[test]
    fn a_slider_over_a_plain_picture_costs_a_rounding_step() {
        let Some((device, queue)) = headless() else {
            eprintln!("no GPU adapter: the shader has nothing to be compared with");
            return;
        };
        let plains = |blue: u8, red: u8, luma: &dyn Fn(usize) -> u8| {
            let (width, height) = (64, 64);
            Planar8 {
                width,
                height,
                chroma_width: width / 2,
                chroma_height: height / 2,
                y: (0..width * height).map(luma).collect(),
                cb: vec![blue; width * height / 4],
                cr: vec![red; width * height / 4],
                colour: crate::playback_native::AvcColour::default(),
            }
        };
        let within = |blue, red| plains(blue, red, &|i| (60 + i % 91) as u8);
        for bundle in [
            [1.15, 8.0, 1.0 / 1.1, 0.7, 0.4],
            [1.0, 0.0, 1.0, 0.0, 1.0],
            [1.0, 0.0, 1.0, -1.0, 0.0],
            IDENTITY_ADJUST,
        ] {
            for (blue, red) in [(128u8, 128u8), (104, 152), (152, 104), (104, 104)] {
                let frame = within(blue, red);
                let shader = paint_adjust(&device, &queue, &frame, None, bundle);
                let mut rgb = Vec::new();
                crate::playback_native::planar8_to_rgb(&frame, &mut rgb, 64 * 64 * 3)
                    .expect("converted");
                let mut cpu = Vec::new();
                crate::player::adjust_rgb(&rgb, &mut cpu, &bundle);
                let worst = worst_between(&shader, &cpu);
                assert!(
                    worst.iter().all(|d| *d <= 2),
                    "bundle {bundle:?} over a {blue}/{red} chroma: the routes parted by {worst:?}"
                );
            }
        }
        // Where the converter does clamp, the routes are not comparable and this
        // is the record of that rather than a tolerance for it: the plane route
        // has the stored sample, the packed route has the black or white the
        // converter left instead, and no maths afterwards recovers it.
        for (what, frame, bundle) in [
            (
                "luma below black",
                plains(128, 128, &|i| (i % 256) as u8),
                [1.3, 14.0, 1.0 / 1.25, 1.0, 0.0],
            ),
            (
                "chroma at the top of the range",
                plains(64, 224, &|i| (60 + i % 91) as u8),
                [1.0, 0.0, 1.0, 0.7, 0.4],
            ),
        ] {
            let shader = paint_adjust(&device, &queue, &frame, None, bundle);
            let mut rgb = Vec::new();
            crate::playback_native::planar8_to_rgb(&frame, &mut rgb, 64 * 64 * 3)
                .expect("converted");
            let mut cpu = Vec::new();
            crate::player::adjust_rgb(&rgb, &mut cpu, &bundle);
            let worst = worst_between(&shader, &cpu);
            assert!(
                worst.iter().any(|d| *d > 20),
                "{what}: the clamped route was expected to part by tens, {worst:?}"
            );
        }
    }
}
