#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
use super::{ColorShader, IDENTITY_ADJUST, VideoGpu, renderer::build_with_source};
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
use crate::color::Grade;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
use crate::playback_native::{AvcColour, PackedPlanar, Planar8};
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
use eframe::wgpu;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
use std::sync::Arc;

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
            0,
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
            0,
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
        let mut geometry = [0u8; 16];
        geometry[0..4].copy_from_slice(&(size[0] as f32).to_le_bytes());
        geometry[4..8].copy_from_slice(&(size[1] as f32).to_le_bytes());
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
