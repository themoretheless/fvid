use std::sync::Arc;

use cudarc::driver::{CudaFunction, CudaSlice, CudaStream, LaunchConfig, PushKernelArg};

use crate::device_pool::{self, SharedDevice};
use crate::host_pinned::HostPinned;

struct Slot {
    input: CudaSlice<u8>,
    output: CudaSlice<u8>,
    host_in: HostPinned,
    host_out: HostPinned,
    stream: Arc<CudaStream>,
}

pub(super) struct Processor {
    _device: Arc<SharedDevice>,
    slots: [Slot; 2],
    params: CudaSlice<u32>,
    kernel: CudaFunction,
    name: String,
    poisoned: bool,
    /// Frames submitted but not yet returned via submit/flush (0..=2).
    queued: u8,
    next_slot: u8,
    oldest_slot: u8,
}

pub(super) fn devices() -> Result<Vec<(usize, String)>, String> {
    device_pool::require_driver()?;
    let count = cudarc::driver::CudaContext::device_count()
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

fn make_slot(
    device: &SharedDevice,
    input_len: usize,
    output_len: usize,
) -> Result<Slot, String> {
    let stream = device.new_stream()?;
    let input = stream
        .alloc_zeros::<u8>(input_len)
        .map_err(|err| format!("CUDA input allocation ({input_len} bytes) failed: {err}"))?;
    let output = stream
        .alloc_zeros::<u8>(output_len)
        .map_err(|err| format!("CUDA output allocation ({output_len} bytes) failed: {err}"))?;
    let host_in = HostPinned::alloc(&device.context, input_len)?;
    let host_out = HostPinned::alloc(&device.context, output_len)?;
    Ok(Slot {
        input,
        output,
        host_in,
        host_out,
        stream,
    })
}

impl Processor {
    pub(super) fn new(
        input_len: usize,
        output_len: usize,
        params: [u32; 32],
        ordinal: usize,
    ) -> Result<Self, String> {
        let device = device_pool::shared(ordinal)?;
        let kernel = device.transform_kernel()?;
        let slot0 = make_slot(&device, input_len, output_len)?;
        let slot1 = make_slot(&device, input_len, output_len)?;
        let params = slot0
            .stream
            .clone_htod(&params)
            .map_err(|err| format!("CUDA parameter upload failed: {err}"))?;
        slot0
            .stream
            .synchronize()
            .map_err(|err| format!("CUDA setup synchronization failed: {err}"))?;
        let name = device.name.clone();
        Ok(Self {
            _device: device,
            slots: [slot0, slot1],
            params,
            kernel,
            name,
            poisoned: false,
            queued: 0,
            next_slot: 0,
            oldest_slot: 0,
        })
    }

    pub(super) fn device_name(&self) -> &str {
        &self.name
    }

    pub(super) fn input_len(&self) -> usize {
        self.slots[0].input.len()
    }

    pub(super) fn output_len(&self) -> usize {
        self.slots[0].output.len()
    }

    pub(super) fn apply(&mut self, input: &[u8], output: &mut [u8]) -> Result<(), String> {
        self.drain_pipeline()?;
        if input.len() != self.input_len() || output.len() != self.output_len() {
            return Err(format!(
                "CUDA buffer lengths must be input={} and output={}; received {} and {}",
                self.input_len(),
                self.output_len(),
                input.len(),
                output.len()
            ));
        }
        if self.poisoned {
            return Err(
                "CUDA processor failed previously; create a new processor before retrying".into(),
            );
        }
        let result = self.launch_slot(0, input).and_then(|()| {
            self.slots[0]
                .stream
                .synchronize()
                .map_err(|err| format!("CUDA frame synchronization failed: {err}"))?;
            output.copy_from_slice(self.slots[0].host_out.as_slice());
            Ok(())
        });
        if result.is_err() {
            self.poisoned = true;
            let _ = self.slots[0].stream.synchronize();
            let _ = self.slots[1].stream.synchronize();
        }
        result
    }

    /// Depth-2 pipeline: queue `input`. If a prior frame finished, write it to `output`
    /// and return `true`.
    pub(super) fn submit(&mut self, input: &[u8], output: &mut [u8]) -> Result<bool, String> {
        if input.len() != self.input_len() || output.len() != self.output_len() {
            return Err(format!(
                "CUDA buffer lengths must be input={} and output={}; received {} and {}",
                self.input_len(),
                self.output_len(),
                input.len(),
                output.len()
            ));
        }
        if self.poisoned {
            return Err(
                "CUDA processor failed previously; create a new processor before retrying".into(),
            );
        }
        let result = (|| {
            let mut produced = false;
            if self.queued == 2 {
                self.complete_oldest(output)?;
                self.queued = 1;
                produced = true;
            }
            let slot = self.next_slot as usize;
            self.launch_slot(slot, input)?;
            self.next_slot ^= 1;
            self.queued += 1;
            Ok(produced)
        })();
        if result.is_err() {
            self.poisoned = true;
            let _ = self.slots[0].stream.synchronize();
            let _ = self.slots[1].stream.synchronize();
        }
        result
    }

    pub(super) fn flush(&mut self, output: &mut [u8]) -> Result<bool, String> {
        if self.poisoned {
            return Err(
                "CUDA processor failed previously; create a new processor before retrying".into(),
            );
        }
        if self.queued == 0 {
            return Ok(false);
        }
        if output.len() != self.output_len() {
            return Err(format!(
                "CUDA output length must be {}; received {}",
                self.output_len(),
                output.len()
            ));
        }
        let result = self.complete_oldest(output).map(|()| {
            self.queued -= 1;
            true
        });
        if result.is_err() {
            self.poisoned = true;
            let _ = self.slots[0].stream.synchronize();
            let _ = self.slots[1].stream.synchronize();
        }
        result
    }

    fn drain_pipeline(&mut self) -> Result<(), String> {
        let mut sink = vec![0u8; self.output_len()];
        while self.queued > 0 {
            self.complete_oldest(&mut sink)?;
            self.queued -= 1;
        }
        self.next_slot = 0;
        self.oldest_slot = 0;
        Ok(())
    }

    fn complete_oldest(&mut self, output: &mut [u8]) -> Result<(), String> {
        let slot = self.oldest_slot as usize;
        self.slots[slot]
            .stream
            .synchronize()
            .map_err(|err| format!("CUDA frame synchronization failed: {err}"))?;
        output.copy_from_slice(self.slots[slot].host_out.as_slice());
        self.oldest_slot ^= 1;
        Ok(())
    }

    fn launch_slot(&mut self, slot: usize, input: &[u8]) -> Result<(), String> {
        let output_len = self.slots[slot].output.len();
        self.slots[slot].host_in.as_mut_slice().copy_from_slice(input);
        self.slots[slot]
            .stream
            .memcpy_htod(&self.slots[slot].host_in, &mut self.slots[slot].input)
            .map_err(|err| format!("CUDA input upload failed: {err}"))?;
        let config = LaunchConfig {
            grid_dim: ((output_len as u32).div_ceil(256), 1, 1),
            block_dim: (256, 1, 1),
            shared_mem_bytes: 0,
        };
        let mut arguments = self.slots[slot].stream.launch_builder(&self.kernel);
        arguments
            .arg(&self.slots[slot].input)
            .arg(&mut self.slots[slot].output)
            .arg(&self.params);
        // SAFETY: fixed ABI; slot buffers exclusive; host read only after sync.
        unsafe { arguments.launch(config) }
            .map_err(|err| format!("CUDA transform launch failed: {err}"))?;
        self.slots[slot]
            .stream
            .memcpy_dtoh(&self.slots[slot].output, &mut self.slots[slot].host_out)
            .map_err(|err| format!("CUDA output download failed: {err}"))?;
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
        let stream = first.slots[0].stream.clone();
        for p in &plans[1..] {
            let output = stream
                .alloc_zeros::<u8>(p[27] as usize)
                .map_err(|e| e.to_string())?;
            let params = stream.clone_htod(p).map_err(|e| e.to_string())?;
            stages.push(ResidentStage { output, params });
        }
        stream.synchronize().map_err(|e| e.to_string())?;
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
            let _ = self.first.slots[0].stream.synchronize();
            let _ = self.first.slots[1].stream.synchronize();
        }
        result
    }
    pub(super) fn upload(&mut self, input: &[u8]) -> Result<(), String> {
        self.check()?;
        if input.len() != self.first.input_len() {
            return Err("CUDA resident input length mismatch".into());
        }
        let slot = &mut self.first.slots[0];
        slot.host_in.as_mut_slice().copy_from_slice(input);
        let result = slot
            .stream
            .memcpy_htod(&slot.host_in, &mut slot.input)
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
        let stream = self.first.slots[0].stream.clone();
        launch_resident(
            &stream,
            &self.first.kernel,
            &self.first.slots[0].input,
            &mut self.first.slots[0].output,
            &self.first.params,
        )?;
        for i in 0..self.stages.len() {
            let (previous, following) = self.stages.split_at_mut(i);
            let input = previous
                .last()
                .map_or(&self.first.slots[0].output, |stage| &stage.output);
            let stage = &mut following[0];
            launch_resident(
                &stream,
                &self.first.kernel,
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
        let source_len = self
            .stages
            .last()
            .map_or(self.first.slots[0].output.len(), |s| s.output.len());
        if output.len() != source_len {
            return Err("CUDA resident output length mismatch".into());
        }
        let host_out_len = self.first.slots[0].host_out.len();
        if source_len != host_out_len {
            let result = {
                let source = self
                    .stages
                    .last()
                    .map_or(&self.first.slots[0].output, |s| &s.output);
                self.first.slots[0]
                    .stream
                    .memcpy_dtoh(source, output)
                    .map_err(|e| e.to_string())
                    .and_then(|()| {
                        self.first.slots[0]
                            .stream
                            .synchronize()
                            .map_err(|e| e.to_string())
                    })
            };
            self.finish(result)?;
            self.phase = Phase::Empty;
            self.transfers.downloads += 1;
            self.transfers.download_bytes += output.len() as u64;
            return Ok(());
        }
        let result = {
            let source = self
                .stages
                .last()
                .map_or(&self.first.slots[0].output, |s| &s.output);
            self.first.slots[0]
                .stream
                .memcpy_dtoh(source, &mut self.first.slots[0].host_out)
                .map_err(|e| e.to_string())
                .and_then(|()| {
                    self.first.slots[0]
                        .stream
                        .synchronize()
                        .map_err(|e| e.to_string())
                })
        };
        self.finish(result)?;
        output.copy_from_slice(self.first.slots[0].host_out.as_slice());
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
    // SAFETY: validate_chain validates ABI; distinct buffers; one stream orders stages.
    unsafe { args.launch(config) }.map_err(|e| e.to_string())?;
    Ok(())
}
