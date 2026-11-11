use super::common::CliResult;

pub(super) fn try_capabilities(args: &[String]) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("capabilities") {
        return Ok(false);
    }
    if args.len() != 1 {
        return Err("capabilities accepts no arguments".into());
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&fvid::native_capabilities::inventory())?
    );
    Ok(true)
}

pub(super) fn try_probe(args: &[String]) -> CliResult<bool> {
    if args.first().map(String::as_str) != Some("probe") {
        return Ok(false);
    }
    let mut paths = Vec::new();
    let mut hint: Option<&str> = None;
    let mut quiet = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--input-format" => {
                if hint.is_some() {
                    return Err("usage: fvid media probe INPUT [--input-format FORMAT]".into());
                }
                let Some(value) = args.get(i + 1) else {
                    return Err("usage: fvid media probe INPUT [--input-format FORMAT]".into());
                };
                hint = Some(value.as_str());
                i += 2;
            }
            "--quiet" | "-q" => {
                quiet = true;
                i += 1;
            }
            arg if arg.starts_with('-') => {
                return Err("usage: fvid media probe INPUT [--input-format FORMAT]".into());
            }
            _ => {
                paths.push(args[i].as_str());
                i += 1;
            }
        }
    }
    if paths.len() != 1 {
        return Err("usage: fvid media probe INPUT [--input-format FORMAT]".into());
    }
    if let Some(info) = fvid::native_probe::try_probe_as(std::path::Path::new(paths[0]), hint)? {
        if !quiet {
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        return Ok(true);
    }
    #[cfg(not(feature = "media"))]
    return Err("native probe does not yet support this container".into());
    #[cfg(feature = "media")]
    Ok(false)
}
