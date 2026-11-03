# Classic QuickTime v0 PCM aliases — qualification

Base:290287eb978d7e393374195af86953df8b7d2c8c.

Apple documents historical v0 sound descriptions using NONE or zero instead of the normal raw/twos tags. The classic form assumes uncompressed8/16-bit mono/stereo and compressionID0:
https://developer.apple.com/documentation/quicktime-file-format/sound_sample_description_version_0

Original fvid drops these audio tracks as unknown while the same headers/payloads using raw/twos play correctly. Initial before/wide-before logs record that refusal. The exact final six-test stand on original runtime is preserved in final-baseline.log/json with source/test hashes and guarded restoration.

Normalize only the classic v0, compressionID0, packetSize0,8/16-bit mono/stereo form. Eight-bit offset-binary maps to raw and16-bit signed big-endian to twos. Payload bytes, clocks, indexes and decoded data are unchanged. Zero channel count/rate are specific Invalid failures; common data-reference validation still precedes normalization. Other versions, compression/packet meanings, widths and wider layouts remain skippable unsupported descriptions. They must not inherit a PCM decoder merely because the FourCC is historical.

The pure separate generator authors102 short files:24 qualified aliases and12 native controls, analytic8/16-bit PCM, exact neutral-colored VP9 frames, six malformed field controls, nonclassic/fallback controls, and wider diagnostic inputs. Qualified cases span mono/stereo and8000/1000/1001 media clocks with8kHz audio. They verify exact PCM, metadata, duration, precise seek/rewind/EOF/MAX,72 actual software-worker sessions per stand, full/interval owned f32/f64 exports and public native inspection. Unsupported descriptions keep the picture and a later valid PCM track; invalid references/channels/rates refuse for their specific field. Successful playback and refusal controls are separate tests.

A wider24/32-bit experiment is not accepted: independent decoding does not match the native PCM or cannot identify zero-FourCC audio. ffmpeg-reference.json retains all48 observations, including those failures; diagnostics in the manifest label these24 aliases unsupported, and tests assert refusal. qualified-reference.json contains only the24 exact8/16 matches. No wider-format capability is counted or silently decoded with a guessed sign/width.

Newly admitted Track/SampleDescription codecs normalize to existing raw/twos identifiers; original aliases remain inspectable in source stsd bytes. Previously the tracks were omitted, so no existing supported interface/enum changes or migration are required. No external encoder, network or subprocess is used in generation or ordinary tests; FFmpeg is a separate optional reference. No performance, physical-device, hardware-video, arbitrary NONE-format or newer-version claim. Accepted as FVID-0538 / batch549:171 passing Rust executions, two existing device ignores, core/player checks and three external-tool-free repeats. All102 generated files are byte-identical and5128 frozen source/input hashes remain stable.
