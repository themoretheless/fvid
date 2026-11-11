struct Params {
    y_offset: f32,
    y_gain: f32,
    c_offset: f32,
    c_gain: f32,
    kr: f32,
    kb: f32,
    srgb: f32,
    sample_scale: f32,
    // The part of the planes a crop keeps, as texture coordinates: the corner
    // it starts at, then the corner it ends at. The whole picture is (0,0)-(1,1).
    window: vec4<f32>,
    // VLC's picture settings as the five numbers its adjust filter computes:
    // the luma becomes gamma⁻¹(clip(lum + contrast·y)) on stored samples and
    // the chroma pair rotates by the hue and scales by the saturation around
    // grey. The identity bundle — 1, 0, 1, 1, 0 — leaves both as they stand.
    adjust_a: vec4<f32>, // contrast, lum, inverse gamma, cos·saturation
    adjust_b: vec4<f32>, // sin·saturation, then whether the bundle moves at all
    // The grade the CPU baked, as one lookup into the table it itself reads.
    // `grid.x` is a grid's edge length, `grid.y` how to read between its nodes —
    // 0 for nearest, 1 for trilinear, 2 for tetrahedral — and `grid.z` says
    // which table: 0 is the 3D grid of nodes bound at 5, 1 is the byte tables
    // bound at 6. `grid.w` turns the lookup off, so a frame with nothing to
    // grade pays nothing for either binding.
    grid: vec4<f32>,
    look: vec4<f32>,
    look_min: vec4<f32>,
    look_max: vec4<f32>,
    encoder_size: vec4<f32>,
    // Stereo3D & 360° spherical viewport parameters:
    // `view_a`: stereo_mode, spherical_mode, spherical_stereo, aspect.
    // `view_b`: yaw_rad, pitch_rad, roll_rad, fov_rad.
    view_a: vec4<f32>,
    view_b: vec4<f32>,
};
@group(0) @binding(0) var plane_y: texture_2d<f32>;
@group(0) @binding(1) var plane_cb: texture_2d<f32>;
@group(0) @binding(2) var plane_cr: texture_2d<f32>;
@group(0) @binding(3) var plane_sampler: sampler;
@group(0) @binding(4) var<uniform> params: Params;
// The grade's nodes, one texel per node: the table lists red fastest, so node
// (r, g, b) is texel (r, g, b) and the walk below is `Lut3d::sample`'s.
@group(0) @binding(5) var grade_grid: texture_3d<f32>;
// A plan that maps each channel on its own is three byte tables, not a grid, and
// that is what the CPU paints from. Texel `code` of this one row holds the three
// tables' answers for a channel coded `code`, each in its own component.
@group(0) @binding(7) var look_grid: texture_3d<f32>;
@group(0) @binding(6) var grade_tables: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) index: u32) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0),
        vec2(-1.0, 1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
    );
    let p = corners[index];
    var out: VertexOut;
    out.position = vec4(p, 0.0, 1.0);
    // The quad spans the rect it is drawn into, so the crop is a span of
    // texture coordinates rather than a change of where the quad lands.
    let t = vec2((p.x + 1.0) * 0.5, (1.0 - p.y) * 0.5);
    let uv = mix(params.window.xy, params.window.zw, t);
    out.uv = uv;
    return out;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    let low = c / 12.92;
    let high = pow((c + 0.055) / 1.055, vec3(2.4));
    return select(high, low, c <= vec3(0.04045));
}

fn load_rgba_texel(p: vec2<i32>) -> vec4<f32> {
    let raw = textureLoad(plane_y, p, 0);
    return vec4(raw.rgb * raw.a, raw.a);
}

// Interpolate normalized byte texels in float32, preserving precision when
// an RGB8 source is scaled into a 10-bit encoder surface.
fn sample_rgba(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(plane_y, 0));
    var at = clamp(uv * vec2<f32>(size) - vec2(0.5), vec2(0.0), vec2<f32>(size - vec2(1)));
    if params.encoder_size.x > 0.0 {
        let edge = vec2<f32>(size - vec2(1));
        let low = clamp(params.window.xy * vec2<f32>(size), vec2(0.0), edge);
        let high = max(low, clamp(params.window.zw * vec2<f32>(size) - vec2(1.0), vec2(0.0), edge));
        at = clamp(at, low, high);
    }
    let lo = vec2<i32>(floor(at));
    let hi = min(lo + vec2(1), size - vec2(1));
    let fraction = fract(at);
    let a = load_rgba_texel(lo);
    let b = load_rgba_texel(vec2(hi.x, lo.y));
    let c = load_rgba_texel(vec2(lo.x, hi.y));
    let d = load_rgba_texel(hi);
    return mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
}

fn sample_rgb(uv: vec2<f32>) -> vec3<f32> {
    let rgba = sample_rgba(uv);
    return rgba.rgb / max(rgba.a, 0.0000001);
}

fn grid_node(grid: texture_3d<f32>, at: vec3<i32>) -> vec3<f32> {
    return textureLoad(grid, at, 0).rgb;
}

/// The component axis `i` names. A vector cannot be indexed by a runtime value,
/// so the tetrahedral walk below asks rather than subscript.
fn axis_of(d: vec3<f32>, i: i32) -> f32 {
    if i == 0 {
        return d.x;
    }
    if i == 1 {
        return d.y;
    }
    return d.z;
}

/// The unit step along axis `i`: 0 is red, 1 green, 2 blue, the same order the
/// CPU's table indexes its nodes in.
fn along_axis(i: i32) -> vec3<i32> {
    if i == 0 {
        return vec3(1, 0, 0);
    }
    if i == 1 {
        return vec3(0, 1, 0);
    }
    return vec3(0, 0, 1);
}

/// The axis of the largest offset, the lowest axis keeping a tie — the sort's
/// first pick. Walking up with a strict `>` leaves the earliest maximum.
fn tallest_axis(d: vec3<f32>) -> i32 {
    var best = i32(0);
    for (var i = i32(1); i < i32(3); i = i + 1) {
        if axis_of(d, i) > axis_of(d, best) {
            best = i;
        }
    }
    return best;
}

/// The axis of the smallest offset, the highest axis keeping a tie — the same
/// sort's last pick. Walking down with a strict `<` leaves the latest minimum.
fn shortest_axis(d: vec3<f32>) -> i32 {
    var best = i32(2);
    for (var i = i32(1); i >= i32(0); i = i - 1) {
        if axis_of(d, i) < axis_of(d, best) {
            best = i;
        }
    }
    return best;
}

/// Read the three byte tables, which is what `Grade::paint` does: the code a
/// channel carries indexes its own table, and the tables were baked along the
/// grey axis — valid only because `Grade::new` measured that the plan keeps its
/// channels apart before it let this route exist.
fn table_read(rgb: vec3<f32>) -> vec3<f32> {
    let at = rgb * vec3(255.0);
    let r = textureLoad(grade_tables, vec2<i32>(i32(round(at.x)), 0), 0).r;
    let g = textureLoad(grade_tables, vec2<i32>(i32(round(at.y)), 0), 0).g;
    let b = textureLoad(grade_tables, vec2<i32>(i32(round(at.z)), 0), 0).b;
    return vec3(r, g, b);
}

/// Read the baked grid, the way `Lut3d::sample` reads the same table: the input
/// is a display code in [0, 1], the grid's domain is [0, 1] because the plan
/// baked it, and the three modes are the CPU's three — a snap to the nearest
/// node, a box blend of eight, and a walk of four nodes along the ordered
/// offsets.
fn grade_read(rgb: vec3<f32>) -> vec3<f32> {
    if params.grid.z > 0.5 { return table_read(rgb); }
    let converted = read_grid(rgb, grade_grid, params.grid);
    if params.look.w < 0.5 { return converted; }
    let normalized = clamp((converted - params.look_min.xyz) / max(params.look_max.xyz - params.look_min.xyz, vec3(0.000001)), vec3(0.0), vec3(1.0));
    if params.look.z > 0.5 {
        let position = normalized * (params.look.x - 1.0);
        return vec3(look_channel(position.x, 0), look_channel(position.y, 1), look_channel(position.z, 2));
    }
    return read_grid(normalized, look_grid, params.look);
}
fn look_node(index: i32) -> vec3<f32> {
    let size = vec3<i32>(textureDimensions(look_grid, 0));
    return textureLoad(look_grid, vec3(index % size.x, (index / size.x) % size.y, index / (size.x * size.y)), 0).rgb;
}
fn look_channel(position: f32, channel: i32) -> f32 {
    let lower = i32(floor(position));
    let upper = min(lower + 1, i32(params.look.x) - 1);
    let weight = position - f32(lower);
    return axis_of(look_node(lower), channel) * (1.0 - weight) + axis_of(look_node(upper), channel) * weight;
}
fn read_grid(rgb: vec3<f32>, grid: texture_3d<f32>, options: vec4<f32>) -> vec3<f32> {
    let last = vec3<i32>(i32(options.x) - 1);
    let p = rgb * vec3(f32(last.x));
    let origin = vec3<i32>(floor(p));
    let offsets = p - vec3<f32>(origin);
    if options.y < 0.5 {
        let near = select(vec3<i32>(0), vec3<i32>(1), offsets > vec3(0.5));
        return grid_node(grid, min(origin + near, last));
    }
    if options.y < 1.5 {
        let r1 = min(origin + vec3(1, 0, 0), last);
        let g1 = min(origin + vec3(0, 1, 0), last);
        let b1 = min(origin + vec3(0, 0, 1), last);
        let c000 = grid_node(grid, origin);
        let c100 = grid_node(grid, r1);
        let c010 = grid_node(grid, g1);
        let c110 = grid_node(grid, min(origin + vec3(1, 1, 0), last));
        let c001 = grid_node(grid, b1);
        let c101 = grid_node(grid, min(r1 + vec3(0, 0, 1), last));
        let c011 = grid_node(grid, min(g1 + vec3(0, 0, 1), last));
        let c111 = grid_node(grid, min(r1 + vec3(0, 1, 1), last));
        let x00 = mix(c000, c100, vec3(offsets.x));
        let x10 = mix(c010, c110, vec3(offsets.x));
        let x01 = mix(c001, c101, vec3(offsets.x));
        let x11 = mix(c011, c111, vec3(offsets.x));
        let y0 = mix(x00, x10, vec3(offsets.y));
        let y1 = mix(x01, x11, vec3(offsets.y));
        return mix(y0, y1, vec3(offsets.z));
    }
    // Order the axes by offset and walk the four nodes of that tetrahedron:
    // origin, plus the tallest axis, plus the middle one, plus all three. The
    // middle is the axis neither of the two walks named, because the three axes
    // are 0, 1 and 2 and the pair can never land on one axis twice.
    let a = tallest_axis(offsets);
    let c = shortest_axis(offsets);
    let b = 3 - a - c;
    let e1 = along_axis(a);
    let e2 = e1 + along_axis(b);
    let n0 = grid_node(grid, origin);
    let n1 = grid_node(grid, min(origin + e1, last));
    let n2 = grid_node(grid, min(origin + e2, last));
    let n3 = grid_node(grid, min(origin + vec3(1, 1, 1), last));
    let sa = axis_of(offsets, a);
    let sb = axis_of(offsets, b);
    let sc = axis_of(offsets, c);
    return n0 * (1.0 - sa) + n1 * (sa - sb) + n2 * (sb - sc) + n3 * sc;
}

/// The five numbers of the bundle run over display codes, which is what the
/// CPU draw path does to a picture that has already left the planes behind:
/// BT.601 limited range solved back out of the bytes, the luma line and its
/// gamma curve, the chroma turned around grey, and the same matrix forward
/// again. `adjust_rgb` is the other half of this and both are read by the same
/// test, because a viewer's slider has to move the shown picture the same way
/// whichever route drew it.
fn adjust_codes(rgb: vec3<f32>) -> vec3<f32> {
    // The same landing on a byte the lookup in front of it makes: the CPU route
    // hands its sliders a frame of stored codes, not the grid's float answer.
    let stored = round(rgb * vec3(255.0));
    let solved = (0.299 * stored.x + 0.587 * stored.y + 0.114 * stored.z) / 1.164 + 16.0;
    let cb = (-0.168736 * stored.x - 0.331264 * stored.y + 0.5 * stored.z) * (224.0 / 255.0);
    let cr = (0.5 * stored.x - 0.418688 * stored.y - 0.081312 * stored.z) * (224.0 / 255.0);
    let luma = clamp(params.adjust_a.x * solved + params.adjust_a.y, 0.0, 255.0);
    let turned = pow(luma / 255.0, params.adjust_a.z) * 255.0;
    // Both lines read the pair as it was solved, which is the rotation VLC's
    // filter works out.
    let cb_adj = params.adjust_a.w * cb + params.adjust_b.x * cr;
    let cr_adj = params.adjust_a.w * cr - params.adjust_b.x * cb;
    let y = 1.164 * (turned - 16.0);
    let r = y + 1.596 * cr_adj;
    let g = y - 0.391 * cb_adj - 0.813 * cr_adj;
    let b = y + 2.013 * cb_adj;
    let shown = clamp(round(vec3(r, g, b)), vec3(0.0), vec3(255.0));
    return shown / vec3(255.0);
}

// Plane sampling returns source codes scaled into eight-bit code units.
// Integer reconstruction before interpolation preserves source low bits.
fn stored(sampled: vec4<f32>) -> f32 { return sampled.r; }
fn decoded_texel(sampled: vec4<f32>) -> vec4<f32> {
    if params.adjust_b.z > 1.5 {
        // Recover exact MSB-aligned integer codes before interpolation.
        return round(sampled * (65535.0 / 64.0)) * 0.25;
    }
    return vec4<f32>(dot(round(sampled.rg * 255.0), vec2(1.0, 256.0)) * params.sample_scale, 0.0, 0.0, 0.0);
}

fn source_uv(uv: vec2<f32>) -> vec2<f32> {
    if params.adjust_b.w == 1.0 { return vec2(uv.y, 1.0 - uv.x); }
    if params.adjust_b.w == 2.0 { return vec2(1.0 - uv.x, 1.0 - uv.y); }
    if params.adjust_b.w == 3.0 { return vec2(1.0 - uv.y, uv.x); }
    return uv;
}
fn source_texel(plane: texture_2d<f32>, p: vec2<i32>) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(plane, 0));
    var point = p;
    if params.adjust_b.w == 1.0 { point = vec2(p.y, size.y - 1 - p.x); }
    if params.adjust_b.w == 2.0 { point = size - vec2(1) - p; }
    if params.adjust_b.w == 3.0 { point = vec2(size.x - 1 - p.y, p.x); }
    return decoded_texel(textureLoad(plane, point, 0));
}

// Wide stored samples must interpolate in float32. Hardware filtering of
// split low/high-byte channels can round each component independently and
// amplify that error at byte boundaries. Native P010 uses this same rule.
fn sample_plane(plane: texture_2d<f32>, uv: vec2<f32>) -> vec4<f32> {
    var size = vec2<i32>(textureDimensions(plane, 0));
    if params.adjust_b.w == 1.0 || params.adjust_b.w == 3.0 { size = size.yx; }
    var bounded_uv = uv;
    if params.encoder_size.x > 0.0 {
        let half_pixel = vec2(0.5) / vec2<f32>(size);
        let low = min(params.window.xy + half_pixel, params.window.zw);
        let high = max(low, params.window.zw - half_pixel);
        bounded_uv = clamp(uv, low, high);
    }
    if params.sample_scale == 0.0 && params.adjust_b.z < 1.5 {
        return textureSample(plane, plane_sampler, source_uv(bounded_uv)) * 255.0;
    }
    let position = bounded_uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(position));
    let fraction = fract(position);
    let last = size - vec2<i32>(1);
    let a = source_texel(plane, clamp(base, vec2<i32>(0), last));
    let b = source_texel(plane, clamp(base + vec2<i32>(1, 0), vec2<i32>(0), last));
    let c = source_texel(plane, clamp(base + vec2<i32>(0, 1), vec2<i32>(0), last));
    let d = source_texel(plane, clamp(base + vec2<i32>(1, 1), vec2<i32>(0), last));
    return mix(mix(a, b, fraction.x), mix(c, d, fraction.x), fraction.y);
}

fn safe_atan2(y: f32, x: f32) -> f32 {
    if x == 0.0 && y == 0.0 {
        return 0.0;
    }
    return atan2(y, x);
}

fn equirect_uv(lon: f32, lat: f32) -> vec2<f32> {
    let pi = 3.14159265;
    let raw_u = (lon / pi + 1.0) * 0.5;
    let u = raw_u - floor(raw_u);
    let v = clamp(0.5 - lat / pi, 0.0, 1.0);
    return vec2(u, v);
}

fn camera_ray(uv: vec2<f32>, aspect: f32, yaw: f32, pitch: f32, roll: f32, fov: f32) -> vec3<f32> {
    let tan_half = tan(fov * 0.5);
    let nx0 = (2.0 * uv.x - 1.0) * tan_half * aspect;
    let ny0 = (1.0 - 2.0 * uv.y) * tan_half;
    let cos_r = cos(roll);
    let sin_r = sin(roll);
    let nx = nx0 * cos_r - ny0 * sin_r;
    let ny = nx0 * sin_r + ny0 * cos_r;
    let cos_p = cos(pitch);
    let sin_p = sin(pitch);
    let y1 = ny * cos_p - sin_p;
    let z1 = ny * sin_p + cos_p;
    let cos_y = cos(yaw);
    let sin_y = sin(yaw);
    let x2 = nx * cos_y + z1 * sin_y;
    let z2 = -nx * sin_y + z1 * cos_y;
    let len = max(sqrt(x2 * x2 + y1 * y1 + z2 * z2), 0.000001);
    return vec3(x2 / len, y1 / len, z2 / len);
}

fn upright_luma_dims() -> vec2<f32> {
    var dims = vec2<f32>(max(textureDimensions(plane_y, 0), vec2<u32>(2u)));
    if params.adjust_b.w == 1.0 || params.adjust_b.w == 3.0 {
        dims = dims.yx;
    }
    return dims;
}

fn project_spherical_uv(uv: vec2<f32>) -> vec3<f32> {
    let mode = i32(round(params.view_a.y));
    if mode <= 0 {
        return vec3(uv, 1.0);
    }
    let pi = 3.14159265;
    let half_pi = 1.57079633;
    let aspect = max(params.view_a.w, 0.0001);
    let yaw = params.view_b.x;
    let pitch = params.view_b.y;
    let roll = params.view_b.z;
    let fov = params.view_b.w;
    var out_uv = uv;
    if mode == 1 || mode == 13 {
        let d = camera_ray(uv, aspect, yaw, pitch, roll, fov);
        out_uv = equirect_uv(safe_atan2(d.x, d.z), asin(clamp(d.y, -1.0, 1.0)));
    } else if mode == 2 || mode == 9 {
        let d = camera_ray(uv, aspect, yaw, pitch, roll, fov);
        let lon = safe_atan2(d.x, d.z);
        let lat = asin(clamp(d.y, -1.0, 1.0));
        let second_eye = lon >= 0.0;
        let eye_lon = select(lon + pi, lon, second_eye);
        let r = clamp(0.5 - lat / pi, 0.0, 1.0) * 0.5;
        let fx = clamp(0.5 + r * cos(eye_lon), 0.0, 1.0);
        let fy = clamp(0.5 + r * sin(eye_lon), 0.0, 1.0);
        if mode == 2 {
            out_uv = vec2(fx * 0.5 + select(0.0, 0.5, second_eye), fy);
        } else {
            out_uv = vec2(fx, fy * 0.5 + select(0.0, 0.5, second_eye));
        }
    } else if mode == 3 || mode == 5 {
        let d = camera_ray(uv, aspect, yaw, pitch, roll, fov);
        let ax = abs(d.x);
        let ay = abs(d.y);
        let az = abs(d.z);
        var face = 0.0;
        var uc = 0.0;
        var vc = 0.0;
        var maxc = 1.0;
        if ax >= ay && ax >= az {
            maxc = max(ax, 0.000001);
            if d.x > 0.0 { face = 0.0; uc = -d.z; vc = select(d.y, -d.y, mode == 3); }
            else { face = 1.0; uc = d.z; vc = select(d.y, -d.y, mode == 3); }
        } else if ay >= ax && ay >= az {
            maxc = max(ay, 0.000001);
            if d.y > 0.0 { face = 2.0; uc = d.x; vc = select(-d.z, d.z, mode == 3); }
            else { face = 3.0; uc = d.x; vc = select(d.z, -d.z, mode == 3); }
        } else if d.z > 0.0 {
            maxc = max(az, 0.000001);
            face = 4.0; uc = d.x; vc = select(d.y, -d.y, mode == 3);
        } else {
            maxc = max(az, 0.000001);
            face = 5.0; uc = -d.x; vc = select(d.y, -d.y, mode == 3);
        }
        var fu = 0.5;
        var fv = 0.5;
        if mode == 3 {
            fu = clamp((uc / maxc) * 0.5 + 0.5, 0.0, 1.0);
            fv = clamp((vc / maxc) * 0.5 + 0.5, 0.0, 1.0);
        } else {
            let qpi = 0.78539816;
            fu = clamp((safe_atan2(uc, maxc) / qpi + 1.0) * 0.5, 0.0, 1.0);
            fv = clamp((safe_atan2(vc, maxc) / qpi + 1.0) * 0.5, 0.0, 1.0);
        }
        out_uv = vec2((face + fu) / 6.0, fv);
    } else if mode == 4 {
        let dx = (uv.x - 0.5) * select(aspect, 1.0, aspect < 1.0);
        let dy = (uv.y - 0.5) * select(1.0, 1.0 / aspect, aspect < 1.0);
        let r = sqrt(dx * dx + dy * dy) * 2.0;
        let lon = safe_atan2(dx, -dy) + yaw;
        let lat = clamp(1.0 - r, -1.0, 1.0) * half_pi;
        out_uv = equirect_uv(lon, lat);
    } else if mode == 6 {
        let tan_half = tan(fov * 0.5);
        let nx0 = (2.0 * uv.x - 1.0) * tan_half * aspect;
        let ny0 = (1.0 - 2.0 * uv.y) * tan_half;
        let nx = nx0 * cos(roll) - ny0 * sin(roll);
        let ny = nx0 * sin(roll) + ny0 * cos(roll);
        let lon_off = atan(nx * 0.5);
        let lat_off = atan(ny * max(1.0 + cos(lon_off), 0.05) * 0.5);
        out_uv = equirect_uv(yaw + lon_off, clamp(pitch + lat_off, -half_pi + 0.01, half_pi - 0.01));
    } else if mode == 7 {
        let tan_half = tan(fov * 0.5);
        let nx0 = (2.0 * uv.x - 1.0) * tan_half * aspect;
        let ny0 = (1.0 - 2.0 * uv.y) * tan_half;
        let nx = nx0 * cos(roll) - ny0 * sin(roll);
        let ny = nx0 * sin(roll) + ny0 * cos(roll);
        out_uv = equirect_uv(yaw + nx, clamp(pitch + atan(ny), -half_pi + 0.01, half_pi - 0.01));
    } else if mode == 8 {
        let half = fov * 0.5;
        let nx = 2.0 * uv.x - 1.0;
        let ny = 1.0 - 2.0 * uv.y;
        out_uv = equirect_uv(yaw + nx * half * aspect, clamp(pitch + atan(sinh(ny * half)), -half_pi + 0.01, half_pi - 0.01));
    } else if mode == 10 {
        let d = camera_ray(uv, aspect, yaw, pitch, roll, fov);
        let l1 = max(abs(d.x) + abs(d.y) + abs(d.z), 0.000001);
        var ox = d.x / l1;
        var oy = d.y / l1;
        if d.z < 0.0 {
            let prev_x = ox;
            ox = (1.0 - abs(oy)) * select(-1.0, 1.0, prev_x >= 0.0);
            oy = (1.0 - abs(prev_x)) * select(-1.0, 1.0, oy >= 0.0);
        }
        out_uv = vec2(clamp(ox * 0.5 + 0.5, 0.0, 1.0), clamp(oy * 0.5 + 0.5, 0.0, 1.0));
    } else if mode == 11 || mode == 19 {
        let nx0 = (2.0 * uv.x - 1.0) * aspect;
        let ny0 = 1.0 - 2.0 * uv.y;
        let nx = nx0 * cos(roll) - ny0 * sin(roll);
        let ny = nx0 * sin(roll) + ny0 * cos(roll);
        let r = sqrt(nx * nx + ny * ny);
        if r > 1.0 { return vec3(0.0, 0.0, 0.0); }
        var theta = 0.0;
        if mode == 11 {
            let r_max = 2.0 * max(sin(fov * 0.5), 0.000001);
            theta = 2.0 * asin(clamp(r * r_max * 0.5, 0.0, 1.0));
        } else {
            let r_max = 2.0 * max(tan(fov * 0.5), 0.000001);
            theta = 2.0 * atan(r * r_max * 0.5);
        }
        let phi = safe_atan2(ny, nx);
        let x_cam = sin(theta) * cos(phi);
        let y_cam = sin(theta) * sin(phi);
        let z_cam = cos(theta);
        let y1 = y_cam * cos(pitch) - z_cam * sin(pitch);
        let z1 = y_cam * sin(pitch) + z_cam * cos(pitch);
        let x2 = x_cam * cos(yaw) + z1 * sin(yaw);
        let z2 = -x_cam * sin(yaw) + z1 * cos(yaw);
        let len = max(sqrt(x2 * x2 + y1 * y1 + z2 * z2), 0.000001);
        out_uv = equirect_uv(safe_atan2(z2 / len, x2 / len), clamp(asin(clamp(y1 / len, -1.0, 1.0)), -half_pi + 0.01, half_pi - 0.01));
    } else if mode == 12 {
        let nx0 = (2.0 * uv.x - 1.0) * aspect;
        let ny0 = 1.0 - 2.0 * uv.y;
        let nx = nx0 * cos(roll) - ny0 * sin(roll);
        let ny = nx0 * sin(roll) + ny0 * cos(roll);
        let r2 = nx * nx + ny * ny;
        if r2 > 1.0 { return vec3(0.0, 0.0, 0.0); }
        let nz = sqrt(1.0 - r2);
        let y1 = ny * cos(pitch) - nz * sin(pitch);
        let z1 = ny * sin(pitch) + nz * cos(pitch);
        let x2 = nx * cos(yaw) + z1 * sin(yaw);
        let z2 = -nx * sin(yaw) + z1 * cos(yaw);
        out_uv = equirect_uv(safe_atan2(z2, x2), clamp(asin(clamp(y1, -1.0, 1.0)), -half_pi + 0.01, half_pi - 0.01));
    } else if mode == 14 {
        let half = fov * 0.5;
        let nx = 2.0 * uv.x - 1.0;
        let ny = 1.0 - 2.0 * uv.y;
        let lat = clamp(pitch + ny * half, -half_pi + 0.01, half_pi - 0.01);
        out_uv = equirect_uv(yaw + (nx * half * aspect) / max(cos(lat), 0.001), lat);
    } else if mode == 15 {
        let half = fov * 0.5;
        let nx = 2.0 * uv.x - 1.0;
        let ny = 1.0 - 2.0 * uv.y;
        let y = pitch + ny * half;
        let lat = clamp(1.25 * atan(sinh(0.8 * y)), -half_pi + 0.01, half_pi - 0.01);
        out_uv = equirect_uv(yaw + nx * half * aspect, lat);
    } else if mode == 16 {
        let nx0 = (2.0 * uv.x - 1.0) * aspect;
        let ny0 = 1.0 - 2.0 * uv.y;
        let nx = nx0 * cos(roll) - ny0 * sin(roll);
        let ny = nx0 * sin(roll) + ny0 * cos(roll);
        let rho = sqrt(nx * nx + ny * ny);
        if rho > 1.0 { return vec3(0.0, 0.0, 0.0); }
        let c = rho * pi;
        let phi = safe_atan2(ny, nx);
        let lat = clamp(asin(clamp(cos(c) * sin(pitch) + sin(c) * cos(pitch) * cos(phi), -1.0, 1.0)), -half_pi + 0.01, half_pi - 0.01);
        let lon = yaw + safe_atan2(sin(phi) * sin(c) * cos(pitch), cos(c) - sin(pitch) * sin(lat));
        out_uv = equirect_uv(lon, lat);
    } else if mode == 17 {
        let half = fov * 0.5;
        let nx = 2.0 * uv.x - 1.0;
        let ny = 1.0 - 2.0 * uv.y;
        let y = clamp(pitch + ny * half, -1.0, 1.0);
        var theta = y;
        for (var iter = 0; iter < 4; iter = iter + 1) {
            theta = theta - (2.0 * theta + sin(theta) - pi * y) / max(2.0 + cos(theta), 0.001);
        }
        let lat = clamp(asin(clamp(theta, -1.0, 1.0)), -half_pi + 0.01, half_pi - 0.01);
        out_uv = equirect_uv(yaw + (nx * half * aspect * 1.41421356) / max(cos(theta), 0.001), lat);
    } else if mode == 18 {
        let half = fov * 0.5;
        let x = (2.0 * uv.x - 1.0) * half * aspect;
        let y = pitch + (1.0 - 2.0 * uv.y) * half;
        let z2 = 1.0 - (x * x) * 0.125 - (y * y) * 0.5;
        if z2 <= 0.0 { return vec3(0.0, 0.0, 0.0); }
        let z = sqrt(z2);
        let lon = yaw + 2.0 * atan(z * x / (2.0 * max(2.0 * z2 - 1.0, 0.001)));
        let lat = clamp(asin(clamp(y * z, -1.0, 1.0)), -half_pi + 0.01, half_pi - 0.01);
        out_uv = equirect_uv(lon, lat);
    }
    let sph_stereo = i32(round(params.view_a.z));
    if sph_stereo == 1 {
        let w = upright_luma_dims().x;
        out_uv.x = min(out_uv.x * 0.5, (floor(w * 0.5) - 0.5) / w);
    } else if sph_stereo == 2 {
        let h = upright_luma_dims().y;
        out_uv.y = min(out_uv.y * 0.5, (floor(h * 0.5) - 0.5) / h);
    }
    return vec3(out_uv, 1.0);
}

fn sample_eye_rgba(uv: vec2<f32>, graded: bool) -> vec4<f32> {
    var rgb = vec3(0.0);
    var opacity = 1.0;
    if params.adjust_b.z < 2.5 {
        var y_stored = stored(sample_plane(plane_y, uv));
        var cb_c = 0.0;
        var cr_c = 0.0;
        if params.adjust_b.z > 0.5 {
            let uv_pair = sample_plane(plane_cb, uv);
            cb_c = uv_pair.r - params.c_offset;
            cr_c = uv_pair.g - params.c_offset;
        } else {
            cb_c = stored(sample_plane(plane_cb, uv)) - params.c_offset;
            cr_c = stored(sample_plane(plane_cr, uv)) - params.c_offset;
        }
        if !graded && params.adjust_b.y > 0.5 {
            let luma = clamp(params.adjust_a.x * y_stored + params.adjust_a.y, 0.0, 255.0);
            y_stored = pow(luma / 255.0, params.adjust_a.z) * 255.0;
            let cb_turned = params.adjust_a.w * cb_c + params.adjust_b.x * cr_c;
            let cr_turned = params.adjust_a.w * cr_c - params.adjust_b.x * cb_c;
            cb_c = cb_turned;
            cr_c = cr_turned;
        }
        let y = (y_stored - params.y_offset) * params.y_gain;
        let cb = cb_c * params.c_gain;
        let cr = cr_c * params.c_gain;
        let r = y + 2.0 * (1.0 - params.kr) * cr;
        let b = y + 2.0 * (1.0 - params.kb) * cb;
        let g = (y - params.kr * r - params.kb * b) / (1.0 - params.kr - params.kb);
        rgb = clamp(vec3(r, g, b), vec3(0.0), vec3(1.0));
    } else {
        let rgba = sample_rgba(uv);
        opacity = rgba.a;
        rgb = rgba.rgb / max(opacity, 0.0000001);
        if !graded && params.adjust_b.y > 0.5 {
            rgb = clamp(adjust_codes(rgb), vec3(0.0), vec3(1.0));
        }
    }
    if graded {
        // The CPU route indexes its grid by the byte it stores, so land on that
        // byte first: the two routes then read the same nodes, and what is left
        // between them is arithmetic rather than a different colour decision.
        let code = round(rgb * 255.0) / 255.0;
        rgb = clamp(grade_read(code), vec3(0.0), vec3(1.0));
        if params.adjust_b.y > 0.5 {
            rgb = clamp(adjust_codes(rgb), vec3(0.0), vec3(1.0));
        }
    }
    return vec4(rgb, opacity);
}

fn sample_viewed_rgba(uv: vec2<f32>, graded: bool) -> vec4<f32> {
    let stereo = i32(round(params.view_a.x));
    if stereo <= 0 {
        return sample_eye_rgba(uv, graded);
    }
    let dims = upright_luma_dims();
    if stereo == 1 || stereo == 3 || stereo == 4 {
        let seam = floor(dims.x * 0.5);
        let left_u = min(uv.x * 0.5, (seam - 0.5) / dims.x);
        let right_u = max(0.5 + uv.x * 0.5, (seam + 0.5) / dims.x);
        if stereo == 3 {
            return sample_eye_rgba(vec2(left_u, uv.y), graded);
        }
        if stereo == 4 {
            return sample_eye_rgba(vec2(right_u, uv.y), graded);
        }
        let left = sample_eye_rgba(vec2(left_u, uv.y), graded);
        let right = sample_eye_rgba(vec2(right_u, uv.y), graded);
        return vec4(left.r, right.g, right.b, 1.0);
    }
    let seam = floor(dims.y * 0.5);
    let top_v = min(uv.y * 0.5, (seam - 0.5) / dims.y);
    let bottom_v = max(0.5 + uv.y * 0.5, (seam + 0.5) / dims.y);
    let top = sample_eye_rgba(vec2(uv.x, top_v), graded);
    let bottom = sample_eye_rgba(vec2(uv.x, bottom_v), graded);
    return vec4(top.r, bottom.g, bottom.b, 1.0);
}

@fragment
fn fs(in: VertexOut) -> @location(0) vec4<f32> {
    let graded = params.grid.w > 0.5;
    let projected = project_spherical_uv(in.uv);
    var rgb = vec3(0.0);
    var opacity = 1.0;
    if projected.z > 0.5 {
        let sampled = sample_viewed_rgba(projected.xy, graded);
        rgb = sampled.rgb;
        opacity = sampled.a;
    }
    // Composite final display codes onto the opaque black video backdrop.
    rgb *= opacity;
    if params.srgb > 0.5 {
        // The swapchain encodes to sRGB on write; hand it linear light.
        rgb = to_linear(rgb);
    }
    return vec4(rgb, 1.0);
}
