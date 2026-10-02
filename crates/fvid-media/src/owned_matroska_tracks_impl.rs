/// Per-track container options. Rotation is clockwise in display space.
#[derive(Clone, Copy, Debug, Default)]
pub struct TrackOptions {
    /// Nominal frame/block duration in nanoseconds; zero omits the hint.
    /// Individual BlockDuration values remain authoritative for variable timing.
    pub default_duration_ns: u64,
    pub video: Option<VideoMetadata>,
    /// Rectangular rotations supported by the native player: 0, 90, 180, 270.
    pub rotation: u16,
    /// Priming to discard, in nanoseconds, subtracted from block timestamps.
    pub codec_delay_ns: u64,
}

fn track_entry(
    spec: &TrackSpec<'_>,
    number: u64,
    options: Option<&TrackOptions>,
) -> Result<Vec<u8>> {
    let metadata = options.and_then(|o| o.video.as_ref());
    let rotation = options.map_or(0, |o| o.rotation);
    let delay = options.map_or(0, |o| o.codec_delay_ns);
    if delay > i64::MAX as u64 {
        return Err(invalid("Matroska codec delay overflow"));
    }
    let (id, config, kind, geometry) = match spec.encoding {
        Encoding::Ass { configuration } => {
            if metadata.is_some() || rotation != 0 || delay != 0 {
                return Err(invalid("video metadata or codec delay supplied for ASS track"));
            }
            let header = std::str::from_utf8(configuration).map_err(|_| invalid("ASS header is not UTF-8"))?;
            if !header.contains("[Script Info]") || !header.contains("[V4+ Styles]")
                || !header.contains("ScriptType: v4.00+") || header.contains('\0') {
                return Err(invalid("invalid ASS codec header"));
            }
            ("S_TEXT/ASS", configuration, 17, Vec::new())
        }
        Encoding::Ffv1V1 { width, height } => (
            "V_FFV1", &[][..], 1, video(width, height, metadata, rotation)?,
        ),
        Encoding::Avc {
            configuration,
            width,
            height,
        } => {
            AvcConfig::parse(configuration)?;
            (
                "V_MPEG4/ISO/AVC",
                configuration,
                1,
                video(width, height, metadata, rotation)?,
            )
        }
        Encoding::Hevc {
            configuration,
            width,
            height,
        } => {
            HevcConfig::parse(configuration)?;
            (
                "V_MPEGH/ISO/HEVC",
                configuration,
                1,
                video(width, height, metadata, rotation)?,
            )
        }
        Encoding::PcmFloat32 { sample_rate, channels } => {
            if sample_rate == 0 || !(1..=64).contains(&channels) {
                return Err(invalid("invalid Matroska PCM geometry"));
            }
            if metadata.is_some() || rotation != 0 {
                return Err(invalid("video metadata supplied for PCM track"));
            }
            ("A_PCM/FLOAT/IEEE", &[][..], 2, element(0xe1, &[
                element(0xb5, &f64::from(sample_rate).to_be_bytes())?,
                uint(0x9f, u64::from(channels))?, uint(0x6264, 32)?,
            ].concat())?)
        },
        Encoding::Opus { configuration } => {
            if metadata.is_some() || rotation!=0 {return Err(invalid("video metadata supplied for Opus track"));}
            let channels=opus_packet::header_channels(configuration)?;
            if delay!=opus_packet::pre_skip_ns(configuration)? {return Err(invalid("Opus codec delay differs from pre-skip"));}
            let sample_rate=u32::from_le_bytes(configuration[12..16].try_into().unwrap());
            if sample_rate==0 {return Err(invalid("owned Matroska Opus output requires a nonzero input sample rate"));}
            ("A_OPUS",configuration,2,element(0xe1,&[
                element(0xb5,&f64::from(sample_rate).to_be_bytes())?,uint(0x9f,u64::from(channels))?
            ].concat())?)
        },
        Encoding::Aac {
            configuration,
            sample_rate,
            channels,
        } => {
            if metadata.is_some() || rotation != 0 {
                return Err(invalid("video metadata supplied for AAC track"));
            }
            let config = AacConfig::parse(configuration)?;
            if sample_rate == 0
                || channels == 0
                || config.sample_rate != sample_rate
                || u16::from(config.channels) != channels
            {
                return Err(invalid("AAC Matroska geometry differs from configuration"));
            }
            (
                "A_AAC",
                configuration,
                2,
                element(
                    0xe1,
                    &[
                        element(0xb5, &f64::from(sample_rate).to_be_bytes())?,
                        uint(0x9f, u64::from(channels))?,
                    ]
                    .concat(),
                )?,
            )
        }
    };
    let mut data = [
        uint(0xd7, number)?,
        uint(0x73c5, number)?,
        uint(0x83, kind)?,
        uint(0x9c, 0)?,
        element(0x86, id.as_bytes())?,
        if config.is_empty() { Vec::new() } else { element(0x63a2, config)? },
        geometry,
    ]
    .concat();
    if delay != 0 {
        data.extend(uint(0x56aa, delay)?);
    }
    if let Some(duration)=options.map(|o|o.default_duration_ns).filter(|&n|n!=0) {
        data.extend(uint(0x23e383,duration)?);
    }
    if matches!(spec.encoding,Encoding::Opus{..}) {
        if delay==0 {data.extend(uint(0x56aa,0)?);}
        data.extend(uint(0x56bb,80_000_000)?);
    }
    if !spec.name.is_empty() {
        data.extend(element(0x536e, spec.name.as_bytes())?);
    }
    // Write an explicit undetermined language instead of Matroska's default eng.
    data.extend(element(
        0x22b59c,
        if spec.language.is_empty() {
            b"und"
        } else {
            spec.language.as_bytes()
        },
    )?);
    element(0xae, &data)
}
