#[cfg(feature = "media")]
use std::path::PathBuf;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "media")]
    {
        run_native(args)
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = args;
        Err("media commands require cargo build --release --features media and FFmpeg development libraries".into())
    }
}
#[cfg(feature = "media")]
fn run_native(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let help = "fvid media probe INPUT | capabilities | remux INPUT OUTPUT [--streams 0,1] | decode-audio INPUT OUTPUT.wav [--streams INDEX] | trim-pcm INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] | trim INPUT OUTPUT --from SECONDS --to SECONDS [--streams 0] | concat OUTPUT INPUT INPUT... [--streams 0] | transcode INPUT OUTPUT --encoder NAME [--encoder-option KEY=VALUE] | transcode-lossless INPUT OUTPUT.mkv [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--from SECONDS --to SECONDS [--seek]] [--streams 0,1] | crop-lossless INPUT OUTPUT.mkv --crop X:Y:WIDTH:HEIGHT [--streams 0,1] | hw-filter INPUT OUTPUT.mp4 [--crop X:Y:WIDTH:HEIGHT] [--hflip] [--vflip] [--device N]";
    let Some(command) = args.first() else {
        return Err(help.into());
    };
    if command == "--help" || command == "-h" {
        println!("{help}");
        return Ok(());
    }
    if command == "capabilities" {
        if args.len() != 1 {
            return Err(help.into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&fvid_media::capabilities())?
        );
        return Ok(());
    }
    if command == "probe" {
        if args.len() != 2 {
            return Err(help.into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&fvid_media::probe(&PathBuf::from(&args[1]))?)?
        );
        return Ok(());
    }
    let mut paths = Vec::new();
    let mut from = None;
    let mut to = None;
    let mut crop = None;
    let mut vertical_flip = false;
    let mut horizontal_flip = false;
    let mut seek = false;
    let mut encoder = None;
    let mut encoder_options = Vec::new();
    let mut device = 0usize;
    let mut options = fvid_media::CopyOptions::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--encoder" => {
                i += 1;
                encoder = Some(args.get(i).ok_or("missing encoder name")?.clone());
            }
            "--encoder-option" => {
                i += 1;
                let (key, value) = args
                    .get(i)
                    .ok_or("missing encoder option")?
                    .split_once('=')
                    .ok_or("encoder option must be KEY=VALUE")?;
                encoder_options.push((key.to_owned(), value.to_owned()));
            }
            "--device" => {
                i += 1;
                device = args.get(i).ok_or("missing device")?.parse()?;
            }
            "--seek" => seek = true,
            "--hflip" => horizontal_flip = true,
            "--vflip" => vertical_flip = true,
            "--crop" => {
                i += 1;
                let values = args
                    .get(i)
                    .ok_or("missing crop")?
                    .split(':')
                    .map(str::parse)
                    .collect::<Result<Vec<usize>, _>>()?;
                if values.len() != 4 {
                    return Err("crop must be X:Y:WIDTH:HEIGHT".into());
                }
                crop = Some(fvid_media::CropRect {
                    x: values[0],
                    y: values[1],
                    width: values[2],
                    height: values[3],
                });
            }
            "--streams" => {
                i += 1;
                options.streams = args
                    .get(i)
                    .ok_or("missing stream indices")?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?;
            }
            "--from" => {
                i += 1;
                from = Some(fvid_media::parse_time(args.get(i).ok_or("missing start")?)?);
            }
            "--to" => {
                i += 1;
                to = Some(fvid_media::parse_time(args.get(i).ok_or("missing end")?)?);
            }
            value if value.starts_with("--") => {
                return Err(format!("unknown media option: {value}").into());
            }
            value => paths.push(PathBuf::from(value)),
        }
        i += 1;
    }
    if command != "transcode" && (encoder.is_some() || !encoder_options.is_empty()) {
        return Err("encoder selection requires explicit transcode command".into());
    }
    if command == "hw-filter" && paths.len() == 2 {
        #[cfg(feature = "media-cuda")]
        {
            let stats = fvid_media::hw_filter(
                &paths[0],
                &paths[1],
                &fvid_media::HwFilterOptions {
                    crop,
                    horizontal_flip,
                    vertical_flip,
                    device,
                    host_bounce: false,
                },
            )?;
            println!("{}", serde_json::to_string_pretty(&stats)?);
            return Ok(());
        }
        #[cfg(not(feature = "media-cuda"))]
        {
            let _ = device;
            return Err(
                "hw-filter requires cargo build --release --features media-cuda (CUDA NVDEC/NVENC)"
                    .into(),
            );
        }
    }
    if (command == "crop-lossless" || command == "transcode-lossless" || command == "transcode")
        && paths.len() == 2
    {
        if command == "crop-lossless" && crop.is_none() {
            return Err("--crop required".into());
        }
        let interval = match (from, to) {
            (None, None) => None,
            (Some(start), Some(end)) => Some((start, end)),
            _ => return Err("lossless interval requires both --from and --to".into()),
        };
        let transform = fvid_media::LosslessTransform {
            crop,
            vertical_flip,
            horizontal_flip,
            interval,
            seek,
        };
        let stats = if command == "transcode" {
            fvid_media::transcode(
                &paths[0],
                &paths[1],
                transform,
                &options,
                &fvid_media::EncoderSettings {
                    name: encoder
                        .ok_or("transcode requires --encoder; quality is defined by its options")?,
                    options: encoder_options,
                },
            )?
        } else {
            fvid_media::transcode_lossless(&paths[0], &paths[1], transform, &options)?
        };
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if crop.is_some() || vertical_flip || horizontal_flip || seek {
        return Err(
            "--crop/--hflip/--vflip/--seek require crop-lossless, transcode-lossless, or hw-filter"
                .into(),
        );
    }
    if command == "decode-audio" && paths.len() == 2 && from.is_none() && to.is_none() {
        let stats = fvid_media::decode_audio(&paths[0], &paths[1], &options)?;
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    if command == "trim-pcm" && paths.len() == 2 {
        let stats = fvid_media::trim_pcm(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?;
        println!("{}", serde_json::to_string_pretty(&stats)?);
        return Ok(());
    }
    let stats = match command.as_str() {
        "remux" if paths.len() == 2 && from.is_none() && to.is_none() => {
            fvid_media::remux(&paths[0], &paths[1], &options)?
        }
        "trim" if paths.len() == 2 => fvid_media::trim(
            &paths[0],
            &paths[1],
            from.ok_or("--from required")?,
            to.ok_or("--to required")?,
            &options,
        )?,
        "concat" if paths.len() >= 3 && from.is_none() && to.is_none() => {
            fvid_media::concat(&paths[1..], &paths[0], &options)?
        }
        _ => return Err(help.into()),
    };
    println!("{}", serde_json::to_string_pretty(&stats)?);
    Ok(())
}
