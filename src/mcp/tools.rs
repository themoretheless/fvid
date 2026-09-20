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
    scale: Option<[u32; 2]>,
    transpose: Option<String>,
    rotate: Option<f64>,
    pad: Option<[u32; 4]>,
    rate: Option<i32>,
    channels: Option<i32>,
    volume: Option<f64>,
    #[serde(default)]
    normalize: Option<bool>,
    #[serde(default)]
    weights: Vec<f32>,
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
        "Strict packet-copy trim [from,to). Closed-GOP mid-GOP pre/post-roll and open-GOP end/start from preceding IDR/IRAP; rejects unsafe boundaries. Use transcode_lossless when stream-copy cannot qualify.",
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
        "FFV1/Matroska export of decoded samples, optional crop [x,y,width,height], scale [width,height] (neighbor), transpose, rotate degrees, pad [width,height,x,y] (black), hflip/vflip and interval. Interval supports PCM audio; seek is video-only.",
        "input output streams crop scale transpose rotate pad hflip vflip from to seek",
        "input output",
    ),
    (
        "fvid_transcode",
        "Encode video with explicit encoder/quality. May be lossy. Other streams copied except PCM interval slicing. No automatic pixel-format conversion; optional neighbor scale, transpose, rotate and pad.",
        "input output streams crop scale transpose rotate pad hflip vflip from to seek encoder encoder_options",
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
        "Decode one selected audio stream to PCM retaining sample precision; optional rate/channels (libswresample) and volume (linear gain on float PCM). Contiguous decoded samples, no synthesized timestamp gaps.",
        "input output streams rate channels volume from to",
        "input output",
    ),
    (
        "fvid_mix_audio",
        "Mix 2..=16 float audio inputs to PCM WAV with FFmpeg amix=duration=shortest semantics; optional weights; normalize defaults true.",
        "inputs output normalize weights",
        "inputs output",
    ),
    (
        "fvid_merge_audio",
        "Channel-merge exactly two float audio inputs to PCM WAV (FFmpeg amerge=inputs=2); output channels are the sum; duration is shortest.",
        "inputs output",
        "inputs output",
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
    let scale = json!({"type":"array","items":{"type":"integer","minimum":1},"minItems":2,"maxItems":2,"description":"[width,height] neighbor scale after crop/flips/transpose/pad"});
    let transpose = json!({"type":"string","enum":["clock","cclock","clock_flip","cclock_flip"],"description":"FFmpeg transpose= mode after crop/flips and before rotate/pad/scale"});
    let rotate = json!({"type":"number","minimum":-3600,"maximum":3600,"description":"rotation degrees; FFmpeg rotate=a=DEG*PI/180:ow=rotw(a):oh=roth(a):c=black"});
    let pad = json!({"type":"array","items":{"type":"integer","minimum":0},"minItems":4,"maxItems":4,"description":"[width,height,x,y] black pad after rotate and before scale"});
    let stage = json!({"type":"object","additionalProperties":false,"properties":{"crop":crop,"hflip":{"type":"boolean"},"vflip":{"type":"boolean"}}});
    json!({
        "input":{"type":"string","minLength":1,"maxLength":4096},"output":{"type":"string","minLength":1,"maxLength":4096},
        "inputs":{"type":"array","items":{"type":"string","minLength":1,"maxLength":4096},"minItems":2,"maxItems":256},
        "streams":{"type":"array","items":{"type":"integer","minimum":0},"maxItems":64},
        "from":{"type":"string","pattern":"^[0-9]+(\\.[0-9]{1,6})?$","maxLength":32},
        "to":{"type":"string","pattern":"^[0-9]+(\\.[0-9]{1,6})?$","maxLength":32},
        "crop":crop,"scale":scale,"transpose":transpose,"rotate":rotate,"pad":pad,"hflip":{"type":"boolean"},"vflip":{"type":"boolean"},"seek":{"type":"boolean"},
        "rate":{"type":"integer","minimum":8000,"maximum":384000,"description":"target sample rate for decode-audio (libswresample)"},
        "channels":{"type":"integer","minimum":1,"maximum":64,"description":"target channel count for decode-audio (FFmpeg -ac via libswresample)"},
        "volume":{"type":"number","minimum":0,"maximum":64,"description":"linear gain for decode-audio float PCM (FFmpeg volume=)"},
        "normalize":{"type":"boolean","description":"amix normalize (default true) for mix-audio"},
        "weights":{"type":"array","items":{"type":"number"},"minItems":1,"maxItems":16,"description":"amix per-input weights; shorter lists repeat the last weight"},
        "encoder":{"type":"string","minLength":1,"maxLength":128},
        "encoder_options":{"type":"object","additionalProperties":false,"properties":{
            "crf":{"type":"string","maxLength":32},"preset":{"type":"string","maxLength":128},"tune":{"type":"string","maxLength":128},
            "lossless":{"type":"string","maxLength":16},"deadline":{"type":"string","maxLength":32},"cpu-used":{"type":"string","maxLength":16},
            "threads":{"type":"string","maxLength":16},"bf":{"type":"string","maxLength":16},"g":{"type":"string","maxLength":16},"level":{"type":"string","maxLength":16},
            "x265-params":{"type":"string","maxLength":512},"svtav1-params":{"type":"string","maxLength":512}},
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
    } else if name == "fvid_mix_audio" || name == "fvid_merge_audio" {
        if name == "fvid_mix_audio" {
            if !(2..=16).contains(&a.inputs.len()) {
                return Err("mix-audio requires 2..=16 inputs".into());
            }
        } else if a.inputs.len() != 2 {
            return Err("merge-audio requires exactly two inputs".into());
        }
        let sources = a
            .inputs
            .iter()
            .map(|p| server.input(p))
            .collect::<Result<Vec<_>, _>>()?;
        if name == "fvid_mix_audio" {
            encode(media::mix_audio(
                &sources,
                &output,
                &media::MixAudioOptions {
                    duration: media::MixDuration::Shortest,
                    normalize: a.normalize.unwrap_or(true),
                    weights: a.weights.clone(),
                },
            )?)?
        } else {
            encode(media::merge_audio(&sources, &output)?)?
        }
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
            "fvid_decode_audio" => {
                let transform = media::AudioDecodeTransform {
                    interval: interval(&a)?,
                    sample_rate: a.rate,
                    channels: a.channels,
                    volume: a.volume,
                };
                encode(media::decode_audio_transformed(
                    &input, &output, transform, &options,
                )?)?
            }
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
                    scale: a.scale.map(|[width, height]| media::ScaleSize { width, height }),
                    transpose: a
                        .transpose
                        .as_deref()
                        .map(media::TransposeMode::parse)
                        .transpose()?,
                    rotate: match a.rotate {
                        Some(degrees) => Some(media::RotateAngle::parse(&degrees.to_string())?),
                        None => None,
                    },
                    pad: a.pad.map(|[width, height, x, y]| media::PadRect {
                        width,
                        height,
                        x,
                        y,
                    }),
                    burn_subs: None,
                    overlay: None,
                    colorspace: None,
                    zscale: None,
                    tonemap: None,
                    xfade: None,
                    yadif: None,
                    bwdif: None,
                    w3fdif: None,
                    tblend: None,
                    tmix: None,
                    hqdn3d: None,
                    gblur: None,
                    eq: None,
                    unsharp: None,
                    hue: None,
                    avgblur: None,
                    boxblur: None,
                    negate: None,
                    edgedetect: None,
                    sobel: None,
                    prewitt: None,
                    roberts: None,
                    kirsch: None,
                    scharr: None,
                    atadenoise: None,
                    owdenoise: None,
                    vaguedenoiser: None,
                    nlmeans: None,
                    bm3d: None,
                    dctdnoiz: None,
                    fftdnoiz: None,
                    smartblur: None,
                    sab: None,
                    bilateral: None,
                    cas: None,
                    epx: None,
                    vignette: None,
                    curves: None,
                    colorbalance: None,
                    colorlevels: None,
                    colorchannelmixer: None,
                    deflicker: None,
                    photosensitivity: None,
                    monochrome: None,
                    grayworld: None,
                    drawbox: None,
                    drawgrid: None,
                    lagfun: None,
                    amplify: None,
                    bitplanenoise: None,
                    deband: None,
                    gradfun: None,
                    lenscorrection: None,
                    pixelize: None,
                    removegrain: None,
                    yaepblur: None,
                    vibrance: None,
                    dilation: None,
                    erosion: None,
                    colorize: None,
                    exposure: None,
                    chromashift: None,
                    colorcontrast: None,
                    colorcorrect: None,
                    histeq: None,
                    shuffleplanes: None,
                    lutyuv: None,
                    colorhold: None,
                    fade: None,
                    perspective: None,
                    lumakey: None,
                    chromakey: None,
                    colorkey: None,
                    despill: None,
                    selectivecolor: None,
                    stereo3d: None,
                    field: None,
                    hqx: None,
                    xbr: None,
                    il: None,
                    super2xsai: None,
                    kerndeint: None,
                    phase: None,
                    estdif: None,
                    tinterlace: None,
                    separatefields: None,
                    weave: None,
                    doubleweave: None,
                    framepack: None,
                    telecine: None,
                    pullup: None,
                    decimate: None,
                    mpdecimate: None,
                    framestep: None,
                    tile: None,
                    untile: None,
                    shuffleframes: None,
                    reverse: None,
                    r#loop: None,
                    thumbnail: None,
                    pseudocolor: None,
                    minterpolate: None,
                    fps: None,
                    pix_fmt: None,
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
    crate::publish::publish_file(&path, output).map_err(|e| e.to_string())?;
    Ok(
        json!({"frames":stats.frames,"input_bytes":stats.input_bytes,"output_bytes":stats.output_bytes,"backend":stats.backend.to_string(),"device":stats.device_name,"controlled_memory_bytes":stats.controlled_memory_bytes,"transfers":transfers}),
    )
}
