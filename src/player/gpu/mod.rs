//! Video presentation on the GPU: the decoded Y, Cb and Cr planes are uploaded
//! as byte textures (R8 for 8-bit, RG8 for little-endian wider samples) and
//! converted to RGB at source precision in a fragment shader (Metal on
//! macOS through wgpu), so no CPU pass touches the pixels after decoding. The
//! shader holds the colour decision too when the grade is one lookup: the table
//! the CPU baked rides as a texture — the 3D grid of nodes, or the three byte
//! tables for a plan that keeps its channels apart — and every sample reads its
//! codes out of that same table.

mod lut3d;
mod metal_export;
mod renderer;
mod shader;

use eframe::{egui_wgpu, wgpu};
use std::sync::Arc;

pub use renderer::{PackedVideoCallback, RgbVideoCallback, VideoCallback, VideoGpu};
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use renderer::SurfaceVideoCallback;
pub use shader::ColorShader;
#[cfg(all(target_os = "macos", feature = "videotoolbox"))]
pub use metal_export::MetalEncoderRenderer;
#[cfg(test)]
use crate::color::{Grade, Interpolation, Lut3d, ShaderLook};
#[cfg(test)]
use crate::playback_native::{AvcColour, PackedPlanar, Planar8};
use renderer::{build, build_with_source};
use shader::SHADER;

/// The settings bundle that leaves every sample as it stands, mirroring VLC's
/// dialog defaults with its switch off: unity multipliers and no turn. The
/// player's sliders are folded into this shape before either draw path sees
/// them.
pub const IDENTITY_ADJUST: [f32; 5] = [1.0, 0.0, 1.0, 1.0, 0.0];

/// Byte width of the shader's `Params` uniform block.
pub const UNIFORM_BYTES: u64 = 192;

/// GPU-side Stereo3D and 360° spherical viewport parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuViewParams {
    pub stereo_mode: f32,
    pub spherical_mode: f32,
    pub spherical_stereo: f32,
    pub aspect: f32,
    pub yaw_rad: f32,
    pub pitch_rad: f32,
    pub roll_rad: f32,
    pub fov_rad: f32,
}

pub const IDENTITY_VIEW: GpuViewParams = GpuViewParams {
    stereo_mode: 0.0,
    spherical_mode: 0.0,
    spherical_stereo: 0.0,
    aspect: 1.0,
    yaw_rad: 0.0,
    pitch_rad: 0.0,
    roll_rad: 0.0,
    fov_rad: 1.3962634, // 80° in radians
};

impl Default for GpuViewParams {
    fn default() -> Self {
        IDENTITY_VIEW
    }
}


/// Explicit GPU renderer selection. Software adapters are excluded and the
/// requested API/device cannot silently fall back to another API or the CPU.
pub fn configuration(
    backend: crate::Backend,
    ordinal: usize,
) -> crate::Result<egui_wgpu::WgpuConfiguration> {
    use crate::Backend;
    let backends = match backend {
        Backend::Auto => wgpu::Backends::PRIMARY | wgpu::Backends::GL,
        Backend::Metal => wgpu::Backends::METAL,
        Backend::Vulkan => wgpu::Backends::VULKAN,
        Backend::Dx12 => wgpu::Backends::DX12,
        Backend::Gl => wgpu::Backends::GL,
        _ => {
            return Err(crate::invalid(
                "player renderer requires auto, metal, vulkan, dx12 or gl; CUDA is a processing/codec backend",
            ));
        }
    };
    if backend == Backend::Auto && ordinal != 0 {
        return Err(crate::invalid(
            "--device requires an explicit player backend when not zero",
        ));
    }
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = backends;
    #[cfg(target_os = "macos")]
    {
        let descriptor = Arc::clone(&setup.device_descriptor);
        setup.device_descriptor = Arc::new(move |adapter| {
            let mut descriptor = descriptor(adapter);
            if adapter.get_info().backend == wgpu::Backend::Metal {
                descriptor.required_features |=
                    adapter.features() & wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
            }
            descriptor
        });
    }
    setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
        let mut available: Vec<_> = adapters
            .iter()
            .filter(|adapter| {
                adapter.get_info().device_type != wgpu::DeviceType::Cpu
                    && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
            })
            .collect();
        available.sort_by_key(|adapter| {
            let info = adapter.get_info();
            (info.name, info.vendor, info.device)
        });
        available
            .get(ordinal)
            .map(|adapter| (*adapter).clone())
            .ok_or_else(|| {
                format!("player backend {backend} hardware device {ordinal} unavailable")
            })
    }));
    Ok(egui_wgpu::WgpuConfiguration {
        wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
        ..Default::default()
    })
}

pub fn install_with_shader(
    state: &egui_wgpu::RenderState,
    shader: Option<&ColorShader>,
) -> crate::Result<()> {
    let validation = state.device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = state.device.push_error_scope(wgpu::ErrorFilter::Internal);
    let oom = state
        .device
        .push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let gpu = build_with_source(
        &state.device,
        state.target_format,
        shader.map_or(SHADER, ColorShader::compiled_source),
    );
    for scope in [oom, internal, validation] {
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(crate::Error::Gpu(error.to_string()));
        }
    }
    state.renderer.write().callback_resources.insert(gpu);
    Ok(())
}

/// Create the pipeline once and register it with the renderer.
pub fn install(state: &egui_wgpu::RenderState) {
    let gpu = build(&state.device, state.target_format);
    state.renderer.write().callback_resources.insert(gpu);
}

#[cfg(test)]
mod tests;
