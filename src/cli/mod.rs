//! Modular CLI command router for `fvid media ...` subcommands.

mod audio_ops;
mod common;
mod decode_audio;
mod decode_video;
mod edit;
mod inspect;
mod loudnorm;
mod remux;
mod subtitles;
#[cfg(test)]
mod tests;
mod transcode;

pub fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "player")]
    if args.first().map(String::as_str) == Some("play") {
        return fvid::player::run_media(args[1..].to_vec());
    }
    // Preserve the adapter's full-resolution negotiation for coupled subsampled input.
    #[cfg(feature = "media")]
    {
        let command = usize::from(args.first().map(String::as_str) == Some("plan"));
        if matches!(
            args.get(command).map(String::as_str),
            Some("decode" | "transcode" | "transcode-lossless")
        ) && let (Some(source), Some(value)) = (
            args.get(command + 1),
            args.windows(2)
                .find(|pair| pair[0] == "--deband")
                .map(|pair| pair[1].as_str()),
        ) && !fvid::native_media::supports_deband_source(std::path::Path::new(source), Some(value))?
        {
            return transcode::run_native(args);
        }
    }
    if subtitles::owned_subtitle_burn_command(args)? {
        return Ok(());
    }
    if edit::owned_xfade_command(args)? {
        return Ok(());
    }
    if transcode::owned_lossless_command(args)? {
        return Ok(());
    }
    if args.first().map(String::as_str) == Some("plan")
        && args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--encoder" | "--encoder-option"))
    {
        return Err("plan does not take metadata mutation or encoder selection".into());
    }
    if let Some(owned) = transcode::try_transcode_rewrite(args)? {
        return run(&owned);
    }
    if let Some(rewritten) = try_plan_shorthand_rewrite(args)? {
        return run(&rewritten);
    }
    if args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("loudnorm")
    {
        return loudnorm::run_plan(&args[1..]);
    }
    if args.first().map(String::as_str) == Some("loudnorm") {
        return loudnorm::run(args);
    }
    if inspect::try_capabilities(args)? {
        return Ok(());
    }

    let normalization_plan = args.first().map(String::as_str) == Some("plan")
        && args.get(1).map(String::as_str) == Some("normalize-loudness");
    let args = if normalization_plan { &args[1..] } else { args };

    if audio_ops::try_plan_loudness(args)? {
        return Ok(());
    }
    if audio_ops::try_normalize_loudness(args, normalization_plan)? {
        return Ok(());
    }
    if audio_ops::try_loudness(args, normalization_plan)? {
        return Ok(());
    }
    if subtitles::try_convert_subtitles(args)? {
        return Ok(());
    }
    if audio_ops::native_mix(args)? || audio_ops::native_merge(args)? {
        return Ok(());
    }
    if transcode::try_native_ffv1_lossless(args)? {
        return Ok(());
    }
    if remux::try_plan_remux(args)? {
        return Ok(());
    }
    if decode_audio::try_plan_decode_audio(args)? {
        return Ok(());
    }
    if inspect::try_probe(args)? {
        return Ok(());
    }
    if remux::try_remux(args)? {
        return Ok(());
    }
    if decode_audio::try_decode_audio(args)? {
        return Ok(());
    }
    if edit::try_owned_overlay(args)? {
        return Ok(());
    }
    if edit::try_video_trim(args)? {
        return Ok(());
    }
    if edit::try_mp4_matroska_concat(args)? {
        return Ok(());
    }
    if edit::try_video_concat(args)? {
        return Ok(());
    }
    if edit::try_native_audio_concat(args)? {
        return Ok(());
    }
    if edit::try_native_audio_trim(args)? {
        return Ok(());
    }
    if decode_video::try_export_y4m(args)? {
        return Ok(());
    }
    if decode_video::try_plain_video_decode(args)? {
        return Ok(());
    }
    if decode_video::try_owned_decode_command(args)? {
        return Ok(());
    }

    #[cfg(feature = "media")]
    {
        transcode::run_native(args)
    }
    #[cfg(not(feature = "media"))]
    {
        let _ = args;
        Err("this media operation requires cargo build --release --features media; plain media decode INPUT is available without that feature".into())
    }
}

fn try_plan_shorthand_rewrite(args: &[String]) -> common::CliResult<Option<Vec<String>>> {
    if !(args.first().map(String::as_str) == Some("plan")
        && !matches!(
            args.get(1).map(String::as_str),
            Some(
                "trim"
                    | "trim-pcm"
                    | "concat"
                    | "remux"
                    | "transcode-lossless"
                    | "crop-lossless"
                    | "transcode"
                    | "overlay"
                    | "xfade"
                    | "burn-subtitles"
                    | "convert-subtitles"
                    | "loudness"
                    | "normalize-loudness"
                    | "loudnorm"
                    | "mix-audio"
                    | "merge-audio"
                    | "decode-audio"
                    | "export-y4m"
                    | "decode"
            )
        ))
    {
        return Ok(None);
    }
    let mut positional = false;
    let mut path_count = 0usize;
    let mut has_transform_flag = false;
    let mut iter = args[1..].iter();
    while let Some(arg) = iter.next() {
        if !positional && arg == "--" {
            positional = true;
            continue;
        }
        if !positional && arg.starts_with('-') {
            if matches!(
                arg.as_str(),
                "--metadata"
                    | "--metadata-delete"
                    | "--stream-metadata"
                    | "--stream-metadata-delete"
                    | "--output-format"
                    | "--max-packets"
                    | "--max-packet-bytes"
                    | "--max-memory-mib"
                    | "--max-rss-mib"
                    | "--streams"
            ) {
                let _ = iter.next();
            } else if matches!(arg.as_str(), "--quiet" | "-q" | "--progress") {
            } else {
                has_transform_flag = true;
                if !matches!(arg.as_str(), "--hflip" | "--vflip" | "--seek") {
                    let _ = iter.next();
                }
            }
        } else {
            path_count += 1;
        }
    }
    if !(1..=2).contains(&path_count) {
        return Err(
            "plan requires INPUT, or: plan trim|trim-pcm|concat|remux|transcode-lossless|overlay|xfade|burn-subtitles|loudness|loudnorm|mix-audio|merge-audio|decode-audio|export-y4m ..."
                .into(),
        );
    }
    let mut rewritten = vec![
        "plan".to_owned(),
        if has_transform_flag {
            "transcode-lossless".to_owned()
        } else {
            "remux".to_owned()
        },
    ];
    rewritten.extend_from_slice(&args[1..]);
    Ok(Some(rewritten))
}
