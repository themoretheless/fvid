//! CoreVideo images mapped into the same Metal device used by wgpu.
use super::*;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureType};

#[link(name = "CoreVideo", kind = "framework")]
unsafe extern "C" {
    fn CVMetalTextureCacheCreate(
        allocator: CFRef,
        attributes: CFRef,
        device: *const c_void,
        texture_attributes: CFRef,
        out: *mut CFRef,
    ) -> i32;
    fn CVMetalTextureCacheCreateTextureFromImage(
        allocator: CFRef,
        cache: CFRef,
        image: CFRef,
        attributes: CFRef,
        format: MTLPixelFormat,
        width: usize,
        height: usize,
        plane: usize,
        out: *mut CFRef,
    ) -> i32;
    fn CVMetalTextureGetTexture(texture: CFRef) -> *mut ProtocolObject<dyn MTLTexture>;
}
struct Owned(CFRef);
impl Drop for Owned {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}
// SAFETY: used only for immutable CVMetalTexture lifetime ownership, never for
// concurrent operations on a texture cache.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

/// Native NV12 or P010 planes. NV12 samples are normalized eight-bit values.
/// For P010, Samples are normalized MSB-aligned ten-bit words:
/// multiply by 65535 / 64 to recover the source ten-bit sample.
pub struct MetalPlanes {
    pub y: wgpu::Texture,
    /// Cb in red, Cr in green.
    pub uv: wgpu::Texture,
}
impl Surface {
    /// Map a retained NV12 or P010 image without locking, downloading or uploading its
    /// pixels. P010 requires TEXTURE_FORMAT_16BIT_NORM; NV12 needs no optional feature.
    pub fn import_metal(&self, device: &wgpu::Device) -> Result<MetalPlanes, Error> {
        let format = unsafe { CVPixelBufferGetPixelFormatType(self.0.image) };
        let (y_format, uv_format, y_metal, uv_metal) = match format {
            NV12_VIDEO | NV12_FULL => (
                wgpu::TextureFormat::R8Unorm,
                wgpu::TextureFormat::Rg8Unorm,
                MTLPixelFormat::R8Unorm,
                MTLPixelFormat::RG8Unorm,
            ),
            P010_VIDEO | P010_FULL => {
                if !device
                    .features()
                    .contains(wgpu::Features::TEXTURE_FORMAT_16BIT_NORM)
                {
                    return Err(Error(
                        "Metal P010 import requires TEXTURE_FORMAT_16BIT_NORM".into(),
                    ));
                }
                (
                    wgpu::TextureFormat::R16Unorm,
                    wgpu::TextureFormat::Rg16Unorm,
                    MTLPixelFormat::R16Unorm,
                    MTLPixelFormat::RG16Unorm,
                )
            }
            _ => return Err(Error("Metal import requires NV12 or P010 surfaces".into())),
        };
        // SAFETY: the cache is created on precisely this wgpu Metal device. The
        // image is retained, synchronous decode completed, and each plane is
        // initialized. Imported resources are read-only and keep both the
        // CVMetalTexture and its source alive until HAL releases the resource.
        unsafe {
            let hal = device
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or_else(|| Error("P010 surface import requires a Metal device".into()))?;
            let mut cache = ptr::null();
            let code = CVMetalTextureCacheCreate(
                ptr::null(),
                ptr::null(),
                Retained::as_ptr(hal.raw_device()).cast(),
                ptr::null(),
                &mut cache,
            );
            if code != 0 || cache.is_null() {
                return Err(status("Metal texture cache", code));
            }
            let cache = Owned(cache);
            let mut textures = Vec::new();
            for (plane, format, metal_format, width, height) in [
                (0, y_format, y_metal, self.width(), self.height()),
                (
                    1,
                    uv_format,
                    uv_metal,
                    self.width().div_ceil(2),
                    self.height().div_ceil(2),
                ),
            ] {
                let size = wgpu::Extent3d {
                    width: u32::try_from(width)
                        .map_err(|_| Error("surface width overflow".into()))?,
                    height: u32::try_from(height)
                        .map_err(|_| Error("surface height overflow".into()))?,
                    depth_or_array_layers: 1,
                };
                if size.width > device.limits().max_texture_dimension_2d
                    || size.height > device.limits().max_texture_dimension_2d
                {
                    return Err(Error("surface exceeds Metal texture limits".into()));
                }
                let mut cv = ptr::null();
                let code = CVMetalTextureCacheCreateTextureFromImage(
                    ptr::null(),
                    cache.0,
                    self.0.image,
                    ptr::null(),
                    metal_format,
                    width,
                    height,
                    plane,
                    &mut cv,
                );
                if code != 0 || cv.is_null() {
                    return Err(status("Metal plane mapping", code));
                }
                let cv = Owned(cv);
                let texture = Retained::retain(CVMetalTextureGetTexture(cv.0))
                    .ok_or_else(|| Error("CoreVideo returned no Metal texture".into()))?;
                let surface = self.clone();
                let raw = wgpu::hal::metal::Device::texture_from_raw(
                    texture,
                    format,
                    MTLTextureType::Type2D,
                    1,
                    1,
                    size.into(),
                    Some(Box::new(move || {
                        drop(cv);
                        drop(surface);
                    })),
                );
                textures.push(device.create_texture_from_hal::<wgpu::hal::api::Metal>(
                    raw,
                    &wgpu::TextureDescriptor {
                        label: Some("VideoToolbox P010 plane"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::RESOURCE,
                ));
            }
            let uv = textures.pop().unwrap();
            let y = textures.pop().unwrap();
            Ok(MetalPlanes { y, uv })
        }
    }
}
