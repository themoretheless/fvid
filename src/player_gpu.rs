//! Video presentation on the GPU: the decoded Y, Cb and Cr planes are uploaded
//! as 8-bit textures and converted to RGB in a fragment shader (Metal on
//! macOS through wgpu), so no CPU pass touches the pixels after decoding.
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
};
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_cb: texture_2d<f32>;
@group(0) @binding(2) var plane_cr: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> params: Params;

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
    bind_group: wgpu::BindGroup,
}

/// Create the pipeline once and register it with the renderer.
pub fn install(state: &egui_wgpu::RenderState) {
    let device = &state.device;
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
                format: state.target_format,
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
        // window and the two setting vectors: 80 bytes laid out as the
        // shader's Params declares.
        size: 80,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    state.renderer.write().callback_resources.insert(VideoGpu {
        pipeline,
        layout,
        sampler,
        uniform,
        srgb: state.target_format.is_srgb(),
        planes: None,
        serial: u64::MAX,
        window: [0.0; 4],
        adjust: IDENTITY_ADJUST,
    });
}

impl VideoGpu {
    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
    ) {
        if self.serial == serial && self.window == window && self.adjust == adjust {
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
        // other, the rest of the space padding to the vector width.
        let params: [f32; 20] = [
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
}
