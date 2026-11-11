use super::{Player, clock::times};
use crate::playback_thread::{Frame, Pixels};
use eframe::egui::{Rect, Vec2};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

/// Owned source projection and initial viewing direction.
#[derive(Clone, Copy)]
pub(super) struct SphericalView {
    pub(super) enabled: bool,
    pub(super) projection: fvid_media::owned_spherical::SphericalProjection,
    pub(super) stereo: fvid_media::owned_spherical::SphericalStereoLayout,
    pub(super) yaw: i32,
    pub(super) pitch: i32,
    pub(super) roll: i32,
    pub(super) fov: i32,
}
impl Default for SphericalView {
    fn default() -> Self {
        Self { stereo: fvid_media::owned_spherical::SphericalStereoLayout::Mono, enabled: false, projection: fvid_media::owned_spherical::SphericalProjection::Equirect, yaw: 0, pitch: 0, roll: 0, fov: 80_000 }
    }
}

/// Magnifications VLC's Zoom menu lists, in thousandths of the fitted size:
/// Quarter, Half, Original and Double.
pub(super) const ZOOMS: [u32; 4] = [250, 500, 1_000, 2_000];
/// The shapes VLC's Crop menu cycles, as width to height. Its first choice is
/// the one that crops nothing, so walking the menu returns to the whole
/// picture; the cinema ratios are kept as their stored numbers (`185:100`),
/// which is what the menu labels `1.85:1`.
pub(super) const CROPS: [(u32, u32); 11] = [
    NO_CROP,
    (16, 10),
    (16, 9),
    (4, 3),
    (185, 100),
    (221, 100),
    (235, 100),
    (239, 100),
    (5, 3),
    (5, 4),
    (1, 1),
];
/// The menu's Default: a picture shown whole.
pub(super) const NO_CROP: (u32, u32) = (0, 0);

/// Which shape the picture is drawn at: the one its container declares, one
/// VLC's aspect-ratio menu forces on it, or the window's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Aspect {
    /// The shape the file's own pixels add up to.
    #[default]
    Source,
    /// A width-to-height shape forced on the picture.
    Shape(u32, u32),
    /// VLC's `Fill Window`: the picture takes the shape of the frame.
    Fill,
}

/// The shapes VLC's aspect-ratio menu cycles, in the order it lists them. Its
/// first choice forces nothing, so walking the menu comes back to the file's own
/// shape; the cinema ratios are kept as their stored numbers (`235:100`), which
/// is what the menu labels `2.35:1`.
pub(super) const ASPECTS: [Aspect; 10] = [
    Aspect::Source,
    Aspect::Shape(16, 9),
    Aspect::Shape(4, 3),
    Aspect::Shape(1, 1),
    Aspect::Shape(16, 10),
    Aspect::Shape(221, 100),
    Aspect::Shape(235, 100),
    Aspect::Shape(239, 100),
    Aspect::Shape(5, 4),
    Aspect::Fill,
];

/// Move to the neighbouring VLC zoom rung, clamped at both ends. A
/// magnification that is not one of the menu's four (from `--zoom`, say) snaps
/// to the next rung rather than inventing an in-between one.
pub(super) fn zoom_step(current: u32, up: bool) -> u32 {
    if up {
        ZOOMS
            .iter()
            .copied()
            .find(|z| *z > current)
            .unwrap_or(*ZOOMS.last().unwrap())
    } else {
        ZOOMS
            .iter()
            .rev()
            .copied()
            .find(|z| *z < current)
            .unwrap_or(ZOOMS[0])
    }
}

/// Walk one step along VLC's Crop menu, wrapping at both ends: the menu starts
/// with the whole picture, so the key that crops also brings it back. A shape
/// the menu does not list (from `--crop`, say) has no place in it, so the walk
/// starts from the top rather than inventing one.
pub(super) fn crop_step(current: (u32, u32), forward: bool) -> (u32, u32) {
    let at = CROPS
        .iter()
        .position(|shape| *shape == current)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % CROPS.len()
    } else {
        (at + CROPS.len() - 1) % CROPS.len()
    };
    CROPS[next]
}

/// Walk one step along VLC's aspect-ratio menu, wrapping at both ends: the menu
/// starts at the file's own shape, so the key that forces one gives it back. A
/// shape the menu does not list (from `--aspect`, say) has no place in it, so
/// the walk starts from the top rather than inventing one.
pub(super) fn aspect_step(current: Aspect, forward: bool) -> Aspect {
    let at = ASPECTS
        .iter()
        .position(|shape| *shape == current)
        .unwrap_or(0);
    let next = if forward {
        (at + 1) % ASPECTS.len()
    } else {
        (at + ASPECTS.len() - 1) % ASPECTS.len()
    };
    ASPECTS[next]
}

/// The control line for a magnification change, which VLC's menu states the
/// same way: a multiple of the picture as fitted.
pub(super) fn zoom_osd(milli: u32) -> String {
    format!("Zoom {}", times(milli))
}

/// A crop shape the way VLC's menu writes it: `4:3`, and a decimal for the
/// cinema ratios it stores as hundredths (`185:100` is `1.85:1`).
pub(super) fn crop_label(shape: (u32, u32)) -> String {
    if shape == NO_CROP {
        return "Default".to_owned();
    }
    if shape.1 == 100 {
        return format!("{}.{:02}:1", shape.0 / 100, shape.0 % 100);
    }
    format!("{}:{}", shape.0, shape.1)
}

/// The control line for a crop change, naming the shape the picture keeps.
pub(super) fn crop_osd(shape: (u32, u32)) -> String {
    format!("Crop {}", crop_label(shape))
}

/// A forced shape the way VLC's menu writes it: `4:3`, a decimal for the cinema
/// ratios it stores as hundredths (`235:100` is `2.35:1`), and one word for
/// each of the two choices that name no shape.
pub(super) fn aspect_label(shape: Aspect) -> String {
    match shape {
        Aspect::Source => "Default".to_owned(),
        Aspect::Fill => "Fill".to_owned(),
        Aspect::Shape(x, 100) => format!("{}.{:02}:1", x / 100, x % 100),
        Aspect::Shape(x, y) => format!("{x}:{y}"),
    }
}

/// The control line for an aspect change, naming the shape the picture is drawn
/// at.
pub(super) fn aspect_osd(shape: Aspect) -> String {
    format!("Aspect {}", aspect_label(shape))
}


/// The size a picture of these coded pixels takes on screen: its height stands,
/// and its width follows the shape the container gives each pixel.
pub(super) fn display_size(coded: Vec2, pixel_aspect: (u32, u32)) -> Vec2 {
    Vec2::new(
        coded.x * pixel_aspect.0 as f32 / pixel_aspect.1 as f32,
        coded.y,
    )
}

/// The size the picture on screen takes: the shape its container declares for
/// its pixels, the shape VLC's aspect-ratio menu forces on it, or the shape of
/// the frame it is shown in. As in `display_size`, the height stands and the
/// width follows, and `video_rect` fits the result into the frame either way, so
/// what reaches the frame is the proportion.
pub(super) fn shown_size(coded: Vec2, pixel_aspect: (u32, u32), aspect: Aspect, frame: Vec2) -> Vec2 {
    match aspect {
        Aspect::Source => display_size(coded, pixel_aspect),
        Aspect::Shape(x, y) => Vec2::new(coded.y * x as f32 / y as f32, coded.y),
        Aspect::Fill => frame,
    }
}

/// Where the picture of a display size lands in the frame: the whole of it
/// fitted inside at 1×, and a magnification grown out of that centre, slid by
/// the drag that follows it. The slide only counts while the picture is bigger
/// than the frame, and never far enough to bare an edge the picture could have
/// filled — so a fitted picture keeps the letterbox the fitting left it with,
/// and a magnified one keeps the frame covered.
pub(super) fn video_rect(frame: Rect, size: Vec2, zoom_milli: u32, pan: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 || frame.width() <= 0.0 || frame.height() <= 0.0 {
        return Rect::from_center_size(frame.center(), Vec2::ZERO);
    }
    let fit = (frame.width() / size.x)
        .min(frame.height() / size.y)
        .max(0.0);
    let drawn = size * (fit * zoom_milli as f32 / 1_000.0);
    let room = (drawn - frame.size()).max(Vec2::ZERO) / 2.0;
    let offset = Vec2::new(pan.x.clamp(-room.x, room.x), pan.y.clamp(-room.y, room.y));
    Rect::from_center_size(frame.center() + offset, drawn)
}

/// The part of a picture VLC's Crop menu keeps: the largest centred region
/// whose displayed shape is the ratio, returned as the pixels cut from each
/// edge — left, top, right, bottom. The shape is the one you see, so a file
/// whose container stretches its pixels has the stretch counted first, and a
/// wider picture than the ratio loses its sides while a taller one loses its
/// top and bottom. The menu's Default cuts nothing.
pub(super) fn crop_insets(source: Vec2, pixel_aspect: (u32, u32), shape: (u32, u32)) -> [u32; 4] {
    if shape == NO_CROP {
        return [0, 0, 0, 0];
    }
    // The same shape measured in stored pixels: a pixel stretched wider needs
    // fewer of them to span the ratio.
    let wanted = f64::from(shape.0) / f64::from(shape.1) * f64::from(pixel_aspect.1)
        / f64::from(pixel_aspect.0);
    let stored = f64::from(source.x) / f64::from(source.y);
    let (keep_x, keep_y) = if stored > wanted {
        let width = f64::from(source.y) * wanted;
        (
            width.round().min(f64::from(source.x)).max(1.0) as u32,
            source.y as u32,
        )
    } else {
        let height = f64::from(source.x) / wanted;
        (
            source.x as u32,
            height.round().min(f64::from(source.y)).max(1.0) as u32,
        )
    };
    let (across, down) = (
        source.x as u32 - keep_x.min(source.x as u32),
        source.y as u32 - keep_y.min(source.y as u32),
    );
    // An odd remainder leaves the extra pixel on the far edge, so the region
    // stays centred as closely as whole pixels allow.
    [across / 2, down / 2, across - across / 2, down - down / 2]
}

/// The size a picture has left after cropping, in the pixels it is stored in.
pub(super) fn cropped_size(source: Vec2, insets: [u32; 4]) -> Vec2 {
    Vec2::new(
        source.x - insets[0] as f32 - insets[2] as f32,
        source.y - insets[1] as f32 - insets[3] as f32,
    )
}

/// Where the cropped part of a picture sits inside the whole of it, as texture
/// coordinates: the first two numbers are the corner the region starts at, the
/// last two the corner it ends at. Both planes of a frame are addressed by the
/// same fractions, however differently they are sampled.
pub(super) fn uv_window(source: Vec2, insets: [u32; 4]) -> [f32; 4] {
    let (x, y) = (source.x.max(1.0), source.y.max(1.0));
    [
        insets[0] as f32 / x,
        insets[1] as f32 / y,
        (x - insets[2] as f32) / x,
        (y - insets[3] as f32) / y,
    ]
}

/// The pixels to cut from each edge of a frame: the borders its container
/// itself states, and then the Crop menu's shape measured against the region
/// those borders leave, so the two cuts stack from the coded edges inward.
/// A frame the container's own borders cannot fit inside — one smaller than
/// the file described — is shown whole rather than cut to nothing.
pub(super) fn shown_insets(
    source: Vec2,
    base: [u32; 4],
    pixel_aspect: (u32, u32),
    shape: (u32, u32),
) -> [u32; 4] {
    let base = if f64::from(base[0]) + f64::from(base[2]) < f64::from(source.x.max(1.0))
        && f64::from(base[1]) + f64::from(base[3]) < f64::from(source.y.max(1.0))
    {
        base
    } else {
        [0; 4]
    };
    let inner = crop_insets(cropped_size(source, base), pixel_aspect, shape);
    [
        base[0] + inner[0],
        base[1] + inner[1],
        base[2] + inner[2],
        base[3] + inner[3],
    ]
}

/// The picture settings of VLC's Adjustments tab: a switch and five sliders
/// whose ranges and defaults are VLC's own (`adjust-hue` −180..180°,
/// `adjust-brightness` and `adjust-contrast` 0..2, `adjust-saturation` 0..3,
/// `adjust-gamma` 0.01..10, all of the last four defaulting to 1 and the hue
/// to 0). The values survive a disabled switch and a new item, as they
/// survive in VLC until its dialog turns them off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Adjust {
    pub(super) on: bool,
    pub(super) hue: f32,
    pub(super) brightness: f32,
    pub(super) contrast: f32,
    pub(super) saturation: f32,
    pub(super) gamma: f32,
}

impl Default for Adjust {
    fn default() -> Self {
        Self {
            on: false,
            hue: 0.0,
            brightness: 1.0,
            contrast: 1.0,
            saturation: 1.0,
            gamma: 1.0,
        }
    }
}

/// The bundle that leaves every pixel as it stands: the sliders' own defaults
/// with the switch off, one of each multiplier and no turn. Shared with the
/// GPU path, whose uniform carries the same five numbers.
pub(super) const ADJUST_IDENTITY: [f32; 5] = crate::player_gpu::IDENTITY_ADJUST;

/// The five numbers VLC's adjust filter precomputes from the sliders, as one
/// bundle the draw paths consume: the luma becomes
/// `gamma⁻¹(clip(lum + contrast·y))` on stored samples and the chroma pair
/// `sat·(cos·u + sin·v)` and `sat·(cos·v − sin·u)` around grey, which is what
/// the filter's tables and its fixed-point chroma math evaluate to. Disabled,
/// or at the dialog's defaults, the bundle is the identity and both paths
/// leave the pixels alone.
pub(super) fn adjust_scalars(adjust: &Adjust) -> [f32; 5] {
    if !adjust.on {
        return ADJUST_IDENTITY;
    }
    let hue = adjust.hue.to_radians();
    let sat = adjust.saturation;
    [
        adjust.contrast,
        (adjust.brightness - 1.0) * 255.0 + 128.0 * (1.0 - adjust.contrast),
        1.0 / adjust.gamma,
        sat * hue.cos(),
        sat * hue.sin(),
    ]
}

/// One pixel through the luma half of the bundle: the contrast and brightness
/// line, clamped to the sample range, then the gamma curve on top.
#[cfg(test)]
pub(super) fn adjust_luma(y: f32, s: &[f32; 5]) -> f32 {
    let t = (s[1] + s[0] * y).clamp(0.0, 255.0);
    if (s[2] - 1.0).abs() <= f32::EPSILON {
        t
    } else {
        (t / 255.0).powf(s[2]) * 255.0
    }
}

/// The picture of an RGB frame with the bundle applied — the software draw
/// path, whose pixels have already left the planes behind. They are stepped
/// back through BT.601 into luma and centred chroma, the two halves of the
/// bundle run over them, and the result returns as RGB, clamped like the
/// filter's tables clamp. Grey (chroma at zero and luma at either anchor)
/// behaves as it does in VLC: grey stays grey through a hue turn.
#[cfg(test)]
pub(crate) fn adjust_rgb(source: &[u8], out: &mut Vec<u8>, s: &[f32; 5]) {
    out.clear();
    out.reserve(source.len());
    for pixel in source.as_chunks::<3>().0.iter() {
        let (r, g, b) = (
            f32::from(pixel[0]),
            f32::from(pixel[1]),
            f32::from(pixel[2]),
        );
        // The converter's fixed-point 601 matrix solved back for the samples
        // it read: the luma row divided by the 255/219 stretch it rode, the
        // chroma rows scaled back to the 224-wide chroma range and recentred.
        let y = (0.299 * r + 0.587 * g + 0.114 * b) / 1.164 + 16.0;
        let cb = (-0.168_736 * r - 0.331_264 * g + 0.5 * b) * (224.0 / 255.0);
        let cr = (0.5 * r - 0.418_688 * g - 0.081_312 * b) * (224.0 / 255.0);
        let y = adjust_luma(y, s);
        // Both of VLC's two lines read the pair as it was stored, which is what
        // makes the turn a rotation: read the turned Cb back in the second line
        // and a 45° hue leaves the Cr row two thirds short, so the chroma lands
        // off the colour wheel rather than around it.
        let (cb_adj, cr_adj) = (s[3] * cb + s[4] * cr, s[3] * cr - s[4] * cb);
        let y = 1.164 * (y - 16.0);
        let r = y + 1.596 * cr_adj;
        let g = y - 0.391 * cb_adj - 0.813 * cr_adj;
        let b = y + 2.013 * cb_adj;
        out.extend([r, g, b].map(|v| v.round().clamp(0.0, 255.0) as u8));
    }
}

pub(super) fn display_rgb(frame: &Frame) -> crate::Result<Vec<u8>> {
        match &frame.pixels {
            #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
            Pixels::Surface(surface) => {
                let packed = crate::playback_native::surface_to_packed(&surface.surface, surface.colour)?.rotated(surface.rotation)?;
                let mut rgb = Vec::new();
                packed.to_rgb(&mut rgb, frame.dimensions[0] * frame.dimensions[1] * 3)?;
                if let Some(grade) = &surface.grade { grade.apply(&mut rgb); }
                Ok(rgb)
            },
            Pixels::Packed(planes, grade) => {
                let mut rgb = Vec::new();
                if frame.rotation != 0 {
                    let packed = planes.rotated(frame.rotation)?;
                    packed.to_rgb(&mut rgb, packed.frame.width * packed.frame.height * 3)?;
                } else {
                    planes.to_rgb(&mut rgb, planes.frame.width * planes.frame.height * 3)?;
                }
                if let Some(grade) = grade { grade.apply(&mut rgb); }
                Ok(rgb)
            }
            Pixels::Planar(planes, grade) => {
                let turned;
                let planes_ref = if frame.rotation != 0 {
                    turned = crate::playback_native::rotate_planar8(planes, frame.rotation);
                    &turned
                } else {
                    planes.as_ref()
                };
                let mut rgb = Vec::new();
                crate::playback_native::planar8_to_rgb(
                    planes_ref,
                    &mut rgb,
                    planes_ref.width * planes_ref.height * 3,
                )?;
                // The grade a plane picture kept for the shader is applied here
                // instead, and it is the same table read the same way: both
                // routes land on these bytes, which is what lets the snapshot
                // stand for what is on screen.
                if let Some(grade) = grade {
                    grade.apply(&mut rgb);
                }
                Ok(rgb)
            }
            Pixels::Rgb(rgb) => Ok(rgb.clone()),
            Pixels::Rgba(rgba) => crate::playback_native::rgba_to_rgb_on_black(rgba, usize::MAX),
        }
}

pub(super) fn pack_u32_rgb(rgb: &[u8]) -> Vec<u32> {
    rgb.chunks_exact(3)
        .map(|p| (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]))
        .collect()
}

pub(super) fn unpack_u32_rgb(pixels: &[u32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 3);
    for &p in pixels {
        out.push((p >> 16) as u8);
        out.push((p >> 8) as u8);
        out.push(p as u8);
    }
    out
}

pub(super) fn stereo_dimensions(dims: [usize; 2], mode: fvid_media::owned_spherical::PlayStereo3D) -> [usize; 2] {
    match mode {
        fvid_media::owned_spherical::PlayStereo3D::SbslAnaglyph
        | fvid_media::owned_spherical::PlayStereo3D::MonoLeft
        | fvid_media::owned_spherical::PlayStereo3D::MonoRight
            if dims[0] >= 2 =>
        {
            [dims[0] / 2, dims[1]]
        }
        fvid_media::owned_spherical::PlayStereo3D::AblAnaglyph if dims[1] >= 2 => [dims[0], dims[1] / 2],
        _ => dims,
    }
}

pub(super) fn validate_viewed_dimensions(
    dims: [usize; 2],
    stereo3d: fvid_media::owned_spherical::PlayStereo3D,
    spherical: SphericalView,
) -> crate::Result<[usize; 2]> {
    if stereo3d != fvid_media::owned_spherical::PlayStereo3D::Off {
        u32::try_from(dims[0]).map_err(|_| crate::invalid("stereo width overflow"))?;
        u32::try_from(dims[1]).map_err(|_| crate::invalid("stereo height overflow"))?;
    }
    let out = stereo_dimensions(dims, stereo3d);
    if spherical.enabled {
        let width = u32::try_from(out[0]).map_err(|_| crate::invalid("spherical width overflow"))?;
        let height =
            u32::try_from(out[1]).map_err(|_| crate::invalid("spherical height overflow"))?;
        let (eye_width, eye_height) = match spherical.stereo {
            fvid_media::owned_spherical::SphericalStereoLayout::Mono => (width, height),
            fvid_media::owned_spherical::SphericalStereoLayout::SideBySide => (width / 2, height),
            fvid_media::owned_spherical::SphericalStereoLayout::TopBottom => (width, height / 2),
        };
        if eye_width == 0 || eye_height == 0 {
            return Err(crate::invalid("spherical stereo frame has no complete eye"));
        }
    }
    Ok(out)
}

pub(super) fn gpu_view_params(
    stereo3d: fvid_media::owned_spherical::PlayStereo3D,
    spherical: SphericalView,
    aspect: f32,
) -> crate::player_gpu::GpuViewParams {
    let stereo_mode = match stereo3d {
        fvid_media::owned_spherical::PlayStereo3D::Off => 0.0,
        fvid_media::owned_spherical::PlayStereo3D::SbslAnaglyph => 1.0,
        fvid_media::owned_spherical::PlayStereo3D::AblAnaglyph => 2.0,
        fvid_media::owned_spherical::PlayStereo3D::MonoLeft => 3.0,
        fvid_media::owned_spherical::PlayStereo3D::MonoRight => 4.0,
    };
    let spherical_mode = if !spherical.enabled {
        0.0
    } else {
        match spherical.projection {
            fvid_media::owned_spherical::SphericalProjection::Equirect => 1.0,
            fvid_media::owned_spherical::SphericalProjection::DualFisheye => 2.0,
            fvid_media::owned_spherical::SphericalProjection::Cubemap => 3.0,
            fvid_media::owned_spherical::SphericalProjection::LittlePlanet => 4.0,
            fvid_media::owned_spherical::SphericalProjection::Eac => 5.0,
            fvid_media::owned_spherical::SphericalProjection::Panini => 6.0,
            fvid_media::owned_spherical::SphericalProjection::Cylindrical => 7.0,
            fvid_media::owned_spherical::SphericalProjection::Mercator => 8.0,
            fvid_media::owned_spherical::SphericalProjection::DualFisheyeTb => 9.0,
            fvid_media::owned_spherical::SphericalProjection::Octahedral => 10.0,
            fvid_media::owned_spherical::SphericalProjection::Equisolid => 11.0,
            fvid_media::owned_spherical::SphericalProjection::Orthographic => 12.0,
            fvid_media::owned_spherical::SphericalProjection::Gnomonic => 13.0,
            fvid_media::owned_spherical::SphericalProjection::Sinusoidal => 14.0,
            fvid_media::owned_spherical::SphericalProjection::Miller => 15.0,
            fvid_media::owned_spherical::SphericalProjection::AzimuthalEquidistant => 16.0,
            fvid_media::owned_spherical::SphericalProjection::Mollweide => 17.0,
            fvid_media::owned_spherical::SphericalProjection::Hammer => 18.0,
            fvid_media::owned_spherical::SphericalProjection::StereographicFisheye => 19.0,
        }
    };
    let spherical_stereo = match spherical.stereo {
        fvid_media::owned_spherical::SphericalStereoLayout::Mono => 0.0,
        fvid_media::owned_spherical::SphericalStereoLayout::SideBySide => 1.0,
        fvid_media::owned_spherical::SphericalStereoLayout::TopBottom => 2.0,
    };
    crate::player_gpu::GpuViewParams {
        stereo_mode,
        spherical_mode,
        spherical_stereo,
        aspect: aspect.max(0.0001),
        yaw_rad: (fvid_media::owned_spherical::clamp_yaw_milli(spherical.yaw) as f32 / 1_000.0).to_radians(),
        pitch_rad: (fvid_media::owned_spherical::clamp_pitch_milli(spherical.pitch) as f32 / 1_000.0).to_radians(),
        roll_rad: (fvid_media::owned_spherical::clamp_roll_milli(spherical.roll) as f32 / 1_000.0).to_radians(),
        fov_rad: (fvid_media::owned_spherical::clamp_fov_milli(spherical.fov) as f32 / 1_000.0).to_radians(),
    }
}

pub(super) fn stereo_frame(frame: &mut Frame, mode: fvid_media::owned_spherical::PlayStereo3D) -> crate::Result<()> {
    if mode == fvid_media::owned_spherical::PlayStereo3D::Off { return Ok(()); }
    let rgb = display_rgb(frame)?;
    let packed = pack_u32_rgb(&rgb);
    let width = u32::try_from(frame.dimensions[0]).map_err(|_| crate::invalid("stereo width overflow"))?;
    let height = u32::try_from(frame.dimensions[1]).map_err(|_| crate::invalid("stereo height overflow"))?;
    let (width, height, pixels) = fvid_media::owned_spherical::apply_play_stereo3d(width, height, &packed, mode);
    frame.dimensions = [width as usize, height as usize];
    frame.pixels = Pixels::Rgb(unpack_u32_rgb(&pixels));
    Ok(())
}

pub(super) fn spherical_frame(frame: &mut Frame, view: SphericalView) -> crate::Result<()> {
    if !view.enabled { return Ok(()); }
    let rgb = display_rgb(frame)?;
    let source = pack_u32_rgb(&rgb);
    let width = u32::try_from(frame.dimensions[0]).map_err(|_| crate::invalid("spherical width overflow"))?;
    let height = u32::try_from(frame.dimensions[1]).map_err(|_| crate::invalid("spherical height overflow"))?;
    // A monoscopic display reads the left eye, retaining the display canvas.
    // Split rows before projection so longitude wraps within that eye only.
    let (eye_width, eye_height) = match view.stereo {
        fvid_media::owned_spherical::SphericalStereoLayout::Mono => (width, height),
        fvid_media::owned_spherical::SphericalStereoLayout::SideBySide => (width / 2, height),
        fvid_media::owned_spherical::SphericalStereoLayout::TopBottom => (width, height / 2),
    };
    if eye_width == 0 || eye_height == 0 {
        return Err(crate::invalid("spherical stereo frame has no complete eye"));
    }
    let eye: Vec<u32> = source.chunks_exact(width as usize).take(eye_height as usize)
        .flat_map(|row| row[..eye_width as usize].iter().copied()).collect();
    let pixels = fvid_media::owned_spherical::project_spherical_view(eye_width, eye_height, &eye, width, height, view.projection, view.yaw, view.pitch, view.roll, view.fov);
    frame.pixels = Pixels::Rgb(unpack_u32_rgb(&pixels));
    Ok(())
}

pub(super) fn prepare_view_frame(
    frame: &mut Frame,
    stereo3d: fvid_media::owned_spherical::PlayStereo3D,
    spherical: SphericalView,
) -> crate::Result<()> {
    if matches!(frame.pixels, Pixels::Rgb(_) | Pixels::Rgba(_)) {
        stereo_frame(frame, stereo3d).and_then(|()| spherical_frame(frame, spherical))
    } else {
        // Planar, Packed and hardware VideoToolbox Surface frames stay on the GPU
        // zero-copy path; stereo3d and 360° projection execute in `player_gpu.wgsl`.
        frame.dimensions = validate_viewed_dimensions(frame.dimensions, stereo3d, spherical)?;
        Ok(())
    }
}

/// Where a snapshot of `video` at a media time is written: beside the file,
/// under its own name, marked to the millisecond.
pub(super) fn snapshot_name(video: &Path, at: Duration) -> PathBuf {
    let total = at.as_millis();
    let (hours, minutes, seconds, millis) = (
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1_000 % 60,
        total % 1_000,
    );
    let stem = video
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    video.parent().unwrap_or(Path::new(".")).join(format!(
        "{stem}-{hours:02}h{minutes:02}m{seconds:02}s{millis:03}.png"
    ))
}

impl Player {
    pub(super) fn change_zoom(&mut self, up: bool) {
        self.zoom_milli = zoom_step(self.zoom_milli, up);
        self.show_osd(zoom_osd(self.zoom_milli));
    }

    /// Step along VLC's Crop menu and say which shape the picture keeps. What is
    /// cut away is gone from the picture rather than hidden behind an edge, so
    /// what is left is the part the frame now fills.
    pub(super) fn change_crop(&mut self, forward: bool) {
        self.crop = crop_step(self.crop, forward);
        self.show_osd(crop_osd(self.crop));
    }

    /// Step along VLC's aspect-ratio menu and say which shape the picture now
    /// takes. Nothing is cut: the same pixels are simply drawn wider or taller,
    /// which is why the menu's Default hands back the shape the file declared.
    pub(super) fn change_aspect(&mut self, forward: bool) {
        self.aspect = aspect_step(self.aspect, forward);
        self.show_osd(aspect_osd(self.aspect));
    }


    /// Write the picture on screen out beside the file it came from. Only the
    /// video is saved: the subtitle line and the controls are drawn over it and
    /// never reach the frame.
    pub(super) fn take_snapshot(&mut self) {
        let Some(path) = self.opened.clone() else {
            self.show_osd("Nothing to save");
            return;
        };
        if self.presented.is_none() {
            self.show_osd("Nothing to save");
            return;
        }
        let beside = snapshot_name(&path, self.timeline().0.unwrap_or_default());
        let target = self.snapshot_dir.as_ref().map_or_else(|| beside.clone(), |directory| {
            directory.join(beside.file_name().expect("snapshot name has a file component"))
        });
        let bytes = match self.picture() {
            Ok(bytes) => bytes,
            Err(error) => {
                self.show_osd(format!("Snapshot failed: {error}"));
                return;
            }
        };
        if let Some(directory) = &self.snapshot_dir
            && let Err(error) = std::fs::create_dir_all(directory)
        {
            self.show_osd(format!("Snapshot failed: {error}"));
            return;
        }
        match std::fs::write(&target, bytes) {
            Ok(()) => self.show_osd(format!(
                "Saved {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            )),
            Err(error) => self.show_osd(format!("Snapshot failed: {error}")),
        }
    }

    /// PNG bytes of the frame last shown, in whichever layout it arrived.
    pub(super) fn upload_frame(&mut self, frame: &Frame) {
                match &frame.pixels {
                    #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                    Pixels::Surface(surface) => {
                        self.video_packed = None; self.video = None; self.texture = None; self.rgb_frame = None;
                        self.video_surface = Some((surface.clone(), frame.serial));
                    },
                    Pixels::Packed(planes, grade) => {
                        self.video_packed = Some((planes.clone(), frame.serial, grade.clone(), frame.rotation));
                        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                        { self.video_surface = None; }
                        self.video = None;
                        self.texture = None;
                        self.rgb_frame = None;
                    }
                    Pixels::Planar(planes, grade) => {
                        self.video = Some((planes.clone(), frame.serial, grade.clone(), frame.rotation));
                        self.video_packed = None;
                        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                        { self.video_surface = None; }
                        self.texture = None;
                        self.rgb_frame = None;
                    }
                    Pixels::Rgb(rgb) | Pixels::Rgba(rgb) => {
                        // Keep immutable display codes; picture settings and
                        // custom effects run in the GPU callback on redraw.
                        self.texture = None;
                        self.rgb_frame = Some((Arc::new(rgb.clone()), frame.dimensions));
                        self.video = None;
                        self.video_packed = None;
                        #[cfg(all(target_os = "macos", feature = "videotoolbox"))]
                        { self.video_surface = None; }
                    }
                }
    }

    pub(super) fn redraw_view(&mut self) {
        if let Some(mut frame) = self.presented_source.clone() {
            match prepare_view_frame(&mut frame, self.stereo3d, self.spherical) {
                Ok(()) => {
                    self.upload_frame(&frame);
                    self.dimensions = frame.dimensions;
                    self.presented = Some(frame);
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }

    pub(super) fn look_spherical(&mut self, yaw: i32, pitch: i32, roll: i32) {
        self.spherical.yaw = ((i64::from(self.spherical.yaw) + i64::from(yaw) + 180_000).rem_euclid(360_000) - 180_000) as i32;
        self.spherical.pitch = (i64::from(self.spherical.pitch) + i64::from(pitch)).clamp(-90_000, 90_000) as i32;
        self.spherical.roll = ((i64::from(self.spherical.roll) + i64::from(roll) + 180_000).rem_euclid(360_000) - 180_000) as i32;
        self.redraw_view();
    }

    pub(super) fn change_spherical(&mut self, cycle: bool) {
        if cycle {
            self.spherical.projection = fvid_media::owned_spherical::cycle_spherical_projection(self.spherical.projection);
            self.spherical.enabled = true;
        } else {
            self.spherical.enabled = !self.spherical.enabled;
        }
        self.redraw_view();
        self.show_osd(if self.spherical.enabled {
            fvid_media::owned_spherical::format_spherical_projection_osd(self.spherical.projection)
        } else { "360° Off".into() });
    }

    pub(super) fn picture(&self) -> crate::Result<Vec<u8>> {
        let frame = self
            .presented
            .as_ref()
            .ok_or_else(|| crate::invalid("nothing has been shown yet"))?;
        if !matches!(frame.pixels, Pixels::Rgb(_) | Pixels::Rgba(_))
            && (self.stereo3d != fvid_media::owned_spherical::PlayStereo3D::Off || self.spherical.enabled)
        {
            if let Some(mut source) = self.presented_source.clone() {
                stereo_frame(&mut source, self.stereo3d)?;
                spherical_frame(&mut source, self.spherical)?;
                let rgb = display_rgb(&source)?;
                return crate::snapshot::png(source.dimensions[0], source.dimensions[1], &rgb);
            }
        }
        let rgb = display_rgb(frame)?;
        crate::snapshot::png(frame.dimensions[0], frame.dimensions[1], &rgb)
    }

}
