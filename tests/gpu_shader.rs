//! Offline translation checks. These do not exercise a driver or GPU hardware.
use naga::back::{glsl, hlsl, msl, spv};
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use naga::{AddressSpace, ImageClass, Module, ResourceBinding, ShaderStage, TypeInner};

const SOURCE: &str = include_str!("../src/gpu/transform.wgsl");
const ENTRY_POINTS: [(ShaderStage, &str); 2] = [
    (ShaderStage::Vertex, "vertex"),
    (ShaderStage::Fragment, "fragment"),
];

fn shader(source: &str) -> (Module, ModuleInfo) {
    let module = naga::front::wgsl::parse_str(source).expect("WGSL must parse");
    let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
        .validate(&module)
        .expect("shader must validate without optional capabilities");
    (module, info)
}

fn binding(index: u32) -> ResourceBinding {
    ResourceBinding {
        group: 0,
        binding: index,
    }
}

#[test]
fn shader_needs_no_compute_or_storage_resources() {
    let (module, _) = shader(SOURCE);
    assert_eq!(module.entry_points.len(), ENTRY_POINTS.len());
    for (stage, name) in ENTRY_POINTS {
        assert!(
            module
                .entry_points
                .iter()
                .any(|e| e.stage == stage && e.name == name)
        );
    }
    for (_, global) in module.global_variables.iter() {
        assert!(
            matches!(global.space, AddressSpace::Handle | AddressSpace::Uniform),
            "unexpected address space for {:?}: {:?}",
            global.name,
            global.space
        );
    }
    for (_, ty) in module.types.iter() {
        assert!(
            !matches!(
                ty.inner,
                TypeInner::Image {
                    class: ImageClass::Storage { .. },
                    ..
                }
            ),
            "storage textures would exclude the intended GL/GLES baseline"
        );
    }
}

fn check_glsl(version: glsl::Version, source: &str) {
    let (module, info) = shader(source);
    let options = glsl::Options {
        version,
        binding_map: (0..8).map(|i| (binding(i), i as u8)).collect(),
        ..Default::default()
    };
    for (stage, name) in ENTRY_POINTS {
        let pipeline = glsl::PipelineOptions {
            shader_stage: stage,
            entry_point: name.into(),
            multiview: None,
        };
        let mut source = String::new();
        let mut writer = glsl::Writer::new(
            &mut source,
            &module,
            &info,
            &options,
            &pipeline,
            Default::default(),
        )
        .unwrap_or_else(|e| panic!("{version:?} {name} writer failed: {e}"));
        writer
            .write()
            .unwrap_or_else(|e| panic!("{version:?} {name} translation failed: {e}"));
        assert!(source.contains("void main()"), "missing GLSL entry point");
    }
}

#[test]
fn translates_vertex_and_fragment_to_desktop_glsl_330() {
    check_glsl(glsl::Version::Desktop(330), SOURCE);
}

#[test]
fn translates_vertex_and_fragment_to_gles_300() {
    check_glsl(glsl::Version::new_gles(300), SOURCE);
}

#[test]
fn translates_vertex_and_fragment_to_hlsl_51() {
    check_hlsl(SOURCE);
}

fn check_hlsl(source: &str) {
    let (module, info) = shader(source);
    let options = hlsl::Options {
        shader_model: hlsl::ShaderModel::V5_1,
        fake_missing_bindings: false,
        sampler_buffer_binding_map: [(
            hlsl::SamplerIndexBufferKey { group: 0 },
            hlsl::BindTarget {
                register: 8,
                ..Default::default()
            },
        )]
        .into(),
        binding_map: (0..8)
            .map(|index| {
                (
                    binding(index),
                    hlsl::BindTarget {
                        register: index,
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    for (stage, name) in ENTRY_POINTS {
        let pipeline = hlsl::PipelineOptions {
            entry_point: Some((stage, name.into())),
        };
        let mut source = String::new();
        let reflection = hlsl::Writer::new(&mut source, &options, &pipeline)
            .write(&module, &info, None)
            .unwrap_or_else(|e| panic!("HLSL {name} translation failed: {e}"));
        assert_eq!(reflection.entry_point_names.len(), 1);
        let translated_name = reflection.entry_point_names[0]
            .as_ref()
            .unwrap_or_else(|e| panic!("HLSL {name} binding translation failed: {e:?}"));
        assert!(source.contains(translated_name));
    }
}

#[test]
fn translates_vertex_and_fragment_to_spirv_10() {
    check_spirv(SOURCE);
}

fn check_spirv(source: &str) {
    let (module, info) = shader(source);
    let options = spv::Options {
        lang_version: (1, 0),
        fake_missing_bindings: false,
        binding_map: (0..8)
            .map(|index| {
                (
                    binding(index),
                    spv::BindingInfo {
                        descriptor_set: 0,
                        binding: index,
                        binding_array_size: None,
                    },
                )
            })
            .collect(),
        capabilities: Some(
            [spv::Capability::Shader, spv::Capability::ImageQuery]
                .into_iter()
                .collect(),
        ),
        ..Default::default()
    };
    for (stage, name) in ENTRY_POINTS {
        let pipeline = spv::PipelineOptions {
            shader_stage: stage,
            entry_point: name.into(),
        };
        let words = spv::write_vec(&module, &info, &options, Some(&pipeline))
            .unwrap_or_else(|e| panic!("SPIR-V {name} translation failed: {e}"));
        assert!(words.len() > 5, "SPIR-V output contains only a header");
        assert_eq!(words[0], 0x0723_0203, "invalid SPIR-V magic");
        assert_eq!(words[1], 0x0001_0000, "unexpected SPIR-V version");
    }
}

#[test]
fn translates_vertex_and_fragment_to_metal_12() {
    check_metal(SOURCE);
}

fn check_metal(source: &str) {
    let (module, info) = shader(source);
    let resources = msl::EntryPointResources {
        resources: module
            .global_variables
            .iter()
            .filter_map(|(_, global)| {
                let resource = global.binding?;
                let mut target = msl::BindTarget::default();
                match module.types[global.ty].inner {
                    TypeInner::Image { .. } => target.texture = Some(resource.binding as u8),
                    TypeInner::Sampler { .. } => {
                        target.sampler =
                            Some(msl::BindSamplerTarget::Resource(resource.binding as u8))
                    }
                    _ => target.buffer = Some(resource.binding as u8),
                }
                Some((resource, target))
            })
            .collect(),
        ..Default::default()
    };
    let options = msl::Options {
        lang_version: (1, 2),
        fake_missing_bindings: false,
        per_entry_point_map: ENTRY_POINTS
            .map(|(_, name)| (name.into(), resources.clone()))
            .into(),
        ..Default::default()
    };
    for (stage, name) in ENTRY_POINTS {
        let pipeline = msl::PipelineOptions {
            entry_point: Some((stage, name.into())),
            ..Default::default()
        };
        let (source, reflection) = msl::write_string(&module, &info, &options, &pipeline)
            .unwrap_or_else(|e| panic!("MSL {name} translation failed: {e}"));
        assert_eq!(reflection.entry_point_names.len(), 1);
        let translated_name = reflection.entry_point_names[0]
            .as_ref()
            .unwrap_or_else(|e| panic!("MSL {name} binding translation failed: {e:?}"));
        assert!(source.contains(translated_name));
    }
}

#[cfg(feature = "gpu")]
#[test]
fn programmable_filters_translate_to_every_shader_backend() {
    for source in [
        include_str!("../shaders/negate.wgsl"),
        include_str!("../shaders/boxblur.wgsl"),
    ] {
        let shader = fvid::resident::ByteShader::new(source).unwrap();
        let source = shader.compiled_source();
        check_glsl(glsl::Version::Desktop(330), source);
        check_glsl(glsl::Version::new_gles(300), source);
        check_hlsl(source);
        check_spirv(source);
        check_metal(source);
    }
}

#[cfg(feature = "player")]
#[test]
fn display_shader_translates_all_player_paths_to_every_backend() {
    for code in [
        include_str!("../shaders/grayscale.wgsl"),
        "fn process_color(rgb: vec3<f32>, uv: vec2<f32>) -> vec3<f32> { return rgb * vec3(uv, 1.0); }",
    ] {
        let shader = fvid::player_gpu::ColorShader::new(code).unwrap();
        let source = shader
            .compiled_source()
            .replace("fn vs(", "fn vertex(")
            .replace("fn fs(", "fn fragment(");
        check_glsl(glsl::Version::Desktop(330), &source);
        check_glsl(glsl::Version::new_gles(300), &source);
        check_hlsl(&source);
        check_spirv(&source);
        check_metal(&source);
    }
}
