use super::{GpuViewParams, IDENTITY_ADJUST, IDENTITY_VIEW, UNIFORM_BYTES, lut3d::Grid, shader::SHADER};
use crate::color::Grade;
use crate::playback_native::{AvcColour, PackedPlanar, Planar8};
use eframe::{egui_wgpu, wgpu};
use std::sync::Arc;

/// GPU objects shared by every frame, stored in egui's callback resources.
pub struct VideoGpu {
    pub(super) pipeline: wgpu::RenderPipeline,
    pub(super) layout: wgpu::BindGroupLayout,
    pub(super) sampler: wgpu::Sampler,
    pub(super) uniform: wgpu::Buffer,
    pub(super) srgb: bool,
    pub(super) planes: Option<Planes>,
    pub(super) surface_mode: f32,
    pub(super) rgb_source: Option<Arc<Vec<u8>>>,
    pub(super) rotation: u16,
    /// The grade the shader reads, which travels with the bind group below.
    pub(super) grid: Grid,
    pub(super) bind_group: Option<wgpu::BindGroup>,
    /// The plane geometry and grade the current bind group was built for: a
    /// change to either needs a new one, and nothing else does.
    pub(super) bound: Option<([usize; 2], [usize; 2], usize)>,
    pub(super) imported_surface: bool,
    pub(super) serial: u64,
    /// The crop the uniform currently holds, kept so a crop changed while the
    /// same frame stays on screen still reaches the shader.
    pub(super) window: [f32; 4],
    /// The settings bundle the uniform currently holds, kept for the same
    /// reason: a slider moved over a held frame reaches the shader too.
    pub(super) adjust: [f32; 5],
    /// Stereo3D and 360° spherical viewport parameters currently in the uniform.
    pub(super) view: GpuViewParams,
    /// Reusable RGBA8 expansion scratch buffer for 3-channel RGB uploads.
    pub(super) rgba_scratch: Vec<u8>,
}
pub(super) struct Planes {
    pub(super) size: [usize; 2],
    pub(super) chroma: [usize; 2],
    pub(super) depth: u8,
    pub(super) textures: [wgpu::Texture; 3],
    pub(super) views: [wgpu::TextureView; 3],
}

/// The objects one video draw needs for one target format: the pipeline, its
/// bind group layout, the sampler that magnifies the picture, and the parameter
/// buffer [`upload`](VideoGpu::upload) fills. `install` asks for the window's
/// format; a test asks for a plain 8-bit one so the picture the shader paints
/// can be read back and set against `planar8_to_rgb`, the conversion the player
/// falls back to as soon as a frame has to be graded.
pub(super) fn build(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> VideoGpu {
    build_with_source(device, target_format, SHADER)
}
pub(super) fn build_with_source(
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
        // window, picture settings, both LUTs, authored domain, encoder geometry
        // and stereo/spherical view: 192 bytes laid out as Params declares.
        size: UNIFORM_BYTES,
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
        imported_surface: false,
        serial: u64::MAX,
        window: [0.0; 4],
        adjust: IDENTITY_ADJUST,
        view: IDENTITY_VIEW,
        rgba_scratch: Vec::new(),
    }
}

struct FramePlanes<'a> {
    data: [&'a [u8]; 3],
    pub(super) size: [usize; 2],
    pub(super) chroma: [usize; 2],
    pub(super) depth: u8,
    colour: AvcColour,
}

impl VideoGpu {
    pub(super) fn upload_shared_rgb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &Arc<Vec<u8>>,
        size: [usize; 2],
        window: [f32; 4],
        adjust: [f32; 5],
        view: GpuViewParams,
    ) -> crate::Result<bool> {
        if self.surface_mode >= 3.0
            && self.planes.as_ref().is_some_and(|p| p.size == size)
            && self
                .rgb_source
                .as_ref()
                .is_some_and(|source| Arc::ptr_eq(source, rgb))
        {
            if self.window != window || self.adjust != adjust || self.view != view {
                self.window = window;
                self.adjust = adjust;
                self.view = view;
                self.write_params(queue, 8, AvcColour::default(), window, adjust, view);
            }
            return Ok(false);
        }
        let channels = if size[0]
            .checked_mul(size[1])
            .and_then(|n| n.checked_mul(4))
            == Some(rgb.len())
        {
            4
        } else {
            3
        };
        self.upload_codes_viewed(device, queue, rgb, size, 0, window, adjust, channels, view)?;
        self.rgb_source = Some(rgb.clone());
        Ok(true)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
    ) {
        self.upload_viewed(
            device,
            queue,
            frame,
            serial,
            window,
            adjust,
            grade,
            rotation,
            IDENTITY_VIEW,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_viewed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &Planar8,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
        view: GpuViewParams,
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
            rotation,
            view,
        );
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_packed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &PackedPlanar,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
    ) -> crate::Result<()> {
        self.upload_packed_viewed(
            device,
            queue,
            frame,
            serial,
            window,
            adjust,
            grade,
            rotation,
            IDENTITY_VIEW,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_packed_viewed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &PackedPlanar,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
        view: GpuViewParams,
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
            rotation,
            view,
        );
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_rgb(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
    ) -> crate::Result<()> {
        self.upload_codes(device, queue, rgb, size, serial, window, adjust, 3)
    }
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(super) fn upload_rgba(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgba: &[u8],
        size: [usize; 2],
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
    ) -> crate::Result<()> {
        self.upload_codes(device, queue, rgba, size, serial, window, adjust, 4)
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_codes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        channels: usize,
    ) -> crate::Result<()> {
        self.upload_codes_viewed(
            device,
            queue,
            rgb,
            size,
            serial,
            window,
            adjust,
            channels,
            IDENTITY_VIEW,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) fn upload_codes_viewed(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rgb: &[u8],
        size: [usize; 2],
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        channels: usize,
        view: GpuViewParams,
    ) -> crate::Result<()> {
        let len = size[0]
            .checked_mul(size[1])
            .and_then(|n| n.checked_mul(channels));
        if size.contains(&0)
            || len != Some(rgb.len())
            || size
                .iter()
                .any(|&dimension| dimension > device.limits().max_texture_dimension_2d as usize)
        {
            return Err(crate::invalid("invalid RGB display frame"));
        }
        let target_mode = 3.0;
        let chroma = [1, 1];
        if self.imported_surface
            || self.surface_mode != target_mode
            || self
                .planes
                .as_ref()
                .is_none_or(|p| p.size != size || p.chroma != chroma)
        {
            let mut planes = self.create_planes(device, size, chroma, 8);
            planes.textures[0] = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("fvid RGBA display"),
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
        self.surface_mode = target_mode;
        self.imported_surface = false;
        self.rgb_source = None;
        self.rotation = 0;
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        self.view = view;
        self.grid.set(device, queue, None);
        self.bind_planes(device, size, chroma);
        let rgba_bytes: &[u8] = if channels == 4 {
            rgb
        } else {
            let pixels = size[0] * size[1];
            self.rgba_scratch.resize(pixels * 4, 255);
            let mut src_chunks = rgb.chunks_exact(12);
            let mut dst_chunks = self.rgba_scratch.chunks_exact_mut(16);
            for (src, dst) in (&mut src_chunks).zip(&mut dst_chunks) {
                dst[0..3].copy_from_slice(&src[0..3]);
                dst[3] = 255;
                dst[4..7].copy_from_slice(&src[3..6]);
                dst[7] = 255;
                dst[8..11].copy_from_slice(&src[6..9]);
                dst[11] = 255;
                dst[12..15].copy_from_slice(&src[9..12]);
                dst[15] = 255;
            }
            for (src, dst) in src_chunks
                .remainder()
                .chunks_exact(3)
                .zip(dst_chunks.into_remainder().chunks_exact_mut(4))
            {
                dst[0..3].copy_from_slice(src);
                dst[3] = 255;
            }
            &self.rgba_scratch
        };
        let planes = self.planes.as_ref().unwrap();
        queue.write_texture(
            planes.textures[0].as_image_copy(),
            rgba_bytes,
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
        self.write_params(queue, 8, AvcColour::default(), window, adjust, view);
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn upload_planes(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: FramePlanes<'_>,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
        view: GpuViewParams,
    ) {
        let size = frame.size;
        let chroma = frame.chroma;
        let same_planes = !self.imported_surface
            && self.surface_mode == 0.0
            && serial != 0
            && serial != u64::MAX
            && self.serial == serial
            && self
                .planes
                .as_ref()
                .is_some_and(|p| p.size == size && p.chroma == chroma && p.depth == frame.depth);
        if same_planes
            && self.window == window
            && self.adjust == adjust
            && self.view == view
            && self.grid.holds(grade)
            && self.rotation == rotation
        {
            return;
        }
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        self.view = view;
        if same_planes {
            self.rotation = rotation;
            self.grid.set(device, queue, grade);
            self.bind_planes(device, size, chroma);
            self.write_params(queue, frame.depth, frame.colour, window, adjust, view);
            return;
        }
        let bytes_per_sample = if frame.depth == 8 { 1 } else { 2 };
        if self.imported_surface
            || self.planes.as_ref().is_none_or(|p| {
                p.size != size
                    || p.chroma != chroma
                    || p.depth != frame.depth
                    || self.surface_mode != 0.0
            })
        {
            self.planes = Some(self.create_planes(device, size, chroma, frame.depth));
            self.bound = None;
        }
        self.surface_mode = 0.0;
        self.imported_surface = false;
        self.rgb_source = None;
        self.rotation = rotation;
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
        self.write_params(queue, frame.depth, frame.colour, window, adjust, view);
    }
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[allow(clippy::too_many_arguments)]
    #[cfg(test)]
    pub(super) fn import_surface(
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
    pub(super) fn import_surface_rotated(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &fvid_vt::Surface,
        colour: AvcColour,
        serial: u64,
        window: [f32; 4],
        adjust: [f32; 5],
        grade: Option<&Arc<Grade>>,
        rotation: u16,
    ) -> crate::Result<()> {
        self.import_surface_viewed(
            device,
            queue,
            frame,
            colour,
            serial,
            window,
            adjust,
            grade,
            rotation,
            IDENTITY_VIEW,
        )
    }
    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn import_surface_viewed(
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
        view: GpuViewParams,
    ) -> crate::Result<()> {
        if ![0, 90, 180, 270].contains(&rotation) {
            return Err(crate::invalid("invalid surface rotation"));
        }
        if !self.imported_surface
            || self.serial != serial
            || self
                .planes
                .as_ref()
                .is_none_or(|p| p.size != [frame.width(), frame.height()] || p.depth != frame.depth())
        {
            let imported = frame
                .import_metal(device)
                .map_err(|e| crate::Error::Gpu(e.to_string()))?;
            let (native_mode, textures) = if let Some(cr) = imported.cr {
                (0.0, [imported.y, imported.uv, cr])
            } else {
                let mode = if frame.depth() == 8 { 1.0 } else { 2.0 };
                (mode, [imported.y, imported.uv.clone(), imported.uv])
            };
            let views = std::array::from_fn(|i| textures[i].create_view(&Default::default()));
            self.planes = Some(Planes {
                size: [frame.width(), frame.height()],
                chroma: [frame.width().div_ceil(2), frame.height().div_ceil(2)],
                depth: frame.depth(),
                textures,
                views,
            });
            self.surface_mode = native_mode;
            self.imported_surface = true;
            self.bound = None;
        } else if self.rotation == rotation
            && self.window == window
            && self.adjust == adjust
            && self.view == view
            && self.grid.holds(grade)
        {
            return Ok(());
        }
        self.rotation = rotation;
        self.rgb_source = None;
        self.serial = serial;
        self.window = window;
        self.adjust = adjust;
        self.view = view;
        self.grid.set(device, queue, grade);
        self.bind_planes(
            device,
            [frame.width(), frame.height()],
            [frame.width().div_ceil(2), frame.height().div_ceil(2)],
        );
        colour.full = frame.full_range();
        self.write_params(queue, frame.depth(), colour, window, adjust, view);
        Ok(())
    }
    pub(super) fn bind_planes(&mut self, device: &wgpu::Device, size: [usize; 2], chroma: [usize; 2]) {
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
    pub(super) fn write_params(
        &self,
        queue: &wgpu::Queue,
        depth: u8,
        colour: AvcColour,
        window: [f32; 4],
        adjust: [f32; 5],
        view: GpuViewParams,
    ) {
        // Limited range maps 16..235 (luma) and 16..240 (chroma) onto 0..1.
        let scale = (1u32 << (depth - 8)) as f32;
        let full_range = ((1u32 << depth) - 1) as f32 / scale;
        let (y_offset, y_range, c_range) = if colour.full {
            (0.0, full_range, full_range)
        } else {
            (16.0, 219.0, 224.0)
        };
        let look = &self.grid.look_params;
        let scalars: [f32; 48] = [
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
            look[0],
            look[1],
            look[2],
            look[3],
            look[4],
            look[5],
            look[6],
            look[7],
            look[8],
            look[9],
            look[10],
            look[11],
            0.0,
            0.0,
            0.0,
            0.0,
            view.stereo_mode,
            view.spherical_mode,
            view.spherical_stereo,
            view.aspect,
            view.yaw_rad,
            view.pitch_rad,
            view.roll_rad,
            view.fov_rad,
        ];
        let mut bytes = [0u8; UNIFORM_BYTES as usize];
        for (dst, &val) in bytes.chunks_exact_mut(4).zip(&scalars) {
            dst.copy_from_slice(&val.to_le_bytes());
        }
        queue.write_buffer(&self.uniform, 0, &bytes);
    }
    pub(super) fn create_planes(
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
    pub rotation: u16,
    pub view: GpuViewParams,
}

/// Display already graded RGB codes through the same custom shader as planar frames.
pub struct RgbVideoCallback {
    pub frame: Arc<Vec<u8>>,
    pub size: [usize; 2],
    pub window: [f32; 4],
    pub adjust: [f32; 5],
    pub view: GpuViewParams,
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
                self.view,
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
            gpu.upload_viewed(
                device,
                queue,
                &self.frame,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
                self.rotation,
                self.view,
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
    pub rotation: u16,
    pub view: GpuViewParams,
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
            if let Err(error) = gpu.upload_packed_viewed(
                device,
                queue,
                &self.frame,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
                self.rotation,
                self.view,
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
    pub view: GpuViewParams,
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
            if let Err(error) = gpu.import_surface_viewed(
                device,
                queue,
                &self.frame,
                self.colour,
                self.serial,
                self.window,
                self.adjust,
                self.grade.as_ref(),
                self.rotation,
                self.view,
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
