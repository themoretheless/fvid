pub(crate) const SHADER: &str = include_str!("player_gpu.wgsl");

/// A display-code RGB shader applied after colour management and adjustment,
/// before the framebuffer's sRGB conversion. Implement
/// `fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32>`.
#[derive(Clone, Debug)]
pub struct ColorShader {
    compiled: String,
}
impl ColorShader {
    pub fn new(source: &str) -> crate::Result<Self> {
        if source.len() > 64 * 1024 {
            return Err(crate::invalid("shader source exceeds 64 KiB"));
        }
        let compiled = format!("{}\n{}", SHADER.replace(
            "    // Composite final display codes onto the opaque black video backdrop.",
            "    rgb = clamp(process_color(rgb, in.uv), vec3(0.0), vec3(1.0));\n    // Composite final display codes onto the opaque black video backdrop."
        ), source);
        let module = naga::front::wgsl::parse_str(&compiled)
            .map_err(|error| crate::Error::Invalid(error.emit_to_string(&compiled)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|error| crate::Error::Invalid(error.emit_to_string(&compiled)))?;
        if module.entry_points.len() != 2 || module.global_variables.len() != 8 {
            return Err(crate::invalid(
                "display shader must only add functions and constants",
            ));
        }
        Ok(Self { compiled })
    }
    pub fn compiled_source(&self) -> &str {
        &self.compiled
    }
    /// Encoder fragment entries for explicit output dimensions and quarter-turn rotation. The
    /// renderer must set `params.srgb` to zero (encoder input is display codes).
    /// Chroma averages four RGB samples before 4:2:0 subsampling.
    pub fn encoder_source(
        &self,
        depth: u8,
        full_range: bool,
        kr: f32,
        kb: f32,
    ) -> crate::Result<String> {
        if ![8, 10].contains(&depth)
            || !kr.is_finite()
            || !kb.is_finite()
            || kr <= 0.0
            || kb <= 0.0
            || kr + kb >= 1.0
        {
            return Err(crate::invalid("invalid encoder depth or YUV coefficients"));
        }
        let (max, offset, range, chroma_range, pack) = if depth == 8 {
            (
                255.0,
                if full_range { 0.0 } else { 16.0 },
                if full_range { 255.0 } else { 219.0 },
                if full_range { 255.0 } else { 224.0 },
                1.0 / 255.0,
            )
        } else {
            (
                1023.0,
                if full_range { 0.0 } else { 64.0 },
                if full_range { 1023.0 } else { 876.0 },
                if full_range { 1023.0 } else { 896.0 },
                64.0 / 65535.0,
            )
        };
        let center = if depth == 8 { 128.0 } else { 512.0 };
        let mut source = self.compiled.replace(
            "@fragment\nfn fs(in: VertexOut) -> @location(0) vec4<f32>",
            "fn display_codes(in: VertexOut) -> vec4<f32>",
        );
        source.push_str(&format!(
            r#"
fn encoder_yuv(rgb: vec3<f32>) -> vec3<f32> {{
    let y = dot(rgb, vec3<f32>({kr},{kg},{kb}));
    return vec3(y, (rgb.b-y)/(2.0*(1.0-{kb})), (rgb.r-y)/(2.0*(1.0-{kr})));
}}
@fragment fn encoder_y(in: VertexOut) -> @location(0) vec4<f32> {{
    let y = encoder_yuv(display_codes(in).rgb).x;
    let code = round(clamp({offset}+y*{range},0.0,{max}));
    return vec4(code*{pack},0.0,0.0,1.0);
}}
@fragment fn encoder_uv(in: VertexOut) -> @location(0) vec4<f32> {{
    let size = params.encoder_size.xy;
    let delta = vec2<f32>(1.0) / size;
    let origin = (floor(in.position.xy)*2.0+vec2(0.5))/size;
    let last = (size-vec2(0.5))/size;
    var point = in;
    point.uv = mix(params.window.xy,params.window.zw,min(origin,last));
    var rgb = display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + vec2(delta.x,0.0),last));
    rgb += display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + vec2(0.0,delta.y),last));
    rgb += display_codes(point).rgb;
    point.uv = mix(params.window.xy,params.window.zw,min(origin + delta,last));
    rgb += display_codes(point).rgb;
    let uv = encoder_yuv(rgb*0.25).yz;
    let code = round(clamp(vec2({center})+uv*{chroma_range},vec2(0.0),vec2({max})));
    return vec4(code*{pack},0.0,1.0);
}}
"#,
            kg = 1.0 - kr - kb
        ));
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|e| crate::invalid(&e.emit_to_string(&source)))?;
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .map_err(|e| crate::invalid(&e.emit_to_string(&source)))?;
        Ok(source)
    }
}
