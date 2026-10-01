//! CUDA C point shaders fused into the validated planar transform kernel.
#[derive(Clone, Debug)]
pub struct ByteShader {
    source: String,
    sampling: bool,
}
impl ByteShader {
    /// Define `__device__ unsigned int process_byte(unsigned int value,
    /// unsigned int plane, unsigned int x, unsigned int y)` in CUDA C.
    /// Coordinates are in the output plane, before crop/reflection sampling.
    /// Source is executable device code; callers must trust it.
    pub fn new(source: impl Into<String>) -> Result<Self, String> {
        let source = source.into();
        if source.trim().is_empty() || source.len() > 64 * 1024 || source.contains('\0') {
            return Err("CUDA shader must contain 1..=65536 bytes without NUL".into());
        }
        Ok(Self {
            source,
            sampling: false,
        })
    }
    /// As `new`, with a fifth `FvidSampler` argument. Use
    /// `sample(sampler, x, y)` to read the current transformed input plane.
    /// Signed 64-bit coordinates clamp at plane edges before crop/reflections.
    pub fn with_sampling(source: impl Into<String>) -> Result<Self, String> {
        let mut shader = Self::new(source)?;
        shader.sampling = true;
        Ok(shader)
    }
    pub fn source(&self) -> &str {
        &self.source
    }
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    pub(crate) fn p010_source(&self) -> String {
        let mut kernel = include_str!("p010.cu").to_owned();
        for (load, plane, coords) in [
            ("src_y[sy * src_pitch_y + sx]", 0, "x, y"),
            ("src_uv[src_i]", 1, "x >> 1, y >> 1"),
            ("src_uv[src_i + 1u]", 2, "x >> 1, y >> 1"),
        ] {
            let sampler = if self.sampling {
                format!(", FvidSampler{{src_y, src_uv, params, {plane}u}}")
            } else {
                String::new()
            };
            kernel = kernel.replace(
                &format!("({load} >> 6) << 6"),
                &format!("(process_byte({load} >> 6, {plane}u, {coords}{sampler}) & 1023u) << 6"),
            );
        }
        let preamble = if self.sampling {
            include_str!("sampler_p010.cuh")
        } else {
            ""
        };
        format!("{preamble}\n{}\n{kernel}", self.source)
    }
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    pub(crate) fn nv12_source(&self) -> Result<String, String> {
        let mut kernel = include_str!("nv12.cu")
            .replace("dst_y[y * dst_pitch_y + x] = src_y[sy * src_pitch_y + sx];",
                "dst_y[y * dst_pitch_y + x] = (unsigned char)process_byte(src_y[sy * src_pitch_y + sx], 0u, x, y);")
            .replace("dst_uv[dst_i] = src_uv[src_i];",
                "dst_uv[dst_i] = (unsigned char)process_byte(src_uv[src_i], 1u, x >> 1, y >> 1);")
            .replace("dst_uv[dst_i + 1u] = src_uv[src_i + 1u];",
                "dst_uv[dst_i + 1u] = (unsigned char)process_byte(src_uv[src_i + 1u], 2u, x >> 1, y >> 1);");
        let preamble = if self.sampling {
            kernel = kernel
                .replace(
                    "0u, x, y);",
                    "0u, x, y, FvidSampler{src_y, src_uv, params, 0u});",
                )
                .replace(
                    "1u, x >> 1, y >> 1);",
                    "1u, x >> 1, y >> 1, FvidSampler{src_y, src_uv, params, 1u});",
                )
                .replace(
                    "2u, x >> 1, y >> 1);",
                    "2u, x >> 1, y >> 1, FvidSampler{src_y, src_uv, params, 2u});",
                );
            include_str!("sampler_nv12.cuh")
        } else {
            ""
        };
        Ok(format!("{preamble}\n{}\n{kernel}", self.source))
    }
    #[cfg(any(target_os = "linux", target_os = "windows", test))]
    pub(crate) fn kernel_source(&self) -> String {
        let call = if self.sampling {
            "output[index] = (unsigned char)process_byte((unsigned int)input[source], plane, local % p[5], local / p[5], FvidSampler{input, params, plane});"
        } else {
            "output[index] = (unsigned char)process_byte((unsigned int)input[source], plane, local % p[5], local / p[5]);"
        };
        let kernel = include_str!("transform.cu").replace("output[index] = input[source];", call);
        let preamble = if self.sampling {
            include_str!("sampler.cuh")
        } else {
            ""
        };
        format!("{preamble}\n{}\n{kernel}", self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires clang++ C++14; host qualification does not execute CUDA"]
    fn p010_generated_sampling_kernel_matches_independent_reference() {
        let shader =
            ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu")).unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("fvid-p010-{}-{nonce}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let source = format!(
            "#define __device__\n#define __global__\nstruct Index {{ unsigned int x,y; }}; Index blockIdx{{0,0}}, blockDim{{32,16}}, threadIdx;\n{}\n{}",
            shader.p010_source(),
            include_str!("p010_reference.cpp")
        );
        std::fs::write(dir.join("reference.cpp"), source).unwrap();
        let compile = std::process::Command::new("clang++")
            .args(["-std=c++14", "-O2"])
            .arg(dir.join("reference.cpp"))
            .arg("-o")
            .arg(dir.join("reference"))
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );
        assert!(
            std::process::Command::new(dir.join("reference"))
                .status()
                .unwrap()
                .success()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn sampling_wrapper_supplies_current_input_and_plane() {
        let shader =
            ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu")).unwrap();
        let kernel = shader.kernel_source();
        assert!(kernel.find("struct FvidSampler").unwrap() < kernel.find("process_byte").unwrap());
        assert!(kernel.contains("FvidSampler{input, params, plane}"));
        assert!(kernel.contains("long long x, long long y"));
        assert!(ByteShader::with_sampling("x".repeat(65537)).is_err());
    }
    #[test]
    fn nv12_wrapper_filters_all_components_without_reinterpreting_chroma_pairs() {
        let shader = ByteShader::new(include_str!("../../../shaders/negate.cu")).unwrap();
        let kernel = shader.nv12_source().unwrap();
        assert!(kernel.contains("process_byte(src_y[sy * src_pitch_y + sx], 0u, x, y)"));
        assert!(kernel.contains("process_byte(src_uv[src_i], 1u, x >> 1, y >> 1)"));
        assert!(kernel.contains("process_byte(src_uv[src_i + 1u], 2u, x >> 1, y >> 1)"));
        let sampled = ByteShader::with_sampling(include_str!("../../../shaders/boxblur.cu"))
            .unwrap()
            .nv12_source()
            .unwrap();
        for plane in 0..3 {
            assert!(sampled.contains(&format!("FvidSampler{{src_y, src_uv, params, {plane}u}}")));
        }
    }
    #[test]
    fn validates_source_and_keeps_transform_bounds_in_wrapper() {
        assert!(ByteShader::new("").is_err());
        assert!(ByteShader::new("x\0y").is_err());
        assert!(ByteShader::new("x".repeat(65537)).is_err());
        let shader = ByteShader::new("__device__ unsigned int process_byte(unsigned int v, unsigned int p, unsigned int x, unsigned int y) { return 255 - v; }").unwrap();
        let kernel = shader.kernel_source();
        assert!(kernel.contains("if (index >= params[27]) return;"));
        assert!(kernel.contains(
            "process_byte((unsigned int)input[source], plane, local % p[5], local / p[5])"
        ));
    }
}
