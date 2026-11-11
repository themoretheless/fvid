use std::time::Duration;

pub(super) type CliError = Box<dyn std::error::Error>;
pub(super) type CliResult<T = ()> = Result<T, CliError>;

pub(super) type DecodeRequest<'a> = (&'a str, bool, Option<(Duration, Duration)>);

pub(super) fn decode_time(text: &str) -> CliResult<Duration> {
    let micros = fvid::media_info::parse_time(text)?;
    Ok(Duration::from_micros(micros as u64))
}

#[cfg(feature = "media")]
pub(super) fn emit_json(quiet: bool, json: String) {
    if !quiet {
        println!("{json}");
    }
}

pub(super) fn make_progress_hook(enabled: bool) -> Option<fvid::media_control::ProgressHook> {
    enabled.then(|| {
        fvid::media_control::ProgressHook::new(|event| {
            eprintln!(
                "{{\"packets\":{},\"payload_bytes\":{},\"done\":{}}}",
                event.packets, event.payload_bytes, event.done
            );
        })
    })
}

pub(super) fn parse_native_audio_limit(
    option: &str,
    value: &str,
    options: &mut fvid::media::CopyOptions,
) -> CliResult {
    match option {
        "--max-packets" => options.max_packets = Some(value.parse()?),
        "--max-packet-bytes" => {
            let limit = value.parse()?;
            if limit == 0 {
                return Err("packet byte limit must be positive".into());
            }
            options.max_packet_bytes = limit;
        }
        "--max-memory-mib" => {
            options.max_controlled_bytes =
                Some(fvid_media::owned_budget::parse_max_memory_mib(value)?)
        }
        "--max-rss-mib" => options.max_rss_bytes = Some(fvid_media::parse_max_rss_mib(value)?),
        _ => return Err("unknown audio limit".into()),
    }
    Ok(())
}

// Unknown transformation options belong to the full parser, never ignore them.
pub(super) fn plain_decode(args: &[String]) -> CliResult<Option<DecodeRequest<'_>>> {
    if args.first().map(String::as_str) != Some("decode") {
        return Ok(None);
    }
    let (mut path, mut quiet, mut positional) = (None, false, false);
    let (mut from, mut to) = (None, None);
    let mut args = args[1..].iter();
    while let Some(arg) = args.next() {
        if !positional && arg == "--" {
            positional = true;
        } else if !positional && matches!(arg.as_str(), "--quiet" | "-q") {
            quiet = true;
        } else if !positional && (arg == "--from" || arg == "--to") {
            let value = args.next().ok_or("missing decode interval boundary")?;
            let time = decode_time(value)?;
            if arg == "--from" {
                from = Some(time);
            } else {
                to = Some(time);
            }
        } else if (!positional && arg.starts_with('-')) || path.replace(arg.as_str()).is_some() {
            return Ok(None);
        }
    }
    let interval = match (from, to) {
        (None, None) => None,
        (Some(a), Some(b)) if a < b => Some((a, b)),
        _ => return Err("decode interval requires both --from and --to with from < to".into()),
    };
    Ok(path.map(|path| (path, quiet, interval)))
}
