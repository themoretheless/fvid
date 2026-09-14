use super::Server;
use crate::media;
use rmcp::model::{Tool, ToolAnnotations};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    input: Option<String>,
    output: Option<String>,
    #[serde(default)]
    inputs: Vec<String>,
    #[serde(default)]
    streams: Vec<usize>,
    from: Option<String>,
    to: Option<String>,
    crop: Option<[usize; 4]>,
    #[serde(default)]
    hflip: bool,
    #[serde(default)]
    vflip: bool,
    #[serde(default)]
    seek: bool,
    encoder: Option<String>,
    #[serde(default)]
    encoder_options: BTreeMap<String, String>,
    backend: Option<String>,
    device: Option<usize>,
    memory_mib: Option<usize>,
    #[serde(default)]
    stages: Vec<Stage>,
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stage {
    crop: Option<[usize; 4]>,
    #[serde(default)]
    hflip: bool,
    #[serde(default)]
    vflip: bool,
}
const DEFINITIONS: &[(&str, &str, &str, &str)] = &[
    (
        "fvid_capabilities",
        "Inventory of linked codecs/containers; inventory is not verified workflow coverage.",
        "",
        "",
    ),
    (
        "fvid_devices",
        "List CPU and GPU adapters with availability reasons; does not initialize a media job.",
        "",
        "",
    ),
    (
        "fvid_probe",
        "Inspect local media streams, codecs and exact time bases.",
        "input",
        "input",
    ),
    (
        "fvid_remux",
        "Copy compressed packets into a new container without decoding; supports stream selection.",
        "input output streams",
        "input output",
    ),
    (
        "fvid_trim",
        "Strict packet-copy trim [from,to). Rejects B-frames and unsafe boundaries; use transcode_lossless for frame selection inside GOP.",
        "input output streams from to",
        "input output from to",
    ),
    (
        "fvid_concat",
        "Concatenate compatible streams without encoding. Strict timestamps/configuration checks; incompatible inputs fail.",
        "inputs output streams",
        "inputs output",
    ),
    (
        "fvid_transcode_lossless",
        "FFV1/Matroska export of decoded samples, optional crop [x,y,width,height], hflip/vflip and interval. Interval supports PCM audio; seek is video-only.",
        "input output streams crop hflip vflip from to seek",
        "input output",
    ),
    (
        "fvid_transcode",
        "Encode video with explicit encoder/quality. May be lossy. Other streams copied except PCM interval slicing. No automatic pixel conversion.",
        "input output streams crop hflip vflip from to seek encoder encoder_options",
        "input output encoder",
    ),
    (
        "fvid_trim_pcm",
        "Sample-exact packed PCM slicing, including within packets; selected streams must all be PCM audio.",
        "input output streams from to",
        "input output from to",
    ),
    (
        "fvid_decode_audio",
        "Decode one selected audio stream to PCM retaining sample precision; contiguous decoded samples, no synthesized timestamp gaps.",
        "input output streams",
        "input output",
    ),
    (
        "fvid_process_y4m",
        "Rust Y4M 8-bit 420/422/444 crop/flips. Explicit GPU backend never falls back. Additional stages run as a resident GPU chain; memory_mib limits controlled frame/staging buffers.",
        "input output crop hflip vflip backend device memory_mib stages",
        "input output",
    ),
];
fn properties() -> Map<String, Value> {
    let crop = json!({"type":"array","items":{"type":"integer","minimum":0},"minItems":4,"maxItems":4,"description":"[x,y,width,height]"});
    let stage = json!({"type":"object","additionalProperties":false,"properties":{"crop":crop,"hflip":{"type":"boolean"},"vflip":{"type":"boolean"}}});
    json!({
        "input":{"type":"string","minLength":1,"maxLength":4096},"output":{"type":"string","minLength":1,"maxLength":4096},
        "inputs":{"type":"array","items":{"type":"string","minLength":1,"maxLength":4096},"minItems":2,"maxItems":256},
        "streams":{"type":"array","items":{"type":"integer","minimum":0},"maxItems":64},
        "from":{"type":"string","pattern":"^[0-9]+(\\.[0-9]{1,6})?$","maxLength":32},
        "to":{"type":"string","pattern":"^[0-9]+(\\.[0-9]{1,6})?$","maxLength":32},
        "crop":crop,"hflip":{"type":"boolean"},"vflip":{"type":"boolean"},"seek":{"type":"boolean"},
        "encoder":{"type":"string","minLength":1,"maxLength":128},
        "encoder_options":{"type":"object","additionalProperties":false,"properties":{
            "crf":{"type":"string","maxLength":32},"preset":{"type":"string","maxLength":128},"tune":{"type":"string","maxLength":128},
            "lossless":{"type":"string","maxLength":16},"deadline":{"type":"string","maxLength":32},"cpu-used":{"type":"string","maxLength":16},
            "threads":{"type":"string","maxLength":16},"bf":{"type":"string","maxLength":16},"g":{"type":"string","maxLength":16},"level":{"type":"string","maxLength":16}}},
        "backend":{"type":"string","enum":["cpu","auto","metal","vulkan","dx12","gl","cuda"]},
        "device":{"type":"integer","minimum":0},"memory_mib":{"type":"integer","minimum":1,"maximum":4096},
        "stages":{"type":"array","items":stage,"maxItems":255}
    }).as_object().unwrap().clone()
}
pub(super) fn catalog() -> &'static [Tool] {
    static CATALOG: std::sync::OnceLock<Vec<Tool>> = std::sync::OnceLock::new();
    CATALOG.get_or_init(build_catalog)
}
fn build_catalog() -> Vec<Tool> {
    let fields = properties();
    DEFINITIONS.iter().map(|&(name, description, allowed, required)| {
        let properties: Map<_, _> = allowed.split_whitespace().map(|key| (key.to_owned(), fields[key].clone())).collect();
        let schema = json!({"type":"object","additionalProperties":false,"properties":properties,"required":required.split_whitespace().collect::<Vec<_>>()});
        let mut tool = Tool::new(name, description, schema.as_object().unwrap().clone());
        let mut annotation = ToolAnnotations::default();
        annotation.read_only_hint = Some(matches!(name, "fvid_probe" | "fvid_devices" | "fvid_capabilities"));
        annotation.destructive_hint = Some(false);
        annotation.open_world_hint = Some(false);
        tool.annotations = Some(annotation);
        tool
    }).collect()
}
fn encode<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|e| e.to_string())
}
fn required<'a>(value: &'a Option<String>, name: &str) -> Result<&'a str, String> {
    value.as_deref().ok_or_else(|| format!("{name} required"))
}
fn interval(args: &Args) -> Result<Option<(i64, i64)>, String> {
    match (&args.from, &args.to) {
        (None, None) => Ok(None),
        (Some(from), Some(to)) => Ok(Some((media::parse_time(from)?, media::parse_time(to)?))),
        _ => Err("from and to must be supplied together".into()),
    }
}
pub(super) fn execute(
    server: &Server,
    name: &str,
    arguments: Map<String, Value>,
) -> Result<Value, String> {
    let definition = DEFINITIONS
        .iter()
        .find(|d| d.0 == name)
        .ok_or("unknown tool")?;
    for key in arguments.keys() {
        if !definition
            .2
            .split_whitespace()
            .any(|allowed| allowed == key)
        {
            return Err(format!("unexpected argument: {key}"));
        }
    }
    for key in definition.3.split_whitespace() {
        if !arguments.contains_key(key) {
            return Err(format!("{key} required"));
        }
    }
    let a: Args = serde_json::from_value(Value::Object(arguments)).map_err(|e| e.to_string())?;
    if a.streams.len() > 64 || a.inputs.len() > 256 || a.stages.len() > 255 {
        return Err("argument count exceeds limit".into());
    }
    if a.encoder_options.keys().any(|k| {
        ![
            "crf", "preset", "tune", "lossless", "deadline", "cpu-used", "threads", "bf", "g",
            "level",
        ]
        .contains(&k.as_str())
    }) {
        return Err("encoder option is not exposed over MCP".into());
    }
    if name == "fvid_capabilities" {
        return Ok(
            json!({"library":media::capabilities(),"note":"Library inventory, not tested workflow coverage","tools":DEFINITIONS.iter().map(|d| d.0).collect::<Vec<_>>() }),
        );
    }
    if name == "fvid_devices" {
        return Ok(
            json!({"cpu":true,"backends":crate::backend::devices().into_iter().map(|b| json!({"backend":b.backend.to_string(),"unavailable_reason":b.unavailable_reason,"devices":b.devices.into_iter().map(|d| json!({"ordinal":d.ordinal,"name":d.name,"type":d.device_type})).collect::<Vec<_>>() })).collect::<Vec<_>>() }),
        );
    }
    let options = media::CopyOptions {
        streams: a.streams.clone(),
        ..Default::default()
    };
    if name == "fvid_probe" {
        return encode(media::probe(&server.input(required(&a.input, "input")?)?)?);
    }
    let output = server.output(required(&a.output, "output")?)?;
    let result = if name == "fvid_concat" {
        if a.inputs.len() < 2 {
            return Err("concat requires at least two inputs".into());
        }
        let sources = a
            .inputs
            .iter()
            .map(|p| server.input(p))
            .collect::<Result<Vec<_>, _>>()?;
        encode(media::concat(&sources, &output, &options)?)?
    } else {
        let input = server.input(required(&a.input, "input")?)?;
        match name {
            "fvid_remux" => encode(media::remux(&input, &output, &options)?)?,
            "fvid_trim" | "fvid_trim_pcm" => {
                let (from, to) = interval(&a)?.ok_or("interval required")?;
                if name == "fvid_trim" {
                    encode(media::trim(&input, &output, from, to, &options)?)?
                } else {
                    encode(media::trim_pcm(&input, &output, from, to, &options)?)?
                }
            }
            "fvid_decode_audio" => encode(media::decode_audio(&input, &output, &options)?)?,
            "fvid_transcode" | "fvid_transcode_lossless" => {
                let transform = media::LosslessTransform {
                    crop: a.crop.map(|[x, y, width, height]| media::CropRect {
                        x,
                        y,
                        width,
                        height,
                    }),
                    horizontal_flip: a.hflip,
                    vertical_flip: a.vflip,
                    interval: interval(&a)?,
                    seek: a.seek,
                };
                if name == "fvid_transcode_lossless" {
                    encode(media::transcode_lossless(
                        &input, &output, transform, &options,
                    )?)?
                } else {
                    encode(media::transcode(
                        &input,
                        &output,
                        transform,
                        &options,
                        &media::EncoderSettings {
                            name: required(&a.encoder, "encoder")?.into(),
                            options: a.encoder_options.clone().into_iter().collect(),
                        },
                    )?)?
                }
            }
            "fvid_process_y4m" => y4m(&input, &output, &a)?,
            _ => return Err("unknown tool".into()),
        }
    };
    Ok(json!({"output":output.to_string_lossy(),"stats":result}))
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn y4m(input: &std::path::Path, output: &std::path::Path, args: &Args) -> Result<Value, String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let transform = |crop: Option<[usize; 4]>, horizontal, vertical| crate::Transform {
        crop: crop.map(|[x, y, width, height]| crate::Crop {
            x,
            y,
            width,
            height,
        }),
        horizontal,
        vertical,
    };
    let mut transforms = vec![transform(args.crop, args.hflip, args.vflip)];
    transforms.extend(
        args.stages
            .iter()
            .map(|s| transform(s.crop, s.hflip, s.vflip)),
    );
    let options = crate::ExecutionOptions {
        backend: args
            .backend
            .as_deref()
            .unwrap_or("cpu")
            .parse()
            .map_err(|e: crate::Error| e.to_string())?,
        device: args.device.unwrap_or(0),
    };
    let memory = args.memory_mib.unwrap_or(256);
    if !(1..=4096).contains(&memory) {
        return Err("memory_mib must be 1..4096".into());
    }
    let memory = memory
        .checked_mul(1024 * 1024)
        .ok_or("memory budget overflow")?;
    let path = output.parent().unwrap().join(format!(
        ".fvid-mcp-{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let _temporary = Temporary(path.clone());
    let reader = BufReader::new(File::open(input).map_err(|e| e.to_string())?);
    let mut writer = BufWriter::new(file);
    let (stats, transfers) = if transforms.len() == 1 {
        (
            crate::process_with_options(reader, &mut writer, transforms[0], memory, options)
                .map_err(|e| e.to_string())?,
            Value::Null,
        )
    } else {
        #[cfg(any(feature = "gpu", feature = "cuda"))]
        {
            let (stats, t) =
                crate::process_gpu_chain(reader, &mut writer, &transforms, memory, options)
                    .map_err(|e| e.to_string())?;
            (
                stats,
                json!({"uploads":t.uploads,"downloads":t.downloads,"upload_bytes":t.upload_bytes,"download_bytes":t.download_bytes,"upload_staging_bytes":t.upload_staging_bytes,"download_staging_bytes":t.download_staging_bytes,"filter_passes":t.filter_passes}),
            )
        }
        #[cfg(not(any(feature = "gpu", feature = "cuda")))]
        {
            return Err("GPU stages require a GPU-enabled build".into());
        }
    };
    writer.flush().map_err(|e| e.to_string())?;
    drop(writer);
    std::fs::hard_link(&path, output).map_err(|e| e.to_string())?;
    Ok(
        json!({"frames":stats.frames,"input_bytes":stats.input_bytes,"output_bytes":stats.output_bytes,"backend":stats.backend.to_string(),"device":stats.device_name,"controlled_memory_bytes":stats.controlled_memory_bytes,"transfers":transfers}),
    )
}
