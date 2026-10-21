//! Portable integer-texture and UMA compute processing on Metal, Vulkan, D3D12 and GL/GLES.
//! No window, surface, float color conversion, or unsafe code in the root crate.
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
    execution: ExecutionMode,
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

enum ExecutionMode {
    #[cfg(target_os = "macos")]
    UmaCompute {
        buffers: Vec<fvid_vt::UmaSharedBuffer>,
        stages: Vec<ComputeStage>,
        last_submission: Option<wgpu::SubmissionIndex>,
    },
    Render {
        frames: Vec<wgpu::Texture>,
        stages: Vec<RenderStage>,
        upload: wgpu::Buffer,
        readback: wgpu::Buffer,
    },
}

#[cfg(target_os = "macos")]
struct ComputeStage {
    pipeline: wgpu::ComputePipeline,
    bindings: wgpu::BindGroup,
    groups_x: u32,
    groups_y: u32,
}

struct RenderStage {
    pipeline: wgpu::RenderPipeline,
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
        Self::new_shader_chain(
            plans,
            &vec![None; plans.len()],
            backend,
            ordinal,
            memory_limit,
        )
    }
    pub(crate) fn new_shader_chain(
        plans: &[&Plan],
        shaders: &[Option<&crate::resident::ByteShader>],
        backend: Backend,
        ordinal: usize,
        memory_limit: usize,
    ) -> Result<Self> {
        if plans.len() != shaders.len() {
            return Err(gpu_error("shader count must match stage count"));
        }
        let plan = plans
            .first()
            .ok_or_else(|| gpu_error("GPU chain cannot be empty"))?;
        let last = plans.last().expect("nonempty chain");
        let adapter = adapters(backend)?
            .into_iter()
            .nth(ordinal)
            .ok_or_else(|| unavailable(backend, &format!("hardware device {ordinal} not found")))?;
        let info = adapter.get_info();
        let name = info.name;
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
        // All resident textures/buffers plus boundary budget.
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
        #[cfg(target_os = "macos")]
        let use_uma_compute = backend == Backend::Metal
            && info.device_type == wgpu::DeviceType::IntegratedGpu
            && limits.max_storage_buffers_per_shader_stage >= 2;
        #[cfg(not(target_os = "macos"))]
        let use_uma_compute = false;

        let base_limits = if use_uma_compute {
            wgpu::Limits::downlevel_defaults().using_resolution(limits.clone())
        } else {
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(limits.clone())
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("fvid GPU"),
            required_limits: wgpu::Limits {
                max_buffer_size: limits.max_buffer_size,
                ..base_limits
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

        let execution = Self::build_execution(
            &device,
            plans,
            shaders,
            &layouts,
            input_layout,
            output_layout,
            format,
            usages,
            use_uma_compute,
        )?;

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
            execution,
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

    #[allow(clippy::too_many_arguments)]
    fn build_execution(
        device: &wgpu::Device,
        plans: &[&Plan],
        shaders: &[Option<&crate::resident::ByteShader>],
        layouts: &[Layout],
        input_layout: Layout,
        output_layout: Layout,
        format: wgpu::TextureFormat,
        usages: wgpu::TextureUsages,
        use_uma_compute: bool,
    ) -> Result<ExecutionMode> {
        #[cfg(target_os = "macos")]
        if use_uma_compute {
            let mut buffers = Vec::with_capacity(layouts.len());
            for (index, layout) in layouts.iter().enumerate() {
                let uma = fvid_vt::UmaSharedBuffer::new(
                    device,
                    layout.bytes,
                    wgpu::BufferUsages::STORAGE,
                    Some(if index == 0 {
                        "UMA zero-copy input"
                    } else {
                        "UMA zero-copy stage output"
                    }),
                )
                .map_err(gpu_error)?;
                buffers.push(uma);
            }
            let bind_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("fvid UMA compute bindings"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: true },
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(4),
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Storage { read_only: false },
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(4),
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: wgpu::BufferSize::new(128),
                            },
                            count: None,
                        },
                    ],
                });
            let pipeline_layout =
                device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("fvid UMA compute pipeline"),
                    bind_group_layouts: &[Some(&bind_layout)],
                    immediate_size: 0,
                });
            let mut pipelines =
                std::collections::HashMap::<&str, wgpu::ComputePipeline>::new();
            let max_wg = device.limits().max_compute_workgroups_per_dimension.max(1);
            let mut stages = Vec::with_capacity(plans.len());
            for (index, plan) in plans.iter().enumerate() {
                let mut params = plan.gpu_params()?;
                params[28] = layouts[index].width;
                params[29] = layouts[index + 1].width;
                let bytes: Vec<u8> = params.into_iter().flat_map(u32::to_le_bytes).collect();
                let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("UMA compute stage plan"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bindings = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("UMA compute stage bindings"),
                    layout: &bind_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffers[index].wgpu_buffer().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: buffers[index + 1].wgpu_buffer().as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: uniform.as_entire_binding(),
                        },
                    ],
                });
                let source = shaders[index].map_or(include_str!("compute.wgsl"), |shader| {
                    shader.compute_compiled_source()
                });
                let pipeline = pipelines
                    .entry(source)
                    .or_insert_with(|| {
                        let shader =
                            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                                label: Some("fvid UMA compute shader"),
                                source: wgpu::ShaderSource::Wgsl(source.into()),
                            });
                        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some("fvid UMA compute crop/reflect"),
                            layout: Some(&pipeline_layout),
                            module: &shader,
                            entry_point: Some("compute_main"),
                            compilation_options: Default::default(),
                            cache: None,
                        })
                    })
                    .clone();
                let words = u32::try_from(plan.output_len.div_ceil(4)).map_err(gpu_error)?;
                let total_groups = words.div_ceil(64).max(1);
                let groups_x = total_groups.min(max_wg);
                let groups_y = total_groups.div_ceil(groups_x).max(1);
                stages.push(ComputeStage {
                    pipeline,
                    bindings,
                    groups_x,
                    groups_y,
                });
            }
            return Ok(ExecutionMode::UmaCompute {
                buffers,
                stages,
                last_submission: None,
            });
        }
        #[cfg(not(target_os = "macos"))]
        let _ = use_uma_compute;

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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fvid pipeline"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mut pipelines = std::collections::HashMap::<&str, wgpu::RenderPipeline>::new();
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
            let source = shaders[index].map_or(include_str!("transform.wgsl"), |shader| {
                shader.compiled_source()
            });
            let pipeline = pipelines
                .entry(source)
                .or_insert_with(|| {
                    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("fvid byte shader"),
                        source: wgpu::ShaderSource::Wgsl(source.into()),
                    });

                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                    })
                })
                .clone();
            stages.push(RenderStage {
                pipeline,
                bindings,
                output_view,
            });
        }
        Ok(ExecutionMode::Render {
            frames,
            stages,
            upload,
            readback,
        })
    }

    pub(crate) fn uses_uma_compute(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            matches!(self.execution, ExecutionMode::UmaCompute { .. })
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
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
        match &mut self.execution {
            #[cfg(target_os = "macos")]
            ExecutionMode::UmaCompute {
                buffers,
                last_submission,
                ..
            } => {
                // If an earlier frame was dispatched and discarded without download,
                // wait for its in-flight compute pass before mutating the shared UMA buffer.
                if let Some(submission) = last_submission.take() {
                    self.device
                        .poll(wgpu::PollType::Wait {
                            submission_index: Some(submission),
                            timeout: Some(Duration::from_secs(30)),
                        })
                        .map_err(gpu_error)?;
                }
                buffers[0].write_slice(input).map_err(gpu_error)?;
            }
            ExecutionMode::Render { frames, upload, .. } => {
                let tx = self.map_sender.clone();
                upload.slice(..).map_async(wgpu::MapMode::Write, move |r| {
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
                {
                    let mut mapped = upload
                        .slice(..)
                        .get_mapped_range_mut()
                        .map_err(gpu_error)?;
                    mapped.slice(..input.len()).copy_from_slice(input);
                    mapped.slice(input.len()..).fill(0);
                }
                upload.unmap();
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("fvid frame"),
                        });
                encoder.copy_buffer_to_texture(
                    wgpu::TexelCopyBufferInfo {
                        buffer: upload,
                        layout: self.input_layout.copy_layout(),
                    },
                    frames[0].as_image_copy(),
                    self.input_layout.extent(),
                );
                self.queue.submit([encoder.finish()]);
            }
        }
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
        let pass_count = match &mut self.execution {
            #[cfg(target_os = "macos")]
            ExecutionMode::UmaCompute {
                stages,
                last_submission,
                ..
            } => {
                for stage in stages.iter() {
                    let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("fvid UMA compute transform"),
                        timestamp_writes: None,
                    });
                    pass.set_pipeline(&stage.pipeline);
                    pass.set_bind_group(0, &stage.bindings, &[]);
                    pass.dispatch_workgroups(stage.groups_x, stage.groups_y, 1);
                }
                let count = stages.len();
                *last_submission = Some(self.queue.submit([encoder.finish()]));
                count
            }
            ExecutionMode::Render { stages, .. } => {
                for stage in stages.iter() {
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
                    pass.set_pipeline(&stage.pipeline);
                    pass.set_bind_group(0, &stage.bindings, &[]);
                    pass.draw(0..3, 0..1);
                }
                let count = stages.len();
                self.queue.submit([encoder.finish()]);
                count
            }
        };
        self.transfers.filter_passes += pass_count as u64;
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
        match &mut self.execution {
            #[cfg(target_os = "macos")]
            ExecutionMode::UmaCompute {
                buffers,
                last_submission,
                ..
            } => {
                let submission = last_submission.take();
                self.device
                    .poll(wgpu::PollType::Wait {
                        submission_index: submission,
                        timeout: Some(Duration::from_secs(30)),
                    })
                    .map_err(gpu_error)?;
                buffers
                    .last()
                    .expect("resident UMA output")
                    .read_slice(output)
                    .map_err(gpu_error)?;
            }
            ExecutionMode::Render {
                frames, readback, ..
            } => {
                let mut encoder =
                    self.device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("explicit resident readback"),
                        });
                encoder.copy_texture_to_buffer(
                    frames.last().expect("resident output").as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: readback,
                        layout: self.output_layout.copy_layout(),
                    },
                    self.output_layout.extent(),
                );
                self.queue.submit([encoder.finish()]);
                let tx = self.map_sender.clone();
                readback.slice(..).map_async(wgpu::MapMode::Read, move |r| {
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
                {
                    let mapped = readback
                        .slice(..)
                        .get_mapped_range()
                        .map_err(gpu_error)?;
                    output.copy_from_slice(&mapped[..output.len()]);
                }
                readback.unmap();
            }
        }
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
