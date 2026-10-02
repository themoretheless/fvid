//! The loudnorm command, preferring owned audio decoding and normalization.
use std::path::Path;
pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() == 2 && matches!(args[1].as_str(), "--help" | "-h") {
        println!(
            "fvid media loudnorm INPUT OUTPUT.wav [--loudnorm-args I=-16:TP=-1.5:LRA=11] [--dual-pass] [--streams 0] [--quiet] [--progress] [--max-packets N] [--max-memory-mib N] [--max-rss-mib N]"
        );
        return Ok(());
    }
    let source = Path::new(
        args.get(1)
            .ok_or("loudnorm requires INPUT.wav OUTPUT.wav")?,
    );
    let destination = Path::new(args.get(2).ok_or("loudnorm requires OUTPUT.wav")?);
    let mut options = fvid_media::CopyOptions::default();
    let mut normalization = None;
    let mut quiet = false;
    let mut progress = false;
    let mut dual_pass = false;
    let mut seen = std::collections::HashSet::new();
    let mut flags = args[3..].iter();
    while let Some(flag) = flags.next() {
        if !seen.insert(flag.as_str()) {
            return Err(format!("duplicate loudnorm option: {flag}").into());
        }
        match flag.as_str() {
            "--loudnorm-args" => normalization = Some(value(&mut flags, flag)?.as_str()),
            "--streams" => {
                options.streams = value(&mut flags, flag)?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()?
            }
            "--max-packets" => options.max_packets = Some(value(&mut flags, flag)?.parse()?),
            "--max-memory-mib" => {
                options.max_controlled_bytes =
                    Some(fvid_media::parse_max_memory_mib(value(&mut flags, flag)?)?)
            }
            "--max-rss-mib" => {
                options.max_rss_bytes =
                    Some(fvid_media::parse_max_rss_mib(value(&mut flags, flag)?)?)
            }
            "--quiet" => quiet = true,
            "--progress" => progress = true,
            "--dual-pass" => {
                dual_pass = true;
            }
            _ => return Err(format!("unsupported loudnorm option: {flag}").into()),
        }
    }
    if progress {
        options.progress = Some(fvid_media::ProgressHook::new(|event| {
            eprintln!(
                "{}",
                serde_json::json!({"packets":event.packets,"payload_bytes":event.payload_bytes,"done":event.done})
            )
        }));
    }
    let stats = match fvid::native_loudnorm::try_apply(
        source,
        destination,
        normalization,
        dual_pass,
        &options,
    )? {
        Some(stats) => stats,
        None => {
            #[cfg(feature = "media")]
            {
                if dual_pass {
                    fvid_media::apply_loudnorm_dual(source, destination, normalization, &options)?
                } else {
                    fvid_media::apply_loudnorm(source, destination, normalization, &options)?
                }
            }
            #[cfg(not(feature = "media"))]
            return Err("loudnorm input or compressed-audio policy is not yet supported by the owned bridge".into());
        }
    };
    if !quiet {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    }
    Ok(())
}

fn value<'a>(flags: &mut std::slice::Iter<'a, String>, flag: &str) -> Result<&'a String, String> {
    flags
        .next()
        .ok_or_else(|| format!("missing value for {flag}"))
}
