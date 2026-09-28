//! Video presentation on the GPU: the decoded Y, Cb and Cr planes are uploaded
//! as 8-bit textures and converted to RGB in a fragment shader (Metal on
//! macOS through wgpu), so no CPU pass touches the pixels after decoding.
use crate::color::{Interpolation, Lut3d};
use crate::playback_native::Planar8;
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
    _pad: f32,
    // The part of the planes a crop keeps, as texture coordinates: the corner
    // it starts at, then the corner it ends at. The whole picture is (0,0)-(1,1).
    window: vec4<f32>,
    // VLC's picture settings as the five numbers its adjust filter computes:
    // the luma becomes gamma⁻¹(clip(lum + contrast·y)) on stored samples and
    // the chroma pair rotates by the hue and scales by the saturation around
    // grey. The identity bundle — 1, 0, 1, 1, 0 — leaves both as they stand.
    adjust_a: vec4<f32>, // contrast, lum, inverse gamma, cos·saturation
    adjust_b: vec4<f32>, // sin·saturation, then spare
    // The grade the CPU baked: one 3D grid of display-referred codes. `grid.x`
    // is the grid's edge length, `grid.y` how to read between its nodes — 0 for
    // nearest, 1 for trilinear, 2 for tetrahedral — and `grid.w` turns the
    // lookup off, so a frame with nothing to grade pays nothing for the binding.
    grid: vec4<f32>,
};
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_cb: texture_2d<f32>;
@group(0) @binding(2) var plane_cr: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> params: Params;
// The grade's nodes, one texel per node: the table lists red fastest, so node
// (r, g, b) is texel (r, g, b) and the walk below is `Lut3d::sample`'s.
@group(0) @binding(5) var grade_grid: texture_3d<f32>;

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
    out.uv = mix(params.window.xy, params.window.zw, t);
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}

fn grid_node(at: vec3<i32>) -> vec3<f32> {
    return textureLoad(grade_grid, at, 0).rgb;
}

fn along_axis(axis: i32) -> vec3<i32> {
    if axis == 0 {
        return vec3(1, 0, 0);
    }
    if axis == 1 {
        return vec3(0, 1, 0);
    }
    return vec3(0, 0, 1);
}

/// The offset's axis with the largest value, lowest axis winning a tie, over the
/// axes `used` has not taken yet. A walk over three with a strict `>` is the
/// stable sort the CPU's tetrahedral read performs.
fn tallest_axis(d: vec3<f32>, used: vec3<i32>) -> i32 {
    var best = 0i32;
    var best_value = -1.0e30;
    for (var i = 0i32; i < 3i32; i = i + 1) {
        if used[i] == 0 && d[i] > best_value {
            best_value = d[i];
            best = i;
        }
    }
    return best;
}

/// Read the baked grid, the way `Lut3d::sample` reads the same table: the input
/// is a display code in [0, 1], the grid's domain is [0, 1] because the plan
/// baked it, and the three modes are the CPU's three — a snap to the nearest
/// node, a box blend of eight, and a walk of four nodes along the ordered
/// offsets.
fn grade_read(rgb: vec3<f32>) -> vec3<f32> {
    let last = vec3<i32>(i32(params.grid.x) - 1);
    let p = rgb * vec3(f32(last.x));
    var offsets = p - vec3<f32>(vec3<i32>(floor(p)));
    let origin = vec3<i32>(floor(p));
    if params.grid.y < 0.5 {
        let near = select(vec3<i32>(0), vec3<i32>(1), offsets > vec3(0.5));
        return grid_node(min(origin + near, last));
    }
    if params.grid.y < 1.5 {
        let r1 = min(origin + vec3(1, 0, 0), last);
        let g1 = min(origin + vec3(0, 1, 0), last);
        let b1 = min(origin + vec3(0, 0, 1), last);
        let c000 = grid_node(origin);
        let c100 = grid_node(r1);
        let c010 = grid_node(g1);
        let c110 = grid_node(min(origin + vec3(1, 1, 0), last));
        let c001 = grid_node(b1);
        let c101 = grid_node(min(r1 + vec3(0, 0, 1), last));
        let c011 = grid_node(min(g1 + vec3(0, 0, 1), last));
        let c111 = grid_node(min(r1 + vec3(0, 1, 1), last));
        let x00 = mix(c000, c100, vec3(offsets.x));
        let x10 = mix(c010, c110, vec3(offsets.x));
        let x01 = mix(c001, c101, vec3(offsets.x));
        let x11 = mix(c011, c111, vec3(offsets.x));
        let y0 = mix(x00, x10, vec3(offsets.y));
        let y1 = mix(x01, x11, vec3(offsets.y));
        return mix(y0, y1, vec3(offsets.z));
    }
    // Order the axes by offset and walk the four nodes of that tetrahedron:
    // origin, plus the tallest axis, plus the middle one, plus all three.
    var used = vec3<i32>(0);
    let a = tallest_axis(offsets, used);
    used[a] = 1;
    let b = tallest_axis(offsets, used);
    used[b] = 1;
    let c = tallest_axis(offsets, used);
    let e1 = along_axis(a);
    let e2 = e1 + along_axis(b);
    let n0 = grid_node(origin);
    let n1 = grid_node(min(origin + e1, last));
    let n2 = grid_node(min(origin + e2, last));
    let n3 = grid_node(min(origin + vec3(1, 1, 1), last));
    let sa = offsets[a];
    let sb = offsets[b];
    let sc = offsets[c];
    return n0 * (1.0 - sa) + n1 * (sa - sb) + n2 * (sb - sc) + n3 * sc;
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    // VLC's adjust filter works on the stored 8-bit samples, so the numbers
    // are applied here — the luma line first, clamped like its tables, then
    // its gamma curve; the chroma around grey, both before any range maths.
    let stored_y = textureSample(plane_y, plane_sampler, in.uv).r * 255.0;
    let luma = clamp(params.adjust_a.x * stored_y + params.adjust_a.y, 0.0, 255.0);
    let y_stored = pow(luma / 255.0, params.adjust_a.z) * 255.0;
    let cb_c = textureSample(plane_cb, plane_sampler, in.uv).r * 255.0 - params.c_offset;
    let cr_c = textureSample(plane_cr, plane_sampler, in.uv).r * 255.0 - params.c_offset;
    let cb_adj = params.adjust_a.w * cb_c + params.adjust_b.x * cr_c;
    let cr_adj = params.adjust_a.w * cr_c - params.adjust_b.x * cb_c;
    let y = (y_stored - params.y_offset) * params.y_gain;
    let cb = cb_adj * params.c_gain;
    let cr = cr_adj * params.c_gain;
    let r = y + 2.0 * (1.0 - params.kr) * cr;
    let b = y + 2.0 * (1.0 - params.kb) * cb;
    let g = (y - params.kr * r - params.kb * b) / (1.0 - params.kr - params.kb);
    var rgb = clamp(vec3(r, g, b), vec3(0.0), vec3(1.0));
    if params.grid.w > 0.5 {
        // The CPU route indexes its grid by the byte it stores, so land on that
        // byte first: the two routes then read the same nodes, and what is left
        // between them is arithmetic rather than a different colour decision.
        let code = round(rgb * 255.0) / 255.0;
        rgb = clamp(grade_read(code), vec3(0.0), vec3(1.0));
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

/// GPU objects shared by every frame, stored in egui's callback resources.
pub struct VideoGpu {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    srgb: bool,
    planes: Option<Planes>,
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
    textures: [wgpu::Texture; 3],
    views: [wgpu::TextureView; 3],
}

/// The 3D texture bound at `grade_grid`, and the table that filled it.
///
/// A grade with nothing to look up still needs something bound — a two-node
/// grid the shader never reads, because its `grid.w` says the lookup is off.
struct Grid {
    /// The CPU table this texture mirrors, kept so the frame after this one can
    /// be told it reads the same grade without comparing a megabyte of nodes.
    source: Option<Arc<Lut3d>>,
    interpolation: Interpolation,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Edge length, read mode and the on switch, as the uniform carries them.
    params: [f32; 4],
}

/// The two-node grid bound when a picture has nothing to grade.
const GRID_OFF: [f32; 4] = [2.0, 0.0, 0.0, 0.0];

/// The number the shader matches a read mode against: 0 snaps to a node, 1
/// blends the eight around it, 2 walks the four of a tetrahedron.
fn grid_mode(interpolation: Interpolation) -> f32 {
    match interpolation {
        Interpolation::Nearest => 0.0,
        Interpolation::Trilinear => 1.0,
        Interpolation::Tetrahedral => 2.0,
    }
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
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fvid video"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
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
        // window, the two setting vectors and the grade's grid: 96 bytes laid
        // out as the shader's Params declares.
        size: 96,
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
        grid: Grid::empty(device),
        bind_group: None,
        bound: None,
        serial: u64::MAX,
        window: [0.0; 4],
        adjust: IDENTITY_ADJUST,
    }
}

impl Grid {
    /// The grid bound while a picture has nothing to look up, with the shader's
    /// switch set so it never reads it.
    fn empty(device: &wgpu::Device) -> Self {
        let (texture, view) = make_grid(device, 2);
        Self {
            source: None,
            texture,
            view,
            size: 2,
            params: GRID_OFF,
            epoch: 0,
        }
    }

    /// Point the bound grid at `lut`, read with `interpolation`. A grid of the
    /// same edge length is written into the texture that is already there, so
    /// the bind group that holds it stays good and only a different size — a
    /// new `--grid`, say — costs a new texture and a rebinding.
    fn set(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lut: Option<&Arc<Lut3d>>,
        interpolation: Interpolation,
    ) {
        let same = match (&self.source, lut) {
            (Some(held), Some(want)) => Arc::ptr_eq(held, want),
            (None, None) => true,
            _ => false,
        };
        if same {
            return;
        }
        let size = lut.map_or(2, |cube| cube.size);
        if size != self.size {
            let (texture, view) = make_grid(device, size);
            self.texture = texture;
            self.view = view;
            self.size = size;
            self.epoch += 1;
        }
        let nodes: Vec<[f32; 3]> = lut.map_or_else(
            || Lut3d::identity(2).data,
            |cube| cube.data.clone(),
        );
        let bytes: Vec<u8> = nodes
            .iter()
            .flat_map(|node| {
                [node[0], node[1], node[2], 1.0]
                    .into_iter()
                    .flat_map(f32::to_ne_bytes)
            })
            .collect();
        let extent = wgpu::Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: size as u32,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                // One texel per node, packed: `write_texture` stages the copy
                // itself, so the grid's row needs no padding to an alignment.
                bytes_per_row: None,
                rows_per_image: None,
            },
            extent,
        );
        self.source = lut.cloned();
        self.params = if lut.is_some() {
            [size as f32, grid_mode(interpolation), 1.0, 0.0]
        } else {
            GRID_OFF
        };
    }
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

impl VideoGpu {
    #[allow(clippy::too_many_arguments)]
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Lut3d>>,
        interpolation: Interpolation,
    ) {
        let same_grade = match (&self.grid.source, grade) {
            (Some(held), Some(want)) => Arc::ptr_eq(held, want),
            (None, None) => true,
            _ => false,
        };
        if self.serial == serial && self.window == window && self.adjust == adjust && same_grade {
            return;
        }
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        let size = [frame.width, frame.height];
        let chroma = [frame.chroma_width, frame.chroma_height];
        if self
            .planes
            .as_ref()
            .is_none_or(|p| p.size != size || p.chroma != chroma)
        {
            self.planes = Some(self.create_planes(device, size, chroma));
        }
        self.grid.set(device, queue, grade, interpolation);
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
                ],
            }));
            self.bound = Some(bound);
        }
        let planes = self.planes.as_ref().unwrap();
        for (index, (data, [w, h])) in [(&frame.y, size), (&frame.cb, chroma), (&frame.cr, chroma)]
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
                    bytes_per_row: Some(w as u32),
                    rows_per_image: Some(h as u32),
                },
                wgpu::Extent3d {
                    width: w as u32,
                    height: h as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        // Limited range maps 16..235 (luma) and 16..240 (chroma) onto 0..1.
        let (y_offset, y_range, c_range) = if frame.colour.full {
            (0.0, 255.0, 255.0)
        } else {
            (16.0, 219.0, 224.0)
        };
        // The settings ride as the shader's two vectors: the luma trio with
        // the cosine of the turn in the fourth slot, the sine alone in the
        // other, the rest of the space padding to the vector width. The grid
        // vector is the texture's own shape, filled in when it was last
        // written: edge length, how to read between nodes, and whether to.
        let params: [f32; 24] = [
            y_offset,
            1.0 / y_range,
            128.0,
            1.0 / c_range,
            frame.colour.kr as f32,
            frame.colour.kb as f32,
            if self.srgb { 1.0 } else { 0.0 },
            0.0,
            window[0],
            window[1],
            window[2],
            window[3],
            adjust[0],
            adjust[1],
            adjust[2],
            adjust[3],
            adjust[4],
            0.0,
            0.0,
            0.0,
            self.grid.params[0],
            self.grid.params[1],
            self.grid.params[2],
            self.grid.params[3],
        ];
        let bytes: Vec<u8> = params.iter().flat_map(|v| v.to_le_bytes()).collect();
        queue.write_buffer(&self.uniform, 0, &bytes);
    }
    fn create_planes(&self, device: &wgpu::Device, size: [usize; 2], chroma: [usize; 2]) -> Planes {
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
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let textures = [
            make("fvid plane y", size),
            make("fvid plane cb", chroma),
            make("fvid plane cr", chroma),
        ];
        let views: Vec<wgpu::TextureView> = textures
            .iter()
            .map(|t| t.create_view(&wgpu::TextureViewDescriptor::default()))
            .collect();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
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
            ],
        });
        Planes {
            size,
            chroma,
            textures,
            bind_group,
        }
    }
}

/// Paint callback drawing one planar frame into its rect, sampling only the
/// part of it a crop leaves.
pub struct VideoCallback {
    pub frame: Arc<Planar8>,
    pub serial: u64,
    pub window: [f32; 4],
    pub adjust: [f32; 5],
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
        let Some(planes) = &gpu.planes else {
            return;
        };
        pass.set_pipeline(&gpu.pipeline);
        pass.set_bind_group(0, &planes.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            layouter[params].size, 80,
            "Params must match the 80-byte uniform the upload writes"
        );
    }

    /// A device with no window, so a test can own the texture it paints into.
    /// A machine that reports no adapter has nothing here to measure against.
    fn headless() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: None,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            ..Default::default()
        }))
        .ok()?;
        Some((device, queue))
    }

    /// The codes one frame's draw leaves behind: the product's own pipeline,
    /// the product's own `upload` filling the parameter buffer, into a plain
    /// 8-bit target so the shader's `srgb` branch stays off and the bytes it
    /// writes are the coded RGB the CPU conversion also hands back.
    fn painted(device: &wgpu::Device, queue: &wgpu::Queue, frame: &Planar8) -> Vec<u8> {
        let (width, height) = (frame.width, frame.height);
        let mut gpu = build(device, wgpu::TextureFormat::Rgba8Unorm);
        gpu.upload(
            device,
            queue,
            frame,
            0,
            [0.0, 0.0, 1.0, 1.0],
            IDENTITY_ADJUST,
        );
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
        let planes = gpu.planes.as_ref().expect("an upload makes the planes");
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
            pass.set_bind_group(0, &planes.bind_group, &[]);
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
            let painted = painted(&device, &queue, &frame);
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
        let painted = painted(&device, &queue, &frame);
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
}
