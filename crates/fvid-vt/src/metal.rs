//! CoreVideo images and page-aligned UMA shared memory mapped into the same Metal device used by wgpu.
use super::*;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_metal::{
    MTLBuffer, MTLDevice, MTLPixelFormat, MTLResourceOptions, MTLTexture, MTLTextureType,
};
use std::alloc::Layout;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};

/// Apple Silicon unified memory page size (16 KiB) required for `newBufferWithBytesNoCopy`.
pub const METAL_UMA_PAGE_BYTES: usize = 16_384;

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
    static kCVMetalTextureUsage: CFRef;
    static kCVPixelBufferIOSurfacePropertiesKey: CFRef;
    fn CVPixelBufferCreate(
        allocator: CFRef,
        width: usize,
        height: usize,
        format: u32,
        attributes: CFRef,
        out: *mut CFRef,
    ) -> i32;
}
struct Owned(CFRef);
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) }
        }
    }
}
// SAFETY: used only for immutable CVMetalTexture lifetime ownership and synchronized
// texture cache reuse across threads.
unsafe impl Send for Owned {}
unsafe impl Sync for Owned {}

fn cached_texture_cache(raw_device: *const c_void) -> Result<Arc<Mutex<Owned>>, Error> {
    static CACHES: OnceLock<Mutex<HashMap<usize, Arc<Mutex<Owned>>>>> = OnceLock::new();
    let map = CACHES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = map
        .lock()
        .map_err(|_| Error("Metal texture cache registry poisoned".into()))?;
    let key = raw_device as usize;
    if let Some(existing) = guard.get(&key) {
        return Ok(existing.clone());
    }
    let mut cache = ptr::null();
    let code = unsafe {
        CVMetalTextureCacheCreate(
            ptr::null(),
            ptr::null(),
            raw_device,
            ptr::null(),
            &mut cache,
        )
    };
    if code != 0 || cache.is_null() {
        return Err(status("Metal texture cache", code));
    }
    let entry = Arc::new(Mutex::new(Owned(cache)));
    guard.insert(key, entry.clone());
    Ok(entry)
}

struct UmaAllocation {
    ptr: NonNull<u8>,
    layout: Layout,
    len: usize,
}
// SAFETY: page-aligned host allocation stays pinned at a stable virtual/physical
// UMA address until all CPU and Metal references drop.
unsafe impl Send for UmaAllocation {}
unsafe impl Sync for UmaAllocation {}
impl Drop for UmaAllocation {
    fn drop(&mut self) {
        // SAFETY: allocated with std::alloc::alloc_zeroed using `self.layout`.
        unsafe {
            std::alloc::dealloc(self.ptr.as_ptr(), self.layout);
        }
    }
}

/// Zero-copy Unified Memory Architecture (UMA) buffer shared between CPU and Apple Silicon GPU.
/// Backed by 16 KiB page-aligned physical RAM wrapped via `newBufferWithBytesNoCopy`
/// (`MTLResourceStorageModeShared`) and imported into `wgpu::Buffer` without bus transfers.
#[derive(Clone)]
pub struct UmaSharedBuffer {
    alloc: Arc<UmaAllocation>,
    buffer: wgpu::Buffer,
}

impl UmaSharedBuffer {
    /// Allocate a zero-copy UMA buffer of `len` logical bytes (rounded up to a 16 KiB page
    /// multiple for Metal's `newBufferWithBytesNoCopy` contract) and bind it to `device`.
    pub fn new(
        device: &wgpu::Device,
        len: usize,
        usage: wgpu::BufferUsages,
        label: Option<&str>,
    ) -> Result<Self, Error> {
        if len == 0 {
            return Err(Error("UMA shared buffer length must be nonzero".into()));
        }
        let aligned_len = len
            .checked_next_multiple_of(METAL_UMA_PAGE_BYTES)
            .ok_or_else(|| Error("UMA shared buffer size overflow".into()))?;
        if aligned_len as u64 > device.limits().max_buffer_size {
            return Err(Error(
                "UMA shared buffer exceeds device max_buffer_size".into(),
            ));
        }
        let layout = Layout::from_size_align(aligned_len, METAL_UMA_PAGE_BYTES)
            .map_err(|_| Error("invalid UMA page alignment layout".into()))?;
        let raw_ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        let ptr = NonNull::new(raw_ptr)
            .ok_or_else(|| Error("UMA page-aligned memory allocation failed".into()))?;
        let alloc = Arc::new(UmaAllocation { ptr, layout, len });
        // SAFETY: `alloc.ptr` is 16 KiB page-aligned, `aligned_len` is a multiple of 16 KiB,
        // and the `Arc<UmaAllocation>` is kept alive until the wgpu/Metal buffer is dropped
        // or the `UmaSharedBuffer` is dropped (whichever happens last).
        let buffer = unsafe {
            let hal = device
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or_else(|| Error("UMA shared buffer requires a Metal wgpu device".into()))?;
            let raw_device = hal.raw_device();
            let nn = NonNull::new_unchecked(alloc.ptr.as_ptr().cast::<c_void>());
            let mtl_buffer: Retained<ProtocolObject<dyn MTLBuffer>> = raw_device
                .newBufferWithBytesNoCopy_length_options_deallocator(
                    nn,
                    aligned_len,
                    MTLResourceOptions::StorageModeShared,
                    None,
                )
                .ok_or_else(|| {
                    Error("Metal newBufferWithBytesNoCopy rejected UMA page allocation".into())
                })?;
            let hal_buffer = wgpu::hal::metal::Device::buffer_from_raw(
                mtl_buffer,
                aligned_len as u64,
            );
            device.create_buffer_from_hal::<wgpu::hal::api::Metal>(
                hal_buffer,
                &wgpu::BufferDescriptor {
                    label,
                    size: aligned_len as u64,
                    usage,
                    mapped_at_creation: false,
                },
            )
        };
        Ok(Self { alloc, buffer })
    }

    /// Logical payload length requested by the caller.
    pub fn len(&self) -> usize {
        self.alloc.len
    }

    pub fn is_empty(&self) -> bool {
        self.alloc.len == 0
    }

    /// Page-aligned capacity visible to Metal (`multiple of 16384`).
    pub fn aligned_capacity(&self) -> usize {
        self.alloc.layout.size()
    }

    /// Underlying `wgpu::Buffer` backed by the same physical RAM pages.
    pub fn wgpu_buffer(&self) -> &wgpu::Buffer {
        &self.buffer
    }

    /// Write bytes directly into unified RAM without staging or bus transfers.
    /// Any trailing bytes up to the 4-byte word boundary are zeroed.
    pub fn write_slice(&self, src: &[u8]) -> Result<(), Error> {
        if src.len() > self.alloc.len {
            return Err(Error("source slice exceeds UMA buffer length".into()));
        }
        // SAFETY: caller serializes host writes before submitting GPU commands on the queue.
        unsafe {
            let dst = self.alloc.ptr.as_ptr();
            ptr::copy_nonoverlapping(src.as_ptr(), dst, src.len());
            let padded_end = src.len().next_multiple_of(4).min(self.alloc.layout.size());
            if padded_end > src.len() {
                ptr::write_bytes(dst.add(src.len()), 0, padded_end - src.len());
            }
        }
        Ok(())
    }

    /// Read bytes directly from unified RAM after GPU queue synchronization (`device.poll(Wait)`).
    pub fn read_slice(&self, dst: &mut [u8]) -> Result<(), Error> {
        if dst.len() > self.alloc.len {
            return Err(Error("destination slice exceeds UMA buffer length".into()));
        }
        // SAFETY: caller polls the Metal device until submission completion before reading.
        unsafe {
            ptr::copy_nonoverlapping(self.alloc.ptr.as_ptr(), dst.as_mut_ptr(), dst.len());
        }
        Ok(())
    }
}

/// Native NV12 or P010 planes. NV12 samples are normalized eight-bit values.
/// For P010, Samples are normalized MSB-aligned ten-bit words:
/// multiply by 65535 / 64 to recover the source ten-bit sample.
pub struct MetalPlanes {
    pub y: wgpu::Texture,
    /// Cb in red, Cr in green.
    pub uv: wgpu::Texture,
}
impl Surface {
    /// Render both native encoder planes without exposing writable texture
    /// handles. The callback records one pass for luma (0) and chroma (1).
    /// Its shaders must write normalized NV12 or MSB-aligned P010 code values.
    pub fn metal_render(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: usize,
        height: usize,
        depth: u8,
        full_range: bool,
        mut draw: impl for<'a> FnMut(&mut wgpu::RenderPass<'a>, usize),
    ) -> Result<Self, Error> {
        let surface = Self::allocate_metal(device, width, height, depth, full_range)?;
        let planes = surface.import_metal_usage(device, true)?;
        let mut encoder = device.create_command_encoder(&Default::default());
        for (index, texture) in [&planes.y, &planes.uv].into_iter().enumerate() {
            let value = if index == 0 {
                if full_range {
                    0.0
                } else if depth == 8 {
                    16.0 / 255.0
                } else {
                    4096.0 / 65535.0
                }
            } else if depth == 8 {
                128.0 / 255.0
            } else {
                32768.0 / 65535.0
            };
            let view = texture.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("render native encoder plane"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: value,
                            g: value,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            draw(&mut pass, index);
        }
        let submission = queue.submit([encoder.finish()]);
        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission),
                timeout: Some(std::time::Duration::from_secs(30)),
            })
            .map_err(|e| Error(format!("Metal output rendering: {e}")))?;
        Ok(surface)
    }
    /// Allocate encoder-compatible NV12/P010 and initialize both planes on
    /// Metal. No host pixel allocation or upload; returns only after GPU work.
    pub fn metal_black(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: usize,
        height: usize,
        depth: u8,
        full_range: bool,
    ) -> Result<Self, Error> {
        Self::metal_render(device, queue, width, height, depth, full_range, |_, _| {})
    }
    fn allocate_metal(
        device: &wgpu::Device,
        width: usize,
        height: usize,
        depth: u8,
        full_range: bool,
    ) -> Result<Self, Error> {
        if depth == 10
            && !device.features().contains(
                wgpu::Features::TEXTURE_FORMAT_16BIT_NORM
                    | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES,
            )
        {
            return Err(Error("P010 output rendering requires normalized16 and adapter-specific texture format features".into()));
        }
        if width == 0
            || height == 0
            || width > device.limits().max_texture_dimension_2d as usize
            || height > device.limits().max_texture_dimension_2d as usize
        {
            return Err(Error("invalid Metal output dimensions".into()));
        }
        let format = match (depth, full_range) {
            (8, false) => NV12_VIDEO,
            (8, true) => NV12_FULL,
            (10, false) => P010_VIDEO,
            (10, true) => P010_FULL,
            _ => return Err(Error("Metal output supports NV12 or P010".into())),
        };
        let surface = unsafe {
            let iosurface = Owned(CFDictionaryCreate(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            ));
            if iosurface.0.is_null() {
                return Err(Error("IOSurface attributes allocation failed".into()));
            }
            let attributes = Owned(CFDictionaryCreate(
                ptr::null(),
                [
                    kCVPixelBufferMetalCompatibilityKey,
                    kCVPixelBufferIOSurfacePropertiesKey,
                ]
                .as_ptr(),
                [kCFBooleanTrue, iosurface.0].as_ptr(),
                2,
                &kCFTypeDictionaryKeyCallBacks,
                &kCFTypeDictionaryValueCallBacks,
            ));
            if attributes.0.is_null() {
                return Err(Error("output attributes allocation failed".into()));
            }
            let mut image = ptr::null();
            let code =
                CVPixelBufferCreate(ptr::null(), width, height, format, attributes.0, &mut image);
            let image = Owned(image);
            if code != 0 || image.0.is_null() {
                return Err(status("Metal output allocation", code));
            }
            // The uninitialized image stays private until both render clears finish.
            Self::retain(image.0)?
        };
        Ok(surface)
    }
    /// Map a retained NV12 or P010 image without locking, downloading or uploading its
    /// pixels. P010 requires TEXTURE_FORMAT_16BIT_NORM; NV12 needs no optional feature.
    pub fn import_metal(&self, device: &wgpu::Device) -> Result<MetalPlanes, Error> {
        self.import_metal_usage(device, false)
    }
    fn import_metal_usage(
        &self,
        device: &wgpu::Device,
        render: bool,
    ) -> Result<MetalPlanes, Error> {
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
        // SAFETY: the cache is reused per wgpu Metal device. The image is retained.
        // Public imports follow synchronous decode and are read-only. Internal output
        // imports remain private during render initialization and GPU completion. Both
        // paths keep the CVMetalTexture, cache, and source alive until HAL releases the resource.
        unsafe {
            let hal = device
                .as_hal::<wgpu::hal::api::Metal>()
                .ok_or_else(|| Error("P010 surface import requires a Metal device".into()))?;
            let cache_entry = cached_texture_cache(Retained::as_ptr(hal.raw_device()).cast())?;
            let cache_guard = cache_entry
                .lock()
                .map_err(|_| Error("Metal texture cache lock poisoned".into()))?;
            let usage = 5i32; // MTLTextureUsageShaderRead | MTLTextureUsageRenderTarget
            let number = render.then(|| {
                Owned(CFNumberCreate(
                    ptr::null(),
                    CF_NUMBER_SINT32,
                    (&usage as *const i32).cast(),
                ))
            });
            if number.as_ref().is_some_and(|n| n.0.is_null()) {
                return Err(Error("Metal usage allocation failed".into()));
            }
            let attributes = number.as_ref().map(|n| {
                Owned(CFDictionaryCreate(
                    ptr::null(),
                    [kCVMetalTextureUsage].as_ptr(),
                    [n.0].as_ptr(),
                    1,
                    &kCFTypeDictionaryKeyCallBacks,
                    &kCFTypeDictionaryValueCallBacks,
                ))
            });
            if attributes.as_ref().is_some_and(|a| a.0.is_null()) {
                return Err(Error("Metal attributes allocation failed".into()));
            }
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
                    cache_guard.0,
                    self.0.image,
                    attributes.as_ref().map_or(ptr::null(), |a| a.0),
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
                let cache_keepalive = cache_entry.clone();
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
                        drop(cache_keepalive);
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
                        usage: wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_SRC
                            | if render {
                                wgpu::TextureUsages::RENDER_ATTACHMENT
                            } else {
                                wgpu::TextureUsages::empty()
                            },
                        view_formats: &[],
                    },
                    wgpu::TextureUses::RESOURCE,
                ));
            }
            drop(cache_guard);
            let uv = textures.pop().unwrap();
            let y = textures.pop().unwrap();
            Ok(MetalPlanes { y, uv })
        }
    }
}
