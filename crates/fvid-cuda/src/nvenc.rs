//! Direct NVENC driver preflight, independent of libavcodec.
//! ABI source: NVIDIA Video Codec SDK, NvEncodeAPIGetMaxSupportedVersion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NvencVersion {
    pub major: u32,
    pub minor: u32,
}
impl NvencVersion {
    fn unpack(value: u32) -> Result<Self, String> {
        let version = Self {
            major: value >> 4,
            minor: value & 15,
        };
        if version.major == 0 {
            return Err("NVENC driver returned an invalid API version".into());
        }
        Ok(version)
    }
    pub fn supports(self, required: Self) -> bool {
        (self.major, self.minor) >= (required.major, required.minor)
    }
}

/// Holds the NVIDIA library alive for later direct encoder session functions.
/// Loading and querying this object creates no encoder or CUDA context.
pub struct NvencApi {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    _library: libloading::Library,
    version: NvencVersion,
}
impl NvencApi {
    pub fn load() -> Result<Self, String> {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            #[cfg(target_os = "linux")]
            let name = "libnvidia-encode.so.1";
            #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
            let name = "nvEncodeAPI64.dll";
            #[cfg(all(target_os = "windows", target_pointer_width = "32"))]
            let name = "nvEncodeAPI.dll";
            // SAFETY: Only the fixed NVIDIA driver library name is loaded;
            // the caller cannot supply an arbitrary library path.
            let library = unsafe { libloading::Library::new(name) }
                .map_err(|e| format!("NVENC driver library {name} is unavailable: {e}"))?;
            // SAFETY: NVIDIA exports this symbol with the NVENCAPI calling
            // convention (system on Windows, C on Linux), a uint32_t output
            // pointer and a four-byte NVENCSTATUS result.
            let query = unsafe {
                library.get::<unsafe extern "system" fn(*mut u32) -> i32>(
                    b"NvEncodeAPIGetMaxSupportedVersion\0",
                )
            }
            .map_err(|e| format!("NVENC version entrypoint is unavailable: {e}"))?;
            let mut packed = 0;
            // SAFETY: The resolved function has the SDK ABI; packed is a live,
            // writable uint32_t and the library outlives this call.
            let status = unsafe { query(&mut packed) };
            let version = query_result(status, packed)?;
            Ok(Self {
                _library: library,
                version,
            })
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            Err("NVENC requires an NVIDIA driver on Linux or Windows".into())
        }
    }
    pub fn version(&self) -> NvencVersion {
        self.version
    }
    pub fn require(&self, version: NvencVersion) -> Result<(), String> {
        require_version(self.version, version)
    }
}
fn require_version(available: NvencVersion, version: NvencVersion) -> Result<(), String> {
    if version.major == 0 || version.minor > 15 {
        return Err("invalid requested NVENC API version".into());
    }
    if !available.supports(version) {
        return Err(format!(
            "NVENC API {}.{} requires a newer driver (available {}.{})",
            version.major, version.minor, available.major, available.minor
        ));
    }
    Ok(())
}

fn query_result(status: i32, packed: u32) -> Result<NvencVersion, String> {
    if status != 0 {
        return Err(format!(
            "NVENC API version query failed with status {status}"
        ));
    }
    NvencVersion::unpack(packed)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn driver_query_rejects_errors_and_zero_version() {
        assert!(query_result(15, 13 << 4).unwrap_err().contains("status 15"));
        assert!(query_result(0, 0).is_err());
        let version = query_result(0, (13 << 4) | 1).unwrap();
        assert_eq!(
            version,
            NvencVersion {
                major: 13,
                minor: 1
            }
        );
        assert!(
            require_version(
                version,
                NvencVersion {
                    major: 13,
                    minor: 0
                }
            )
            .is_ok()
        );
        assert!(
            require_version(
                version,
                NvencVersion {
                    major: 14,
                    minor: 0
                }
            )
            .is_err()
        );
        assert!(
            require_version(
                version,
                NvencVersion {
                    major: 13,
                    minor: 16
                }
            )
            .is_err()
        );
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    #[test]
    fn unsupported_host_does_not_load_nvenc() {
        assert!(NvencApi::load().is_err());
    }
}
