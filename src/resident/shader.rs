use crate::{Error, Result};

/// Validated WGSL planar filter. Implement
/// `fn process_byte(value: u32, plane: u32, x: u32, y: u32) -> u32`.
/// Plane 0 is Y, 1 Cb, 2 Cr; coordinates are local to the transformed plane.
/// `sample(plane, x, y)` reads a clamped neighbour after crop/reflections.
/// Values are saturated to 0..255; no normalized or sRGB conversion occurs.
/// This is a GPU program, not a sandbox with bounded execution time.
#[derive(Clone, Debug)]
pub struct ByteShader {
    compiled: String,
}
impl ByteShader {
    /// Parse and validate before any GPU is initialized. Source is limited to 64 KiB.
    pub fn new(source: &str) -> Result<Self> {
        if source.len() > 64 * 1024 {
            return Err(Error::Invalid("shader source exceeds 64 KiB".into()));
        }
        let compiled = format!(
            "{}\n{}\n{}",
            include_str!("../gpu/transform.wgsl")
                .replace("fn gather(output_index:", "fn raw_gather(output_index:"),
            HELPERS,
            source
        );
        let module = naga::front::wgsl::parse_str(&compiled)
            .map_err(|error| Error::Invalid(error.emit_to_string(&compiled)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| Error::Invalid(error.emit_to_string(&compiled)))?;
        // Users can add functions/constants, but cannot alter the resource ABI or
        // add a compute stage with unbudgeted buffers.
        if module.entry_points.len() != 2 || module.global_variables.len() != 2 {
            return Err(Error::Invalid("shader must only add functions and constants; extra entry points or globals are unsupported".into()));
        }
        Ok(Self { compiled })
    }
    pub fn compiled_source(&self) -> &str {
        &self.compiled
    }
}

const HELPERS: &str = r#"
fn sample(plane: u32, x: i32, y: i32) -> u32 {
    let base = min(plane, 2u) * 8u;
    let width = parameter(base + 5u);
    let height = parameter(base + 6u);
    let sx = u32(clamp(x, 0, i32(width) - 1));
    let sy = u32(clamp(y, 0, i32(height) - 1));
    return raw_gather(parameter(base + 1u) + sy * width + sx);
}
fn gather(output_index: u32) -> u32 {
    if output_index >= parameter(27u) { return 0u; }
    var plane = 0u;
    if output_index >= parameter(9u) { plane = 1u; }
    if output_index >= parameter(17u) { plane = 2u; }
    let base = plane * 8u;
    let offset = output_index - parameter(base + 1u);
    let width = parameter(base + 5u);
    return min(process_byte(raw_gather(output_index), plane, offset % width, offset / width), 255u);
}
"#;
