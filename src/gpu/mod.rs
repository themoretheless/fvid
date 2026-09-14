//! Portable integer-texture processing on Metal, Vulkan, D3D12 and GL/GLES.
//! No window, surface, float color conversion, compute shader, or unsafe code.
use crate::backend::{Backend, DeviceInfo, unavailable};
use crate::resident::TransferStats;
use crate::{Error, Plan, Result};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use wgpu::util::DeviceExt;

fn gpu_error(e: impl std::fmt::Display) -> Error {
    Error::Gpu(e.to_string())
}
fn mask(backend: Backend) -> Result<wgpu::Backends> {
    Ok(match backend {
        Backend::Metal => wgpu::Backends::METAL,
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Gl => wgpu::Backends::GL,
        _ => return Err(unavailable(backend, "not a wgpu backend")),
    })
}
fn adapters(backend: Backend) -> Result<Vec<wgpu::Adapter>> {
    let backends = mask(backend)?;
    if !wgpu::Instance::enabled_backend_features().contains(backends) {
        return Err(unavailable(
            backend,
            "not compiled for this platform; GL on Apple requires the angle feature and ANGLE/EGL; Vulkan on Apple requires vulkan-portability and MoltenVK",
        ));
    }
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = backends;
    let instance = wgpu::Instance::new(descriptor);
    let mut adapters = pollster::block_on(instance.enumerate_adapters(backends));
    // Do not mislabel software rasterization as GPU execution.
    adapters.retain(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
    adapters.sort_by_key(|a| {
        let i = a.get_info();
        (i.name, i.vendor, i.device)
    });
    Ok(adapters)
}
pub(crate) fn devices(backend: Backend) -> Result<Vec<DeviceInfo>> {
    Ok(adapters(backend)?
        .into_iter()
        .enumerate()
        .map(|(ordinal, a)| {
            let i = a.get_info();
            DeviceInfo {
                backend,
                ordinal,
                name: i.name,
                device_type: format!("{:?}", i.device_type),
            }
        })
        .collect())
}
#[derive(Clone, Copy, Debug)]
struct Layout {
    width: u32,
    height: u32,
    bytes: usize,
}
impl Layout {
    fn new(length: usize, limit: u32) -> Result<Self> {
        if length == 0 || length > u32::MAX as usize {
            return Err(gpu_error(
                "GPU payload must fit a nonzero 32-bit byte range",
            ));
        }
        if limit < 64 {
            return Err(gpu_error(
                "adapter texture limit is below row-alignment requirement",
            ));
        }
        let texels = length.div_ceil(4);
        let min_width = texels.div_ceil(limit as usize);
        let width = min_width.max(texels.min(1024)).next_multiple_of(64) as u32;
        let height = u32::try_from(length.div_ceil(width as usize * 4)).map_err(gpu_error)?;
        if width > limit || height > limit {
            return Err(gpu_error(format!(
                "frame texture {width}x{height} exceeds adapter limit {limit}"
            )));
        }
        let bytes = (width as usize)
            .checked_mul(height as usize)
            .and_then(|v| v.checked_mul(4))
            .ok_or_else(|| gpu_error("GPU texture allocation size overflow"))?;
        if bytes > u32::MAX as usize {
            return Err(gpu_error("padded GPU layout exceeds 32-bit indexing"));
        }
        Ok(Self {
            width,
            height,
            bytes,
        })
    }
    fn extent(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }
    fn copy_layout(self) -> wgpu::TexelCopyBufferLayout {
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(self.width * 4),
            rows_per_image: Some(self.height),
        }
    }
}
pub(crate) struct GpuProcessor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    frames: Vec<wgpu::Texture>,
    stages: Vec<Stage>,
    upload: wgpu::Buffer,
    readback: wgpu::Buffer,
    input_layout: Layout,
    output_layout: Layout,
    input_len: usize,
    output_len: usize,
    asynchronous_error: Arc<Mutex<Option<String>>>,
    pub name: String,
    pub controlled_memory: usize,
    pub transfers: TransferStats,
    poisoned: bool,
    map_sender: mpsc::SyncSender<std::result::Result<(), wgpu::BufferAsyncError>>,
    map_receiver: mpsc::Receiver<std::result::Result<(), wgpu::BufferAsyncError>>,
}
struct Stage {
    bindings: wgpu::BindGroup,
    output_view: wgpu::TextureView,
}
impl GpuProcessor {
    pub fn new(plan: &Plan, backend: Backend, ordinal: usize, memory_limit: usize) -> Result<Self> {
        Self::new_chain(&[plan], backend, ordinal, memory_limit)
    }
    pub(crate) fn new_chain(
        plans: &[&Plan],
        backend: Backend,
        ordinal: usize,
        memory_limit: usize,
    ) -> Result<Self> {
        let plan = plans
            .first()
            .ok_or_else(|| gpu_error("GPU chain cannot be empty"))?;
        let last = plans.last().expect("nonempty chain");
        let adapter = adapters(backend)?
            .into_iter()
            .nth(ordinal)
            .ok_or_else(|| unavailable(backend, &format!("hardware device {ordinal} not found")))?;
        let name = adapter.get_info().name;
        let limits = adapter.limits();
        let format = wgpu::TextureFormat::Rgba8Uint;
        let allowed = adapter.get_texture_format_features(format).allowed_usages;
        let usages = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        if !allowed.contains(usages) {
            return Err(unavailable(
                backend,
                "RGBA8Uint texture/render/readback not supported",
            ));
        }
        let mut layouts = vec![Layout::new(
            plan.input_len,
            limits.max_texture_dimension_2d,
        )?];
        for p in plans {
            layouts.push(Layout::new(p.output_len, limits.max_texture_dimension_2d)?);
        }
        let input_layout = layouts[0];
        let output_layout = *layouts.last().expect("input and output layouts");
        if input_layout.bytes as u64 > limits.max_buffer_size
            || output_layout.bytes as u64 > limits.max_buffer_size
        {
            return Err(gpu_error("staging buffer exceeds adapter max_buffer_size"));
        }
        // All resident textures plus exactly one boundary upload/readback pair.
        // Include caller boundary frame buffers to retain the CLI budget contract.
        let mut controlled_memory = plan
            .input_len
            .checked_add(last.output_len)
            .and_then(|n| n.checked_add(input_layout.bytes))
            .and_then(|n| n.checked_add(output_layout.bytes))
            .ok_or_else(|| gpu_error("GPU memory budget overflow"))?;
        for layout in &layouts {
            controlled_memory = controlled_memory
                .checked_add(layout.bytes)
                .ok_or_else(|| gpu_error("GPU memory budget overflow"))?;
        }
        controlled_memory = plans
            .len()
            .checked_mul(128)
            .and_then(|n| controlled_memory.checked_add(n))
            .ok_or_else(|| gpu_error("GPU memory budget overflow"))?;
        if controlled_memory > memory_limit {
            return Err(gpu_error(format!(
                "GPU frame/staging buffers need {controlled_memory} bytes, exceeding memory limit {memory_limit}"
            )));
        }
        let required_limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(limits.clone());
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fvid GPU"),
            required_limits: wgpu::Limits {
                max_buffer_size: limits.max_buffer_size,
                ..required_limits
            },
            memory_hints: wgpu::MemoryHints::MemoryUsage,
            ..Default::default()
        }))
        .map_err(gpu_error)?;
        let asynchronous_error = Arc::new(Mutex::new(None));
        let error_slot = asynchronous_error.clone();
        device.on_uncaptured_error(Arc::new(move |e| {
            if let Ok(mut slot) = error_slot.lock() {
                *slot = Some(e.to_string());
            }
        }));
        let lost_slot = asynchronous_error.clone();
        device.set_device_lost_callback(move |reason, message| {
            if let Ok(mut slot) = lost_slot.lock() {
                *slot = Some(format!("GPU device lost ({reason:?}): {message}"));
            }
        });
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let frames: Vec<_> = layouts
            .iter()
            .enumerate()
            .map(|(index, layout)| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(if index == 0 {
                        "resident input"
                    } else {
                        "resident stage output"
                    }),
                    size: layout.extent(),
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: usages,
                    view_formats: &[],
                })
            })
            .collect();
        let upload = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reusable upload"),
            size: input_layout.bytes as u64,
            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("reusable readback"),
            size: output_layout.bytes as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fvid bindings"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(128),
                    },
                    count: None,
                },
            ],
        });
        let mut stages = Vec::with_capacity(plans.len());
        for (index, plan) in plans.iter().enumerate() {
            let mut params = plan.gpu_params()?;
            params[28] = layouts[index].width;
            params[29] = layouts[index + 1].width;
            let bytes: Vec<u8> = params.into_iter().flat_map(u32::to_le_bytes).collect();
            let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("resident stage plan"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let input_view = frames[index].create_view(&Default::default());
            let output_view = frames[index + 1].create_view(&Default::default());
            let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("resident stage bindings"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&input_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: uniform.as_entire_binding(),
                    },
                ],
            });
            stages.push(Stage {
                bindings,
                output_view,
            });
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fvid byte gather"),
            source: wgpu::ShaderSource::Wgsl(include_str!("transform.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fvid pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fvid crop/reflect"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vertex"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fragment"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        if let Some(error) = [
            pollster::block_on(validation.pop()),
            pollster::block_on(internal.pop()),
            pollster::block_on(oom.pop()),
        ]
        .into_iter()
        .flatten()
        .next()
        {
            return Err(gpu_error(error));
        }
        if let Some(error) = asynchronous_error.lock().map_err(gpu_error)?.take() {
            return Err(gpu_error(error));
        }
        let (map_sender, map_receiver) = mpsc::sync_channel(1);
        Ok(Self {
            map_sender,
            map_receiver,
            device,
            queue,
            pipeline,
            frames,
            stages,
            upload,
            readback,
            input_layout,
            output_layout,
            input_len: plan.input_len,
            output_len: last.output_len,
            asynchronous_error,
            name,
            controlled_memory,
            transfers: TransferStats::default(),
            poisoned: false,
        })
    }
    fn map(&self, buffer: &wgpu::Buffer, mode: wgpu::MapMode) -> Result<()> {
        // Calls are sequential. A timeout poisons the pipeline, so a late
        // callback can never be consumed by a later map operation.
        let tx = self.map_sender.clone();
        buffer.slice(..).map_async(mode, move |r| {
            let _ = tx.send(r);
        });
        self.device
            .poll(wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(Duration::from_secs(30)),
            })
            .map_err(gpu_error)?;
        self.map_receiver
            .recv_timeout(Duration::from_secs(30))
            .map_err(gpu_error)?
            .map_err(gpu_error)?;
        self.check_error()
    }
    fn check_error(&self) -> Result<()> {
        if self.poisoned {
            return Err(gpu_error("GPU pipeline is poisoned; recreate it"));
        }
        if let Some(e) = self.asynchronous_error.lock().map_err(gpu_error)?.as_ref() {
            return Err(gpu_error(e));
        }
        Ok(())
    }
    pub fn apply(&mut self, input: &[u8], output: &mut [u8]) -> Result<()> {
        if input.len() != self.input_len || output.len() != self.output_len {
            return Err(gpu_error("GPU frame length does not match plan"));
        }
        self.upload_frame(input)?;
        self.dispatch_resident()?;
        self.download_frame(output)
    }
    pub(crate) fn upload_frame(&mut self, input: &[u8]) -> Result<()> {
        if input.len() != self.input_len {
            return Err(gpu_error("GPU input length does not match chain"));
        }
        let result = self.upload_inner(input);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn upload_inner(&mut self, input: &[u8]) -> Result<()> {
        self.check_error()?;
        self.map(&self.upload, wgpu::MapMode::Write)?;
        {
            let mut mapped = self
                .upload
                .slice(..)
                .get_mapped_range_mut()
                .map_err(gpu_error)?;
            mapped.slice(..input.len()).copy_from_slice(input);
            mapped.slice(input.len()..).fill(0);
        }
        self.upload.unmap();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("fvid frame"),
            });
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.upload,
                layout: self.input_layout.copy_layout(),
            },
            self.frames[0].as_image_copy(),
            self.input_layout.extent(),
        );
        self.queue.submit([encoder.finish()]);
        self.transfers.uploads += 1;
        self.transfers.upload_bytes += input.len() as u64;
        self.transfers.upload_staging_bytes += self.input_layout.bytes as u64;
        self.check_error()
    }
    pub(crate) fn dispatch_resident(&mut self) -> Result<()> {
        self.check_error()?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("resident filter chain; no host transfers"),
            });
        for stage in &self.stages {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("fvid transform"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &stage.output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &stage.bindings, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        self.transfers.filter_passes += self.stages.len() as u64;
        self.check_error()
    }
    pub(crate) fn download_frame(&mut self, output: &mut [u8]) -> Result<()> {
        if output.len() != self.output_len {
            return Err(gpu_error("GPU output length does not match chain"));
        }
        let result = self.download_inner(output);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn download_inner(&mut self, output: &mut [u8]) -> Result<()> {
        self.check_error()?;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("explicit resident readback"),
            });
        encoder.copy_texture_to_buffer(
            self.frames.last().expect("resident output").as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.readback,
                layout: self.output_layout.copy_layout(),
            },
            self.output_layout.extent(),
        );
        self.queue.submit([encoder.finish()]);
        self.map(&self.readback, wgpu::MapMode::Read)?;
        {
            let mapped = self
                .readback
                .slice(..)
                .get_mapped_range()
                .map_err(gpu_error)?;
            output.copy_from_slice(&mapped[..output.len()]);
        }
        self.readback.unmap();
        self.transfers.downloads += 1;
        self.transfers.download_bytes += output.len() as u64;
        self.transfers.download_staging_bytes += self.output_layout.bytes as u64;
        self.check_error()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layout_handles_unaligned_payload() {
        let l = Layout::new(45, 2048).unwrap();
        assert_eq!(l.bytes, 256);
        assert_eq!(l.width * 4 % 256, 0);
        assert!(Layout::new(0, 2048).is_err());
        let large = Layout::new(7680 * 4320 * 3, 16384).unwrap();
        assert!(large.width > 1024 && large.height <= 16384);
        assert!(large.bytes >= 7680 * 4320 * 3);
        assert!(Layout::new(4096 * 4096 * 4, 1024).is_err());
    }
}
