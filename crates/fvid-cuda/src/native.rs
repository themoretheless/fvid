use std::sync::Arc;

use cudarc::driver::{
    CudaContext, CudaFunction, CudaSlice, CudaStream, LaunchConfig, PushKernelArg,
};
use cudarc::nvrtc::{CompileOptions, compile_ptx_with_opts};

pub(super) struct Processor {
    // cudarc buffers retain their stream/context and function retains its module.
    input: CudaSlice<u8>,
    output: CudaSlice<u8>,
    params: CudaSlice<u32>,
    kernel: CudaFunction,
    stream: Arc<CudaStream>,
    name: String,
    poisoned: bool,
}

fn require_driver() -> Result<(), String> {
    // SAFETY: Loads only the CUDA driver's fixed standard library names through
    // cudarc. This is the same native-code trust boundary as creating its context;
    // no caller-controlled library paths or pointers are accepted by this crate.
    if !unsafe { cudarc::driver::sys::is_culib_present() } {
        return Err("CUDA driver library was not found; install an NVIDIA driver and make libcuda.so.1 (Linux) or nvcuda.dll (Windows) available".into());
    }
    Ok(())
}

pub(super) fn devices() -> Result<Vec<(usize, String)>, String> {
    require_driver()?;
    let count = CudaContext::device_count()
        .map_err(|err| format!("CUDA driver initialization failed: {err}"))?;
    (0..count)
        .map(|ordinal| {
            let device = cudarc::driver::result::device::get(ordinal)
                .map_err(|err| format!("CUDA device {ordinal} lookup failed: {err}"))?;
            let name = cudarc::driver::result::device::get_name(device)
                .map_err(|err| format!("CUDA device {ordinal} name lookup failed: {err}"))?;
            Ok((ordinal as usize, name))
        })
        .collect()
}

impl Processor {
    pub(super) fn new(
        input_len: usize,
        output_len: usize,
        params: [u32; 32],
        ordinal: usize,
    ) -> Result<Self, String> {
        require_driver()?;
        let count = CudaContext::device_count()
            .map_err(|err| format!("CUDA driver initialization failed: {err}"))?;
        if ordinal >= count as usize {
            return Err(format!(
                "CUDA device ordinal {ordinal} is unavailable ({count} devices found)"
            ));
        }
        // SAFETY: As above, this probes fixed vendor library names; no FFI pointers
        // or untrusted kernel sources are accepted. Probe before cudarc's loader,
        // which otherwise panics when the optional NVRTC library is absent.
        if !unsafe { cudarc::nvrtc::sys::is_culib_present() } {
            return Err("CUDA NVRTC compiler was not found; install the CUDA 12 NVRTC runtime and expose libnvrtc.so (Linux) or nvrtc64_120_0.dll (Windows) in the library search path".into());
        }
        let context = CudaContext::new(ordinal)
            .map_err(|err| format!("CUDA device {ordinal} initialization failed: {err}"))?;
        let name = context
            .name()
            .map_err(|err| format!("CUDA device name failed: {err}"))?;
        let (major, minor) = context
            .compute_capability()
            .map_err(|err| format!("CUDA compute capability lookup failed: {err}"))?;
        let ptx = compile_ptx_with_opts(
            include_str!("transform.cu"),
            CompileOptions {
                options: vec![format!("--gpu-architecture=compute_{major}{minor}")],
                name: Some("fvid_transform.cu".into()),
                ..Default::default()
            },
        )
        .map_err(|err| {
            format!(
                "CUDA kernel compilation failed; use an NVRTC version supporting this GPU: {err:?}"
            )
        })?;
        let module = context.load_module(ptx).map_err(|err| {
            format!("CUDA kernel loading failed; check driver and NVRTC compatibility: {err}")
        })?;
        let kernel = module
            .load_function("fvid_transform")
            .map_err(|err| format!("CUDA transform function loading failed: {err}"))?;
        let stream = context
            .new_stream()
            .map_err(|err| format!("CUDA stream creation failed: {err}"))?;
        let input = stream
            .alloc_zeros::<u8>(input_len)
            .map_err(|err| format!("CUDA input allocation ({input_len} bytes) failed: {err}"))?;
        let output = stream
            .alloc_zeros::<u8>(output_len)
            .map_err(|err| format!("CUDA output allocation ({output_len} bytes) failed: {err}"))?;
        let params = stream
            .clone_htod(&params)
            .map_err(|err| format!("CUDA parameter upload failed: {err}"))?;
        stream
            .synchronize()
            .map_err(|err| format!("CUDA setup synchronization failed: {err}"))?;
        Ok(Self {
            input,
            output,
            params,
            kernel,
            stream,
            name,
            poisoned: false,
        })
    }

    pub(super) fn device_name(&self) -> &str {
        &self.name
    }

    pub(super) fn apply(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), String> {
        if input.len() != self.input.len() || output.len() != self.output.len() {
            return Err(format!(
                "CUDA buffer lengths must be input={} and output={}; received {} and {}",
                self.input.len(),
                self.output.len(),
                input.len(),
                output.len()
            ));
        }
        if self.poisoned {
            return Err(
                "CUDA processor failed previously; create a new processor before retrying".into(),
            );
        }
        let result = self.apply_frame(input, output);
        if result.is_err() {
            self.poisoned = true;
            // Finish queued work even on a launch/copy error before caller buffers
            // can be reused. cudarc additionally synchronizes borrowed host slices.
            let _ = self.stream.synchronize();
        }
        result
    }

    fn apply_frame(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), String> {
        self.stream
            .memcpy_htod(input, &mut self.input)
            .map_err(|err| format!("CUDA input upload failed: {err}"))?;
        let mut arguments = self.stream.launch_builder(&self.kernel);
        arguments
            .arg(&self.input)
            .arg(&mut self.output)
            .arg(&self.params);
        let config = LaunchConfig {
            grid_dim: ((output.len() as u32).div_ceil(256), 1, 1),
            block_dim: (256, 1, 1),
            shared_mem_bytes: 0,
        };
        // SAFETY: The fixed kernel accepts (const u8*, u8*, const u32*) in this
        // exact order. `validate` proves every address, nonzero divisor, packed
        // output coverage and all u32 arithmetic bounds before allocation. Each
        // thread owns one output byte. CudaSlice handles keep allocations alive;
        // one stream orders uploads/kernel/download, and synchronization below
        // completes GPU work before apply returns. No raw device pointer escapes.
        unsafe { arguments.launch(config) }
            .map_err(|err| format!("CUDA transform launch failed: {err}"))?;
        self.stream
            .memcpy_dtoh(&self.output, output)
            .map_err(|err| format!("CUDA output download failed: {err}"))?;
        self.stream
            .synchronize()
            .map_err(|err| format!("CUDA frame synchronization failed: {err}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::CudaProcessor;

    #[test]
    #[ignore = "requires an NVIDIA GPU, CUDA driver and NVRTC; run explicitly on a CUDA host"]
    fn native_cuda_crop_reflections_and_buffer_reuse() {
        let mut input: Vec<u8> = (0..24).collect();
        for (horizontal, vertical, expected_y) in [
            (0, 0, [5, 6, 9, 10]),
            (1, 0, [6, 5, 10, 9]),
            (0, 1, [9, 10, 5, 6]),
            (1, 1, [10, 9, 6, 5]),
        ] {
            let mut p = crate::tests::params();
            p[24] = horizontal;
            p[25] = vertical;
            let mut processor = CudaProcessor::new(24, 6, p, 0).expect("CUDA device required");
            let mut output = [0xa5; 6];
            assert!(processor.apply(&input[..23], &mut output).is_err());
            for delta in [0u8, 40] {
                for (index, value) in input.iter_mut().enumerate() {
                    *value = index as u8 + delta;
                }
                processor.apply(&input, &mut output).unwrap();
                let expected = [
                    expected_y[0],
                    expected_y[1],
                    expected_y[2],
                    expected_y[3],
                    16,
                    20,
                ]
                .map(|value| value + delta);
                assert_eq!(output, expected);
            }
        }
    }

    #[test]
    #[ignore = "requires an NVIDIA GPU, CUDA driver and NVRTC; run explicitly on a CUDA host"]
    fn native_cuda_strided_planes_across_workgroups() {
        let mut params = [0; 32];
        let mut input_offset = 0usize;
        let mut output_offset = 0usize;
        let planes = [
            (68usize, 50usize, 2usize, 2usize, 60usize, 46usize),
            (34, 25, 1, 1, 30, 23),
            (34, 25, 1, 1, 30, 23),
        ];
        for (plane, &(stride, input_height, x, y, width, height)) in planes.iter().enumerate() {
            params[plane * 8..plane * 8 + 8].copy_from_slice(&[
                input_offset as u32,
                output_offset as u32,
                stride as u32,
                x as u32,
                y as u32,
                width as u32,
                height as u32,
                0,
            ]);
            input_offset += stride * input_height;
            output_offset += width * height;
        }
        params[26] = input_offset as u32;
        params[27] = output_offset as u32;
        let input: Vec<u8> = (0..input_offset)
            .map(|index| ((index * 31 + index / 13) % 251) as u8)
            .collect();
        for horizontal in [false, true] {
            for vertical in [false, true] {
                params[24] = u32::from(horizontal);
                params[25] = u32::from(vertical);
                let mut expected = Vec::with_capacity(output_offset);
                let mut start = 0;
                for &(stride, input_height, x, y, width, height) in &planes {
                    let mut rows: Vec<Vec<u8>> = (y..y + height)
                        .map(|row| {
                            input[start + row * stride + x..start + row * stride + x + width]
                                .to_vec()
                        })
                        .collect();
                    if vertical {
                        rows.reverse();
                    }
                    for mut row in rows {
                        if horizontal {
                            row.reverse();
                        }
                        expected.extend(row);
                    }
                    start += stride * input_height;
                }
                let mut processor = CudaProcessor::new(input_offset, output_offset, params, 0)
                    .expect("CUDA device required");
                let mut output = vec![0xa5; output_offset];
                processor.apply(&input, &mut output).unwrap();
                assert_eq!(output, expected);
            }
        }
    }
}

struct ResidentStage {
    output: CudaSlice<u8>,
    params: CudaSlice<u32>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Empty,
    Uploaded,
    Processed,
}
pub(super) struct Pipeline {
    first: Processor,
    stages: Vec<ResidentStage>,
    phase: Phase,
    pub(super) bytes: usize,
    pub(super) transfers: crate::pipeline::TransferStats,
}
impl Pipeline {
    pub(super) fn new(plans: &[[u32; 32]], ordinal: usize, bytes: usize) -> Result<Self, String> {
        let p = plans[0];
        let first = Processor::new(p[26] as usize, p[27] as usize, p, ordinal)?;
        let mut stages = Vec::with_capacity(plans.len() - 1);
        for p in &plans[1..] {
            let output = first
                .stream
                .alloc_zeros::<u8>(p[27] as usize)
                .map_err(|e| e.to_string())?;
            let params = first.stream.clone_htod(p).map_err(|e| e.to_string())?;
            stages.push(ResidentStage { output, params });
        }
        first.stream.synchronize().map_err(|e| e.to_string())?;
        Ok(Self {
            first,
            stages,
            phase: Phase::Empty,
            bytes,
            transfers: Default::default(),
        })
    }
    pub(super) fn device_name(&self) -> &str {
        self.first.device_name()
    }
    fn check(&self) -> Result<(), String> {
        if self.first.poisoned {
            Err("CUDA resident pipeline is poisoned; recreate it".into())
        } else {
            Ok(())
        }
    }
    fn finish(&mut self, result: Result<(), String>) -> Result<(), String> {
        if result.is_err() {
            self.first.poisoned = true;
            let _ = self.first.stream.synchronize();
        }
        result
    }
    pub(super) fn upload(&mut self, input: &[u8]) -> Result<(), String> {
        self.check()?;
        if input.len() != self.first.input.len() {
            return Err("CUDA resident input length mismatch".into());
        }
        let result = self
            .first
            .stream
            .memcpy_htod(input, &mut self.first.input)
            .map_err(|e| e.to_string());
        self.finish(result)?;
        self.phase = Phase::Uploaded;
        self.transfers.uploads += 1;
        self.transfers.upload_bytes += input.len() as u64;
        Ok(())
    }
    pub(super) fn process(&mut self) -> Result<(), String> {
        self.check()?;
        if self.phase != Phase::Uploaded {
            return Err("upload a frame before CUDA processing".into());
        }
        let result = self.process_inner();
        self.finish(result)?;
        self.phase = Phase::Processed;
        self.transfers.filter_passes += 1 + self.stages.len() as u64;
        Ok(())
    }
    fn process_inner(&mut self) -> Result<(), String> {
        let first = &mut self.first;
        launch_resident(
            &first.stream,
            &first.kernel,
            &first.input,
            &mut first.output,
            &first.params,
        )?;
        for i in 0..self.stages.len() {
            let (previous, following) = self.stages.split_at_mut(i);
            let input = previous.last().map_or(&first.output, |stage| &stage.output);
            let stage = &mut following[0];
            launch_resident(
                &first.stream,
                &first.kernel,
                input,
                &mut stage.output,
                &stage.params,
            )?;
        }
        Ok(())
    }
    pub(super) fn download(&mut self, output: &mut [u8]) -> Result<(), String> {
        self.check()?;
        if self.phase != Phase::Processed {
            return Err("process a frame before CUDA download".into());
        }
        let source = self.stages.last().map_or(&self.first.output, |s| &s.output);
        if output.len() != source.len() {
            return Err("CUDA resident output length mismatch".into());
        }
        let result = self
            .first
            .stream
            .memcpy_dtoh(source, output)
            .map_err(|e| e.to_string())
            .and_then(|()| self.first.stream.synchronize().map_err(|e| e.to_string()));
        self.finish(result)?;
        self.phase = Phase::Empty;
        self.transfers.downloads += 1;
        self.transfers.download_bytes += output.len() as u64;
        Ok(())
    }
}
fn launch_resident(
    stream: &Arc<CudaStream>,
    kernel: &CudaFunction,
    input: &CudaSlice<u8>,
    output: &mut CudaSlice<u8>,
    params: &CudaSlice<u32>,
) -> Result<(), String> {
    let config = LaunchConfig {
        grid_dim: ((output.len() as u32).div_ceil(256), 1, 1),
        block_dim: (256, 1, 1),
        shared_mem_bytes: 0,
    };
    let mut args = stream.launch_builder(kernel);
    args.arg(input).arg(output).arg(params);
    // SAFETY: validate_chain validates every stage's fixed byte-gather ABI and
    // predecessor length before allocation. Input and output are distinct owned
    // allocations. One stream orders all stages; no host read occurs here.
    // CudaSlice retains stream/context and synchronizes lifetime-sensitive drops.
    unsafe { args.launch(config) }.map_err(|e| e.to_string())?;
    Ok(())
}
