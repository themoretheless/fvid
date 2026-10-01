//! Resident CUDA chain with explicit host boundaries and a checked phase machine.
#[derive(Clone, Copy, Debug, Default)]
pub struct TransferStats {
    pub uploads: u64,
    pub downloads: u64,
    pub upload_bytes: u64,
    pub download_bytes: u64,
    pub filter_passes: u64,
}
pub struct CudaPipeline {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    inner: crate::native::Pipeline,
}
impl CudaPipeline {
    /// One optional CUDA C byte shader per transform, compiled once by NVRTC.
    /// Only upload/download cross host boundaries; each shader is fused with
    /// its crop/reflection pass. Source code must be trusted by the caller.
    pub fn with_shaders(
        plans: &[[u32; 32]],
        shaders: &[Option<&crate::ByteShader>],
        ordinal: usize,
        memory_limit: usize,
    ) -> Result<Self, String> {
        if shaders.len() != plans.len() {
            return Err("CUDA shader count must match transform count".into());
        }
        let bytes = validate_chain(plans, memory_limit)?;
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            crate::native::Pipeline::with_shaders(plans, shaders, ordinal, bytes)
                .map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = bytes;
            Err(crate::unsupported())
        }
    }
    pub fn new(plans: &[[u32; 32]], ordinal: usize, memory_limit: usize) -> Result<Self, String> {
        let bytes = validate_chain(plans, memory_limit)?;
        if ordinal > i32::MAX as usize {
            return Err("CUDA device ordinal exceeds the driver index range".into());
        }
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            crate::native::Pipeline::new(plans, ordinal, bytes).map(|inner| Self { inner })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = bytes;
            Err(crate::unsupported())
        }
    }
    pub fn device_name(&self) -> &str {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.device_name()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            "CUDA unavailable"
        }
    }
    pub fn controlled_memory_bytes(&self) -> usize {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.bytes
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            0
        }
    }
    pub fn transfers(&self) -> TransferStats {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.transfers
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            TransferStats::default()
        }
    }
    pub fn upload(&mut self, input: &[u8]) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.upload(input)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = input;
            Err(crate::unsupported())
        }
    }
    pub fn process(&mut self) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.process()
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err(crate::unsupported())
        }
    }
    pub fn download(&mut self, output: &mut [u8]) -> Result<(), String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.inner.download(output)
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = output;
            Err(crate::unsupported())
        }
    }
}
fn validate_chain(plans: &[[u32; 32]], limit: usize) -> Result<usize, String> {
    if plans.is_empty() || plans.len() > 256 {
        return Err("CUDA chain must contain 1..=256 transforms".into());
    }
    let mut bytes = 0usize;
    let mut add = |n| {
        bytes = bytes.checked_add(n).ok_or("CUDA chain memory overflow")?;
        Ok::<_, String>(())
    };
    let in0 = plans[0][26] as usize;
    let out0 = plans[0][27] as usize;
    let out_last = plans.last().unwrap()[27] as usize;
    add(in0)?; // caller host input
    add(out_last)?; // caller host output
    // Depth-2 processor slots: 2 × (pinned in/out + device in/out) for first-stage sizes.
    add(in0.checked_mul(4).ok_or("CUDA chain memory overflow")?)?;
    add(out0.checked_mul(4).ok_or("CUDA chain memory overflow")?)?;
    for (index, p) in plans.iter().enumerate() {
        crate::validate(p[26] as usize, p[27] as usize, p)?;
        if index > 0 && p[26] != plans[index - 1][27] {
            return Err("CUDA chain buffer lengths are discontinuous".into());
        }
        if index > 0 {
            // Extra device outputs for stages after the first (first outs are in the slots).
            add(p[27] as usize)?;
        }
        add(128)?;
    }
    if bytes > limit {
        return Err(format!(
            "CUDA resident buffers need {bytes} bytes, exceeding memory limit {limit}"
        ));
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_shader_count_before_device_access() {
        let error = CudaPipeline::with_shaders(&[crate::tests::params()], &[], 0, usize::MAX)
            .err()
            .unwrap();
        assert!(error.contains("shader count"));
    }
    #[test]
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[ignore = "requires NVIDIA driver, GPU and CUDA 13 NVRTC"]
    fn resident_cuda_shaders_fuse_geometry_and_keep_one_host_roundtrip() {
        let mut first = crate::tests::params();
        first[24] = 1;
        let mut second = [0; 32];
        second[..8].copy_from_slice(&[0, 0, 2, 0, 0, 2, 2, 0]);
        second[8..16].copy_from_slice(&[4, 4, 1, 0, 0, 1, 1, 0]);
        second[16..24].copy_from_slice(&[5, 5, 1, 0, 0, 1, 1, 0]);
        second[26] = 6;
        second[27] = 6;
        let negate = crate::ByteShader::new("__device__ unsigned int process_byte(unsigned int v, unsigned int p, unsigned int x, unsigned int y) { return 255 - v; }").unwrap();
        let coordinates = crate::ByteShader::new("__device__ unsigned int process_byte(unsigned int v, unsigned int p, unsigned int x, unsigned int y) { return v + p + 3*x + 5*y; }").unwrap();
        let mut pipeline = CudaPipeline::with_shaders(
            &[first, second],
            &[Some(&negate), Some(&coordinates)],
            0,
            usize::MAX,
        )
        .unwrap();
        let input: Vec<u8> = (0..24).collect();
        let mut output = vec![0; 6];
        pipeline.upload(&input).unwrap();
        pipeline.process().unwrap();
        pipeline.download(&mut output).unwrap();
        let expected = [(6, 0), (5, 3), (10, 5), (9, 8), (16, 1), (20, 2)]
            .map(|(source, offset)| (255u32 - u32::from(input[source]) + offset) as u8);
        assert_eq!(output, expected);
        let stats = pipeline.transfers();
        assert_eq!(
            (stats.uploads, stats.downloads, stats.filter_passes),
            (1, 1, 2)
        );
        assert_eq!((stats.upload_bytes, stats.download_bytes), (24, 6));
    }
    #[test]
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[ignore = "requires NVIDIA GPU, driver and CUDA 13 NVRTC"]
    fn sampling_shader_matches_independent_crop_and_reflection_reference() {
        let shader =
            crate::ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu")).unwrap();
        let input: Vec<u8> = (0..24).map(|i| i * 7).collect();
        for horizontal in [false, true] {
            for vertical in [false, true] {
                let mut plan = crate::tests::params();
                plan[24] = u32::from(horizontal);
                plan[25] = u32::from(vertical);
                let mut transformed = [0u8; 4];
                for y in 0..2 {
                    for x in 0..2 {
                        let sx = if horizontal { 1 - x } else { x };
                        let sy = if vertical { 1 - y } else { y };
                        transformed[y * 2 + x] = input[(sy + 1) * 4 + sx + 1];
                    }
                }
                let mut expected = vec![0u8; 6];
                for y in 0..2i32 {
                    for x in 0..2i32 {
                        let mut total = 0u32;
                        for dy in -1..=1 {
                            for dx in -1..=1 {
                                let sx = (x + dx).clamp(0, 1) as usize;
                                let sy = (y + dy).clamp(0, 1) as usize;
                                total += u32::from(transformed[sy * 2 + sx]);
                            }
                        }
                        expected[y as usize * 2 + x as usize] = (total / 9) as u8;
                    }
                }
                expected[4] = input[16];
                expected[5] = input[20];
                let mut pipeline =
                    CudaPipeline::with_shaders(&[plan], &[Some(&shader)], 0, usize::MAX).unwrap();
                let mut output = vec![0; 6];
                pipeline.upload(&input).unwrap();
                pipeline.process().unwrap();
                pipeline.download(&mut output).unwrap();
                assert_eq!(output, expected, "hflip={horizontal}, vflip={vertical}");
                assert_eq!(
                    (pipeline.transfers().uploads, pipeline.transfers().downloads),
                    (1, 1)
                );
            }
        }
    }
    #[test]
    fn budgets_every_resident_buffer_and_rejects_discontinuity() {
        let first = crate::tests::params(); // 24 -> 6
        let mut second = [0; 32];
        second[..8].copy_from_slice(&[0, 0, 2, 0, 0, 2, 2, 0]);
        second[8..16].copy_from_slice(&[4, 4, 1, 0, 0, 1, 1, 0]);
        second[16..24].copy_from_slice(&[5, 5, 1, 0, 0, 1, 1, 0]);
        second[26] = 6;
        second[27] = 6;
        // host in/out + 2 slots × (pin+dev)×(in+out) + stage1 device out + 2×128
        // 24+6 + 4*24 + 4*6 + 6 + 256 = 30 + 96 + 24 + 6 + 256 = 412
        let bytes = 24 + 6 + 4 * 24 + 4 * 6 + 6 + 256;
        assert_eq!(validate_chain(&[first, second], bytes).unwrap(), bytes);
        assert!(validate_chain(&[first, second], bytes - 1).is_err());
        assert!(validate_chain(&[first, first], usize::MAX).is_err());
        assert!(validate_chain(&[], usize::MAX).is_err());
        second[24] = 2;
        assert!(validate_chain(&[first, second], usize::MAX).is_err());
    }
}
