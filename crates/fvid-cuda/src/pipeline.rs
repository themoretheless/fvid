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
