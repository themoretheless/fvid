//! Spherical 360° camera projections and Stereo3D packing/anaglyph transforms.
type Result<T> = std::result::Result<T, String>;


/// Packed 3D layout → display view for `fvid play` (VLC-style stereo3d subset).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayStereo3D {
    #[default]
    Off,
    /// Side-by-side left|right → red/cyan anaglyph.
    SbslAnaglyph,
    /// Above-below left/right → red/cyan anaglyph.
    AblAnaglyph,
    /// Show left eye only (SBS half).
    MonoLeft,
    /// Show right eye only (SBS half).
    MonoRight,
}

pub fn cycle_play_stereo3d(mode: PlayStereo3D) -> PlayStereo3D {
    match mode {
        PlayStereo3D::Off => PlayStereo3D::SbslAnaglyph,
        PlayStereo3D::SbslAnaglyph => PlayStereo3D::AblAnaglyph,
        PlayStereo3D::AblAnaglyph => PlayStereo3D::MonoLeft,
        PlayStereo3D::MonoLeft => PlayStereo3D::MonoRight,
        PlayStereo3D::MonoRight => PlayStereo3D::Off,
    }
}

pub fn play_stereo3d_label(mode: PlayStereo3D) -> &'static str {
    match mode {
        PlayStereo3D::Off => "off",
        PlayStereo3D::SbslAnaglyph => "sbsl→anaglyph",
        PlayStereo3D::AblAnaglyph => "abl→anaglyph",
        PlayStereo3D::MonoLeft => "mono-left",
        PlayStereo3D::MonoRight => "mono-right",
    }
}

pub fn format_play_stereo3d_osd(mode: PlayStereo3D) -> String {
    format!("3D {}", play_stereo3d_label(mode))
}

fn anaglyph_rc(left: u32, right: u32) -> u32 {
    let lr = (left >> 16) & 0xff;
    let rg = (right >> 8) & 0xff;
    let rb = right & 0xff;
    (lr << 16) | (rg << 8) | rb
}

/// Convert packed stereo layout into a single-view RGB frame.
pub fn apply_play_stereo3d(
    width: u32,
    height: u32,
    pixels: &[u32],
    mode: PlayStereo3D,
) -> (u32, u32, Vec<u32>) {
    let w = width as usize;
    let h = height as usize;
    if matches!(mode, PlayStereo3D::Off) || w == 0 || h == 0 || pixels.len() < w * h {
        return (width, height, pixels.to_vec());
    }
    match mode {
        PlayStereo3D::SbslAnaglyph | PlayStereo3D::MonoLeft | PlayStereo3D::MonoRight if w >= 2 => {
            let out_w = w / 2;
            let mut out = vec![0u32; out_w * h];
            for y in 0..h {
                for x in 0..out_w {
                    let left = pixels[y * w + x];
                    let right = pixels[y * w + out_w + x];
                    out[y * out_w + x] = match mode {
                        PlayStereo3D::MonoLeft => left,
                        PlayStereo3D::MonoRight => right,
                        _ => anaglyph_rc(left, right),
                    };
                }
            }
            (out_w as u32, height, out)
        }
        PlayStereo3D::AblAnaglyph if h >= 2 => {
            let out_h = h / 2;
            let mut out = vec![0u32; w * out_h];
            for y in 0..out_h {
                for x in 0..w {
                    let left = pixels[y * w + x];
                    let right = pixels[(y + out_h) * w + x];
                    out[y * w + x] = anaglyph_rc(left, right);
                }
            }
            (width, out_h as u32, out)
        }
        _ => (width, height, pixels.to_vec()),
    }
}

/// 360° source layout / projection mode beyond plain equirect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SphericalProjection {
    #[default]
    Equirect,
    DualFisheye,
    Cubemap,
    LittlePlanet,
    /// YouTube Equi-Angular Cubemap (EAC).
    Eac,
    /// Panini projection (architectural / wide FOV 360).
    Panini,
    /// Cylindrical panorama.
    Cylindrical,
    /// Mercator map projection.
    Mercator,
    /// Dual fisheye top-bottom layout.
    DualFisheyeTb,
    /// Octahedral environment map.
    Octahedral,
    /// Equisolid fisheye viewport over equirect.
    Equisolid,
    /// Orthographic globe projection.
    Orthographic,
    /// Gnomonic (rectilinear tangent) projection.
    Gnomonic,
    /// Sinusoidal equal-area projection.
    Sinusoidal,
    /// Miller cylindrical projection.
    Miller,
    /// Azimuthal equidistant projection.
    AzimuthalEquidistant,
    /// Mollweide equal-area projection.
    Mollweide,
    /// Hammer azimuthal equal-area projection.
    Hammer,
    /// Stereographic fisheye, distinct from the little-planet remap.
    StereographicFisheye,
}

pub fn cycle_spherical_projection(mode: SphericalProjection) -> SphericalProjection {
    match mode {
        SphericalProjection::Equirect => SphericalProjection::DualFisheye,
        SphericalProjection::DualFisheye => SphericalProjection::Cubemap,
        SphericalProjection::Cubemap => SphericalProjection::LittlePlanet,
        SphericalProjection::LittlePlanet => SphericalProjection::Eac,
        SphericalProjection::Eac => SphericalProjection::Panini,
        SphericalProjection::Panini => SphericalProjection::Cylindrical,
        SphericalProjection::Cylindrical => SphericalProjection::Mercator,
        SphericalProjection::Mercator => SphericalProjection::DualFisheyeTb,
        SphericalProjection::DualFisheyeTb => SphericalProjection::Octahedral,
        SphericalProjection::Octahedral => SphericalProjection::Equisolid,
        SphericalProjection::Equisolid => SphericalProjection::Orthographic,
        SphericalProjection::Orthographic => SphericalProjection::Gnomonic,
        SphericalProjection::Gnomonic => SphericalProjection::Sinusoidal,
        SphericalProjection::Sinusoidal => SphericalProjection::Miller,
        SphericalProjection::Miller => SphericalProjection::AzimuthalEquidistant,
        SphericalProjection::AzimuthalEquidistant => SphericalProjection::Mollweide,
        SphericalProjection::Mollweide => SphericalProjection::Hammer,
        SphericalProjection::Hammer => SphericalProjection::StereographicFisheye,
        SphericalProjection::StereographicFisheye => SphericalProjection::Equirect,
    }
}

pub fn spherical_projection_label(mode: SphericalProjection) -> &'static str {
    match mode {
        SphericalProjection::Equirect => "Equirect",
        SphericalProjection::DualFisheye => "Dual fisheye",
        SphericalProjection::Cubemap => "Cubemap",
        SphericalProjection::LittlePlanet => "Little planet",
        SphericalProjection::Eac => "EAC",
        SphericalProjection::Panini => "Panini",
        SphericalProjection::Cylindrical => "Cylindrical",
        SphericalProjection::Mercator => "Mercator",
        SphericalProjection::DualFisheyeTb => "Dual fisheye TB",
        SphericalProjection::Octahedral => "Octahedral",
        SphericalProjection::Equisolid => "Equisolid",
        SphericalProjection::Orthographic => "Orthographic",
        SphericalProjection::Gnomonic => "Gnomonic",
        SphericalProjection::Sinusoidal => "Sinusoidal",
        SphericalProjection::Miller => "Miller",
        SphericalProjection::AzimuthalEquidistant => "Azimuthal EQ",
        SphericalProjection::Mollweide => "Mollweide",
        SphericalProjection::Hammer => "Hammer",
        SphericalProjection::StereographicFisheye => "Stereo fisheye",
    }
}

pub fn format_spherical_projection_osd(mode: SphericalProjection) -> String {
    format!("360° {}", spherical_projection_label(mode))
}

/// Remap dual-fisheye (side-by-side) into a temporary equirect for viewpoint projection.
pub fn dual_fisheye_to_equirect(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    if src_w < 2 || src_h == 0 || src.is_empty() {
        return out;
    }
    let eye_w = src_w / 2;
    for y in 0..out_h {
        let lat = (0.5 - (y as f32 + 0.5) / out_h as f32) * std::f32::consts::PI;
        for x in 0..out_w {
            let lon = ((x as f32 + 0.5) / out_w as f32 * 2.0 - 1.0) * std::f32::consts::PI;
            let use_right = lon >= 0.0;
            let eye_lon = if use_right {
                lon
            } else {
                lon + std::f32::consts::PI
            };
            let r = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0) * 0.5;
            let angle = eye_lon;
            let fx = 0.5 + r * angle.cos();
            let fy = 0.5 + r * angle.sin();
            let ox = ((fx.clamp(0.0, 1.0) * (eye_w.saturating_sub(1) as f32)).round() as u32)
                + if use_right { eye_w } else { 0 };
            let oy = (fy.clamp(0.0, 1.0) * (src_h.saturating_sub(1) as f32)).round() as u32;
            let idx = (oy as usize) * (src_w as usize) + (ox.min(src_w - 1) as usize);
            out[(y * out_w + x) as usize] = src.get(idx).copied().unwrap_or(0);
        }
    }
    out
}

/// Sample a horizontal cubemap strip (6 square faces: +X -X +Y -Y +Z -Z).
pub fn sample_cubemap_pixel(
    pixels: &[u32],
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
    dz: f32,
) -> u32 {
    let ax = dx.abs();
    let ay = dy.abs();
    let az = dz.abs();
    let (face, u, v) = if ax >= ay && ax >= az {
        if dx > 0.0 {
            (0u32, -dz / ax, -dy / ax)
        } else {
            (1, dz / ax, -dy / ax)
        }
    } else if ay >= ax && ay >= az {
        if dy > 0.0 {
            (2, dx / ay, dz / ay)
        } else {
            (3, dx / ay, -dz / ay)
        }
    } else if dz > 0.0 {
        (4, dx / az, -dy / az)
    } else {
        (5, -dx / az, -dy / az)
    };
    let face_w = (width / 6).max(1);
    let face_h = height.max(1);
    let uu = ((u * 0.5 + 0.5).clamp(0.0, 1.0) * (face_w.saturating_sub(1) as f32)).round() as u32;
    let vv = ((v * 0.5 + 0.5).clamp(0.0, 1.0) * (face_h.saturating_sub(1) as f32)).round() as u32;
    let x = face * face_w + uu.min(face_w - 1);
    let y = vv.min(face_h - 1);
    let idx = (y as usize) * (width as usize) + (x.min(width - 1) as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Sample EAC (equi-angular cubemap) strip using atan face UVs.
pub fn sample_eac_pixel(pixels: &[u32], width: u32, height: u32, dx: f32, dy: f32, dz: f32) -> u32 {
    let (face, u, v) = eac_face_uv_from_dir(dx, dy, dz);
    let face_w = (width / 6).max(1);
    let face_h = height.max(1);
    let uu = (u.clamp(0.0, 1.0) * (face_w.saturating_sub(1) as f32)).round() as u32;
    let vv = (v.clamp(0.0, 1.0) * (face_h.saturating_sub(1) as f32)).round() as u32;
    let x = u32::from(face) * face_w + uu.min(face_w - 1);
    let y = vv.min(face_h - 1);
    let idx = (y as usize) * (width as usize) + (x.min(width - 1) as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Stereographic "little planet" remap from equirectangular source.
pub fn project_little_planet(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    let cx = out_w as f32 * 0.5;
    let cy = out_h as f32 * 0.5;
    let radius = cx.min(cy).max(1.0);
    for y in 0..out_h {
        for x in 0..out_w {
            let dx = (x as f32 + 0.5) - cx;
            let dy = (y as f32 + 0.5) - cy;
            let r = (dx * dx + dy * dy).sqrt() / radius;
            let lon = dx.atan2(-dy) + yaw;
            let lat = (1.0 - r).clamp(-1.0, 1.0) * std::f32::consts::FRAC_PI_2;
            out[(y * out_w + x) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Project a spherical view using the selected source projection.
pub fn project_spherical_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    projection: SphericalProjection,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    match projection {
        SphericalProjection::Equirect => project_equirect_view_ex(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::DualFisheye => {
            let equirect = dual_fisheye_to_equirect(src_w, src_h, src, src_w.max(2), src_h.max(1));
            project_equirect_view_ex(
                src_w.max(2),
                src_h.max(1),
                &equirect,
                out_w,
                out_h,
                yaw_deg_milli,
                pitch_deg_milli,
                roll_deg_milli,
                fov_deg_milli,
            )
        }
        SphericalProjection::Cubemap => {
            // Reuse equirect camera rays, sampling cubemap faces instead.
            let out_w = out_w.max(1);
            let out_h = out_h.max(1);
            let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
            let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
            let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
            let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
            let aspect = out_w as f32 / out_h as f32;
            let tan_half = (fov * 0.5).tan();
            let (sin_y, cos_y) = yaw.sin_cos();
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_r, cos_r) = roll.sin_cos();
            let mut out = vec![0u32; out_w as usize * out_h as usize];
            for oy in 0..out_h {
                let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
                for ox in 0..out_w {
                    let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
                    let nx = nx0 * cos_r - ny0 * sin_r;
                    let ny = nx0 * sin_r + ny0 * cos_r;
                    let x1 = nx;
                    let y1 = ny * cos_p - 1.0 * sin_p;
                    let z1 = ny * sin_p + 1.0 * cos_p;
                    let x2 = x1 * cos_y + z1 * sin_y;
                    let y2 = y1;
                    let z2 = -x1 * sin_y + z1 * cos_y;
                    let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
                    out[(oy * out_w + ox) as usize] =
                        sample_cubemap_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
                }
            }
            out
        }
        SphericalProjection::Eac => {
            let out_w = out_w.max(1);
            let out_h = out_h.max(1);
            let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
            let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
            let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
            let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
            let aspect = out_w as f32 / out_h as f32;
            let tan_half = (fov * 0.5).tan();
            let (sin_y, cos_y) = yaw.sin_cos();
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_r, cos_r) = roll.sin_cos();
            let mut out = vec![0u32; out_w as usize * out_h as usize];
            for oy in 0..out_h {
                let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
                for ox in 0..out_w {
                    let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
                    let nx = nx0 * cos_r - ny0 * sin_r;
                    let ny = nx0 * sin_r + ny0 * cos_r;
                    let x1 = nx;
                    let y1 = ny * cos_p - 1.0 * sin_p;
                    let z1 = ny * sin_p + 1.0 * cos_p;
                    let x2 = x1 * cos_y + z1 * sin_y;
                    let y2 = y1;
                    let z2 = -x1 * sin_y + z1 * cos_y;
                    let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
                    out[(oy * out_w + ox) as usize] =
                        sample_eac_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
                }
            }
            out
        }
        SphericalProjection::LittlePlanet => {
            project_little_planet(src_w, src_h, src, out_w, out_h, yaw_deg_milli)
        }
        SphericalProjection::Panini => project_panini_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
            1_000,
        ),
        SphericalProjection::Cylindrical => project_cylindrical_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Mercator => project_mercator_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::DualFisheyeTb => {
            let equirect =
                dual_fisheye_tb_to_equirect(src_w, src_h, src, src_w.max(2), src_h.max(1));
            project_equirect_view_ex(
                src_w.max(2),
                src_h.max(1),
                &equirect,
                out_w,
                out_h,
                yaw_deg_milli,
                pitch_deg_milli,
                roll_deg_milli,
                fov_deg_milli,
            )
        }
        SphericalProjection::Octahedral => project_octahedral_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Equisolid => project_equisolid_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Orthographic => project_orthographic_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
        ),
        SphericalProjection::Gnomonic => project_gnomonic_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Sinusoidal => project_sinusoidal_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Miller => project_miller_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::AzimuthalEquidistant => project_azimuthal_equidistant_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
        ),
        SphericalProjection::Mollweide => project_mollweide_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::Hammer => project_hammer_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            fov_deg_milli,
        ),
        SphericalProjection::StereographicFisheye => project_stereographic_fisheye_view(
            src_w,
            src_h,
            src,
            out_w,
            out_h,
            yaw_deg_milli,
            pitch_deg_milli,
            roll_deg_milli,
            fov_deg_milli,
        ),
    }
}

/// Sample octahedral environment map.
pub fn sample_octahedral_pixel(
    pixels: &[u32],
    width: u32,
    height: u32,
    dx: f32,
    dy: f32,
    dz: f32,
) -> u32 {
    let len = (dx.abs() + dy.abs() + dz.abs()).max(1e-6);
    let mut x = dx / len;
    let mut y = dy / len;
    let z = dz / len;
    if z < 0.0 {
        let ox = x;
        x = (1.0 - y.abs()) * if ox >= 0.0 { 1.0 } else { -1.0 };
        y = (1.0 - ox.abs()) * if y >= 0.0 { 1.0 } else { -1.0 };
    }
    let u = x * 0.5 + 0.5;
    let v = y * 0.5 + 0.5;
    let px = ((u.clamp(0.0, 1.0) * (width.saturating_sub(1) as f32)).round() as u32)
        .min(width.saturating_sub(1));
    let py = ((v.clamp(0.0, 1.0) * (height.saturating_sub(1) as f32)).round() as u32)
        .min(height.saturating_sub(1));
    let idx = (py as usize) * (width as usize) + (px as usize);
    pixels.get(idx).copied().unwrap_or(0)
}

pub fn project_octahedral_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let x1 = nx;
            let y1 = ny * cos_p - sin_p;
            let z1 = ny * sin_p + cos_p;
            let x2 = x1 * cos_y + z1 * sin_y;
            let y2 = y1;
            let z2 = -x1 * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
            out[(oy * out_w + ox) as usize] =
                sample_octahedral_pixel(src, src_w, src_h, x2 / len, y2 / len, z2 / len);
        }
    }
    out
}

/// Equisolid fisheye viewport sampling equirect.
pub fn project_equisolid_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let r_max = 2.0 * (fov * 0.5).sin().max(1e-6);
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let r = (nx * nx + ny * ny).sqrt();
            if r > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let theta = 2.0 * ((r * r_max * 0.5).clamp(0.0, 1.0)).asin();
            let phi = ny.atan2(nx);
            let x_cam = theta.sin() * phi.cos();
            let y_cam = theta.sin() * phi.sin();
            let z_cam = theta.cos();
            // Apply yaw/pitch to camera forward
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_y, cos_y) = yaw.sin_cos();
            let y1 = y_cam * cos_p - z_cam * sin_p;
            let z1 = y_cam * sin_p + z_cam * cos_p;
            let x2 = x_cam * cos_y + z1 * sin_y;
            let z2 = -x_cam * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y1 * y1 + z2 * z2).sqrt().max(1e-6);
            let lon = (z2 / len).atan2(x2 / len);
            let lat = (y1 / len).asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Panini wide-FOV projection from equirect (d_milli = 1000 → classic d=1).
pub fn project_panini_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
    d_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let d = (d_milli.clamp(100, 5_000) as f32) / 1_000.0;
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let lon_off = (nx / (d + 1.0)).atan();
            let lat_off = (ny * (d + lon_off.cos()).max(0.05) / (d + 1.0)).atan();
            let lon = yaw + lon_off;
            let lat = (pitch + lat_off).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Cylindrical panorama projection from equirect.
pub fn project_cylindrical_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let lon = yaw + nx; // linear azimuth
            let lat = (pitch + ny.atan()).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Stereo packing inside a 360° sphere (YouTube VR / GoPro VR / Spatial Media).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SphericalStereoLayout {
    #[default]
    Mono,
    TopBottom,
    SideBySide,
}

pub fn cycle_spherical_stereo(layout: SphericalStereoLayout) -> SphericalStereoLayout {
    match layout {
        SphericalStereoLayout::Mono => SphericalStereoLayout::TopBottom,
        SphericalStereoLayout::TopBottom => SphericalStereoLayout::SideBySide,
        SphericalStereoLayout::SideBySide => SphericalStereoLayout::Mono,
    }
}

/// UV rect (normalized 0..=1000) for left/right eye within a stereo 360 frame.
pub fn spherical_stereo_uv_rect(
    layout: SphericalStereoLayout,
    right_eye: bool,
) -> (i32, i32, i32, i32) {
    match layout {
        SphericalStereoLayout::Mono => (0, 0, 1_000, 1_000),
        SphericalStereoLayout::TopBottom => {
            if right_eye {
                (0, 500, 1_000, 1_000)
            } else {
                (0, 0, 1_000, 500)
            }
        }
        SphericalStereoLayout::SideBySide => {
            if right_eye {
                (500, 0, 1_000, 1_000)
            } else {
                (0, 0, 500, 1_000)
            }
        }
    }
}

/// Parse spherical stereo layout CLI token.
pub fn parse_spherical_stereo(spec: &str) -> Result<SphericalStereoLayout> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "mono" | "off" => Ok(SphericalStereoLayout::Mono),
        "tb" | "top-bottom" | "over-under" => Ok(SphericalStereoLayout::TopBottom),
        "sbs" | "side-by-side" => Ok(SphericalStereoLayout::SideBySide),
        other => Err(format!(
            "spherical stereo expected mono|tb|sbs got `{other}`"
        )),
    }
}

/// Equi-Angular Cubemap face index + local UV from direction (YouTube EAC).
pub fn eac_face_uv_from_dir(x: f32, y: f32, z: f32) -> (u8, f32, f32) {
    let ax = x.abs();
    let ay = y.abs();
    let az = z.abs();
    let (face, uc, vc, maxc) = if ax >= ay && ax >= az {
        if x > 0.0 {
            (0u8, -z, y, ax)
        } else {
            (1, z, y, ax)
        }
    } else if ay >= ax && ay >= az {
        if y > 0.0 {
            (2, x, -z, ay)
        } else {
            (3, x, z, ay)
        }
    } else if z > 0.0 {
        (4, x, y, az)
    } else {
        (5, -x, y, az)
    };
    let maxc = maxc.max(1e-6);
    // Equi-angular: atan mapping vs perspective
    let u = (uc.atan2(maxc) / std::f32::consts::FRAC_PI_4 + 1.0) * 0.5;
    let v = (vc.atan2(maxc) / std::f32::consts::FRAC_PI_4 + 1.0) * 0.5;
    (face, u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// Mercator projection from equirect (cartographic / planet viewers).
pub fn project_mercator_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + nx * half * aspect;
            let lat = (pitch + (ny * half).sinh().atan()).clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Dual fisheye top-bottom → equirect (Insta360 / GoPro TB layout).
pub fn dual_fisheye_tb_to_equirect(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    if src_w == 0 || src_h < 2 || src.is_empty() {
        return out;
    }
    let eye_h = src_h / 2;
    for y in 0..out_h {
        let lat = (0.5 - (y as f32 + 0.5) / out_h as f32) * std::f32::consts::PI;
        for x in 0..out_w {
            let lon = ((x as f32 + 0.5) / out_w as f32 * 2.0 - 1.0) * std::f32::consts::PI;
            let use_bottom = lon >= 0.0;
            let eye_lon = if use_bottom {
                lon
            } else {
                lon + std::f32::consts::PI
            };
            let r = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0) * 0.5;
            let fx = 0.5 + r * eye_lon.cos();
            let fy = 0.5 + r * eye_lon.sin();
            let ox = (fx.clamp(0.0, 1.0) * (src_w.saturating_sub(1) as f32)).round() as u32;
            let oy = ((fy.clamp(0.0, 1.0) * (eye_h.saturating_sub(1) as f32)).round() as u32)
                + if use_bottom { eye_h } else { 0 };
            let idx =
                (oy.min(src_h - 1) as usize) * (src_w as usize) + (ox.min(src_w - 1) as usize);
            out[(y * out_w + x) as usize] = src.get(idx).copied().unwrap_or(0);
        }
    }
    out
}

/// Orthographic spherical projection (planetarium / scientific viewers).
pub fn project_orthographic_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let r2 = nx * nx + ny * ny;
            if r2 > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let nz = (1.0 - r2).sqrt();
            let y1 = ny * cos_p - nz * sin_p;
            let z1 = ny * sin_p + nz * cos_p;
            let x2 = nx * cos_y + z1 * sin_y;
            let z2 = -nx * sin_y + z1 * cos_y;
            let lon = z2.atan2(x2);
            let lat = y1.asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Gnomonic (gnomonic/rectilinear) projection from equirect.
pub fn project_gnomonic_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    // Same ray math as equirect viewport — gnomonic is the rectilinear camera model.
    project_equirect_view_ex(
        src_w,
        src_h,
        src,
        out_w,
        out_h,
        yaw_deg_milli,
        pitch_deg_milli,
        roll_deg_milli,
        fov_deg_milli,
    )
}

/// Sinusoidal equal-area map projection from equirect.
pub fn project_sinusoidal_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        let lat = (pitch + ny * half).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        let cos_lat = lat.cos().max(1e-3);
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + (nx * half * aspect) / cos_lat;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Miller cylindrical projection from equirect.
pub fn project_miller_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        // Inverse Miller: lat = 1.25*atan(sinh(0.8*y))
        let y = pitch + ny * half;
        let lat = (1.25 * (0.8 * y).sinh().atan()).clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + nx * half * aspect;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Azimuthal equidistant projection (radar / planetarium style).
pub fn project_azimuthal_equidistant_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let rho = (nx * nx + ny * ny).sqrt();
            if rho > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let c = rho * std::f32::consts::PI; // angular distance
            let phi = ny.atan2(nx);
            let lat = (c.cos() * pitch.sin() + c.sin() * pitch.cos() * phi.cos())
                .asin()
                .clamp(
                    -std::f32::consts::FRAC_PI_2 + 0.01,
                    std::f32::consts::FRAC_PI_2 - 0.01,
                );
            let lon =
                yaw + (phi.sin() * c.sin() * pitch.cos()).atan2(c.cos() - pitch.sin() * lat.sin());
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Mollweide equal-area projection (approximate inverse).
pub fn project_mollweide_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        let y = (pitch + ny * half).clamp(-1.0, 1.0);
        let mut theta = y;
        for _ in 0..4 {
            let f = 2.0 * theta + theta.sin() - std::f32::consts::PI * y;
            let df = 2.0 + theta.cos();
            theta -= f / df.max(1e-3);
        }
        let lat = theta.asin().clamp(
            -std::f32::consts::FRAC_PI_2 + 0.01,
            std::f32::consts::FRAC_PI_2 - 0.01,
        );
        let cos_t = theta.cos().max(1e-3);
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let lon = yaw + (nx * half * aspect * std::f32::consts::SQRT_2) / cos_t;
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Hammer equal-area azimuthal projection.
pub fn project_hammer_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let half = fov * 0.5;
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx = 2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0;
            let x = nx * half * aspect;
            let y = pitch + ny * half;
            let z2 = 1.0 - (x * x) / 8.0 - (y * y) / 2.0;
            if z2 <= 0.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let z = z2.sqrt();
            let lon = yaw + 2.0 * (z * x / (2.0 * (2.0 * z2 - 1.0).max(1e-3))).atan();
            let lat = (y * z).asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Stereographic fisheye: r = 2*tan(theta/2).
pub fn project_stereographic_fisheye_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let r_max = 2.0 * (fov * 0.5).tan().max(1e-6);
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = 1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            let r = (nx * nx + ny * ny).sqrt();
            if r > 1.0 {
                out[(oy * out_w + ox) as usize] = 0;
                continue;
            }
            let theta = 2.0 * (r * r_max * 0.5).atan();
            let phi = ny.atan2(nx);
            let x_cam = theta.sin() * phi.cos();
            let y_cam = theta.sin() * phi.sin();
            let z_cam = theta.cos();
            let (sin_p, cos_p) = pitch.sin_cos();
            let (sin_y, cos_y) = yaw.sin_cos();
            let y1 = y_cam * cos_p - z_cam * sin_p;
            let z1 = y_cam * sin_p + z_cam * cos_p;
            let x2 = x_cam * cos_y + z1 * sin_y;
            let z2 = -x_cam * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y1 * y1 + z2 * z2).sqrt().max(1e-6);
            let lon = (z2 / len).atan2(x2 / len);
            let lat = (y1 / len).asin().clamp(
                -std::f32::consts::FRAC_PI_2 + 0.01,
                std::f32::consts::FRAC_PI_2 - 0.01,
            );
            out[(oy * out_w + ox) as usize] = sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

pub fn spherical_projections_differ(
    a: SphericalProjection,
    b: SphericalProjection,
    src: &[u32],
    src_w: u32,
    src_h: u32,
) -> bool {
    if a == b || src.is_empty() || src_w == 0 || src_h == 0 {
        return false;
    }
    let va = project_spherical_view(src_w, src_h, src, 8, 8, a, 30_000, 10_000, 0, 90_000);
    let vb = project_spherical_view(src_w, src_h, src, 8, 8, b, 30_000, 10_000, 0, 90_000);
    va != vb
}

/// VLC-compatible 360° FOV limits (degrees ×1000).
pub const FOV_MIN_MILLI: i32 = 20_000;
pub const FOV_MAX_MILLI: i32 = 150_000;
pub const FOV_DEFAULT_MILLI: i32 = 80_000;
pub const PITCH_MIN_MILLI: i32 = -89_000;
pub const PITCH_MAX_MILLI: i32 = 89_000;
pub const ROLL_STEP_MILLI: i32 = 5_000;

pub fn clamp_yaw_milli(value: i32) -> i32 {
    let mut yaw = value % 360_000;
    if yaw < 0 {
        yaw += 360_000;
    }
    yaw
}

pub fn clamp_pitch_milli(value: i32) -> i32 {
    value.clamp(PITCH_MIN_MILLI, PITCH_MAX_MILLI)
}

pub fn clamp_fov_milli(value: i32) -> i32 {
    value.clamp(FOV_MIN_MILLI, FOV_MAX_MILLI)
}

pub fn clamp_roll_milli(value: i32) -> i32 {
    let mut roll = value % 360_000;
    if roll < 0 {
        roll += 360_000;
    }
    roll
}

/// Sample equirectangular source at normalized lon/lat (radians).
pub fn sample_equirect_pixel(pixels: &[u32], width: u32, height: u32, lon: f32, lat: f32) -> u32 {
    let w = width.max(1) as f32;
    let h = height.max(1) as f32;
    let u = ((lon / std::f32::consts::PI + 1.0) * 0.5).rem_euclid(1.0);
    let v = (0.5 - lat / std::f32::consts::PI).clamp(0.0, 1.0);
    let x = ((u * w).floor() as usize).min(width.saturating_sub(1) as usize);
    let y = ((v * h).floor() as usize).min(height.saturating_sub(1) as usize);
    let idx = y.saturating_mul(width as usize).saturating_add(x);
    pixels.get(idx).copied().unwrap_or(0)
}

/// Project a rectilinear viewport from an equirectangular 360° frame (VLC-style).
pub fn project_equirect_view(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    project_equirect_view_ex(
        src_w,
        src_h,
        src,
        out_w,
        out_h,
        yaw_deg_milli,
        pitch_deg_milli,
        0,
        fov_deg_milli,
    )
}

/// Like [`project_equirect_view`] with viewpoint roll (degrees ×1000).
pub fn project_equirect_view_ex(
    src_w: u32,
    src_h: u32,
    src: &[u32],
    out_w: u32,
    out_h: u32,
    yaw_deg_milli: i32,
    pitch_deg_milli: i32,
    roll_deg_milli: i32,
    fov_deg_milli: i32,
) -> Vec<u32> {
    let out_w = out_w.max(1);
    let out_h = out_h.max(1);
    let yaw = (clamp_yaw_milli(yaw_deg_milli) as f32 / 1_000.0).to_radians();
    let pitch = (clamp_pitch_milli(pitch_deg_milli) as f32 / 1_000.0).to_radians();
    let roll = (clamp_roll_milli(roll_deg_milli) as f32 / 1_000.0).to_radians();
    let fov = (clamp_fov_milli(fov_deg_milli) as f32 / 1_000.0).to_radians();
    let aspect = out_w as f32 / out_h as f32;
    let tan_half = (fov * 0.5).tan();
    let (sin_y, cos_y) = yaw.sin_cos();
    let (sin_p, cos_p) = pitch.sin_cos();
    let (sin_r, cos_r) = roll.sin_cos();
    let mut out = vec![0u32; out_w as usize * out_h as usize];
    for oy in 0..out_h {
        let ny0 = (1.0 - 2.0 * (oy as f32 + 0.5) / out_h as f32) * tan_half;
        for ox in 0..out_w {
            let nx0 = (2.0 * (ox as f32 + 0.5) / out_w as f32 - 1.0) * tan_half * aspect;
            let nx = nx0 * cos_r - ny0 * sin_r;
            let ny = nx0 * sin_r + ny0 * cos_r;
            // Camera looks +Z; rotate pitch then yaw.
            let x1 = nx;
            let y1 = ny * cos_p - 1.0 * sin_p;
            let z1 = ny * sin_p + 1.0 * cos_p;
            let x2 = x1 * cos_y + z1 * sin_y;
            let y2 = y1;
            let z2 = -x1 * sin_y + z1 * cos_y;
            let len = (x2 * x2 + y2 * y2 + z2 * z2).sqrt().max(1e-6);
            let dx = x2 / len;
            let dy = y2 / len;
            let dz = z2 / len;
            let lon = dx.atan2(dz);
            let lat = dy.clamp(-1.0, 1.0).asin();
            out[oy as usize * out_w as usize + ox as usize] =
                sample_equirect_pixel(src, src_w, src_h, lon, lat);
        }
    }
    out
}

/// Parse `--spherical-projection` values.
pub fn parse_spherical_projection(spec: &str) -> Result<SphericalProjection> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "equirect" | "equirectangular" | "360" => Ok(SphericalProjection::Equirect),
        "dual-fisheye" | "fisheye" | "dfisheye" => Ok(SphericalProjection::DualFisheye),
        "cubemap" | "cube" => Ok(SphericalProjection::Cubemap),
        "little-planet" | "planet" | "stereographic" => Ok(SphericalProjection::LittlePlanet),
        "eac" | "equi-angular" | "equiangular" => Ok(SphericalProjection::Eac),
        "panini" => Ok(SphericalProjection::Panini),
        "cylindrical" | "cylinder" => Ok(SphericalProjection::Cylindrical),
        "mercator" => Ok(SphericalProjection::Mercator),
        "dual-fisheye-tb" | "dfisheye-tb" | "fisheye-tb" => Ok(SphericalProjection::DualFisheyeTb),
        "octahedral" | "octa" => Ok(SphericalProjection::Octahedral),
        "equisolid" => Ok(SphericalProjection::Equisolid),
        "orthographic" | "ortho" => Ok(SphericalProjection::Orthographic),
        "gnomonic" | "rectilinear" => Ok(SphericalProjection::Gnomonic),
        "sinusoidal" | "sanson" => Ok(SphericalProjection::Sinusoidal),
        "miller" => Ok(SphericalProjection::Miller),
        "azimuthal" | "azimuthal-equidistant" | "aeqd" => {
            Ok(SphericalProjection::AzimuthalEquidistant)
        }
        "mollweide" => Ok(SphericalProjection::Mollweide),
        "hammer" | "hammer-aitoff" => Ok(SphericalProjection::Hammer),
        "stereographic-fisheye" | "stereo-fisheye" | "sg-fisheye" => {
            Ok(SphericalProjection::StereographicFisheye)
        }
        other => Err(format!(
            "unknown spherical projection `{other}` (equirect|dual-fisheye|cubemap|little-planet|eac|panini|cylindrical|mercator|dual-fisheye-tb|octahedral|equisolid|orthographic|gnomonic|sinusoidal|miller|azimuthal|mollweide|hammer|stereographic-fisheye)"
        )
        .into()),
    }
}

/// Parse `--play-stereo3d` values: `off|sbsl|abl|mono-left|mono-right`.
pub fn parse_play_stereo3d(spec: &str) -> Result<PlayStereo3D> {
    match spec.trim().to_ascii_lowercase().as_str() {
        "off" | "none" | "0" => Ok(PlayStereo3D::Off),
        "sbsl" | "sbs" | "anaglyph" => Ok(PlayStereo3D::SbslAnaglyph),
        "abl" | "tab" => Ok(PlayStereo3D::AblAnaglyph),
        "mono-left" | "ml" | "left" => Ok(PlayStereo3D::MonoLeft),
        "mono-right" | "mr" | "right" => Ok(PlayStereo3D::MonoRight),
        other => Err(format!(
            "unknown play stereo3d `{other}` (off|sbsl|abl|mono-left|mono-right)"
        )
        .into()),
    }
}

/// Parse degrees for viewpoint CLI (`45`, `45.5`) into milli-degrees.
pub fn parse_degrees_milli(spec: &str) -> Result<i32> {
    let value: f32 = spec
        .parse()
        .map_err(|_| format!("invalid degrees `{spec}`"))?;
    if !value.is_finite() {
        return Err(format!("invalid degrees `{spec}`").into());
    }
    Ok((value * 1_000.0).round() as i32)
}
