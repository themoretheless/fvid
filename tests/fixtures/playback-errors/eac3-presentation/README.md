# Expanded E-AC-3 programme assembly

27 owned cases: 7.1 extension, 7.1 front-channel replacement, 10-channel height
extension and a 16-channel semantic layout. Three-block conventional base 5.1
and dependent streams share their rate/block count. Dependent IDs are sequential
and only the last dependent has compre set. Cases exercise unchanged and changing
last-dependent dynrng, deliberately conflicting base dynrng, stationary/mixed
spectra, per-stream amplitude differences and final-clamp headroom. A neutral
7.1 control and 44.1 kHz rational-clock witness are included.

Regenerate separately: `python3 scripts/generate_eac3_presentation.py`.
No external codec/process/network or private source media is required. Each case
has raw programme bundles, raw components, independent scalar controls and
control videos. Programme videos store each syncframe in a separate Matroska
SimpleBlock at its programme timestamp, following the A_EAC3 mapping description.
An early quad prototype was excluded: output up to 5.1 follows the compatible
independent programme and is not counted as expanded-output acceptance.

The oracle reconstructs each independent control in native order, uses literal
coded-to-native permutations and explicitly authored speaker identities, replaces
or appends channels, applies the last dependent's gain once, then clamps. The
high-amplitude case distinguishes premature per-stream clipping from final mix
clipping. All present cases are scalar conventional, so their PCM comparisons
are exact; this does not qualify active AHT in expanded presentations.

Four integrations accept direct assembly/reset, normal decoder dispatch, signed
source-window preservation across split/queued inputs, incomplete-EOF refusal,
Matroska PCM/masks/PTS/duration and worker rewind/partial seek. Wave-mappable
layouts (7.1 and 10 channels) reach the backend capture with explicit masks.
Selected 48/44.1 kHz cases also seek 20 ms into the second packet. The 16-channel
video is demultiplexed and decoded through the semantic API; the audio reader
explicitly refuses its unsupported device mask rather than aliasing positions.
No physical device or GUI playback claim is made.

The 44.1 kHz case specifically reproduces zero-duration dependent packets
resetting the selected sample clock: old duration 51.414965 ms versus exact
2304/44100 = 52.244897 ms (floored ns). The old clock passes flat 48 kHz controls;
that passing observation is kept in `/tmp/fvid-presentation-split-clock-flat-control.log`.
The rational-rate red baseline is `/tmp/fvid-presentation-split-clock-before.log`.

Optional expanded FFmpeg audit stays unvalidated/red: all 21 wave-mappable
comparisons have a geometry mismatch on this individual-syncframe packetization.
FFmpeg 9.0.2 returns six-channel compatible PCM and warns that dependent frames
are ignored without an independent frame; FVid returns the requested 8/10
channels. This is not direct full-presentation agreement or a blanket FFmpeg
feature claim. `benches/check_eac3_presentation_reference.py` records all cases
without aborting; `/tmp/fvid-presentation-reference.json`. Earlier bundled-stage
results are kept separately and are not current qualification.

Raw elementary acceptance additionally covers all 21 WAVE-mappable cases:
8/10-channel packet geometry and exact PCM, primary-only clocks, rewind and
20 ms second-packet seeks through the worker. `Ac3AudioReader::open` selects the
native expanded programme; `open_compatible` explicitly retains the independent
six-channel mix. Six 16-role cases still require extended device metadata.
The owned `truncated-expanded-tail.ec3` and `.mkv` witnesses shorten only the
last dependent syncframe: complete programmes remain usable and EOF reports the
truncated extension. Four tests in `eac3_raw_presentation` cover these paths.

An isolated diagnostic FFmpeg 9.0.2 build that relocates base channels into the
union speaker layout matches the neutral 48/44.1 kHz raw controls within
1.56e-8 maximum absolute error. Conflicting base dynrng still differs by
0.0257542. This patched build is a diagnostic, not official-reference agreement;
`/tmp/fvid-presentation-relocated-reference.json` records its source revision,
source hashes, build options and results. Production code uses no FFmpeg.

Remaining: device metadata for all extended SMPTE positions,
explicit programme selection, dynamic speaker maps/topology, expanded
AHT/SPX/coupling profiles, heavy compression, other rates and expanded MP4
acceptance. These fixtures do not prove complete E-AC-3 or all codec coverage.
