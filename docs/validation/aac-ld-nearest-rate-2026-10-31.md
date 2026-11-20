# AAC-LD nearest-defined frequency table acceptance

The public `er_ad_ldmps_16khz.mp4` fails before native decoding with
`unsupported AAC LD band geometry`. An audit of all50 public LD-MPS compressed
MP4 inputs finds exactly this admission failure. The issue is not an unsupported
MPS tool: the core parser incorrectly restricts sampling rates to the nominal
frequencies with explicitly defined LD band tables.

## Authoritative basis

ISO/IEC14496-3:2009 section4.5.1.1,Table4.82 maps explicit sampling rates to
nominal frequency groups and requires the nearest defined table when a nominal
entry is missing. The LD480/512 band geometries are defined only for the
22.05/24,32,44.1/48-kHz groups. Therefore the outer tables also apply to the
remaining AAC frequency groups. The respective LD TNS limits follow the same
mapping. No new table coefficients or inferred spectral values are needed.

Local full AAC standard SHA256:
`913b72c340ba90bc9f4fc3bbc2edad8005369076c78a51f147bbbbbe6b5cf20d`.
Source: <https://ossrs.io/lts/zh-cn/assets/files/ISO_IEC_14496-3-AAC-2009-90cda427e845cc176293bffee3f92f99.pdf>.
This is the AAC standard, not the still-unretrieved full ISO/IEC23003-2 Annex B.
Original independently pinned FDK source (`ee76460efbdb147e26d804c798949c23f174460b`)
corroborates this mapping in `libAACdec/src/aac_rom.cpp`, its13-frequency
480/512 scale-factor-band rows and `tns_max_bands_{480,512}`.

## Authored reproduction and playback

The new `aac-ld-nearest-rate` corpus contains360 own eight-AU programmes in
MP4 and Matroska (720 movies),480/512 core lengths,ten indexed/explicit low/high
rates,ten configured layouts and all eight spatial trees. Nonzero first/final
band coefficients,window changes and LTP updates are retained from own nominal
24/48-kHz programmes. Regeneration asserts every source core packet equals its
frozen parent. PCM uses the existing independent sparse-cosine core reference;
spatial output compares explicit PCM composition of that reference.

Before the fix,40 nominal controls pass and both new core/spatial tests fail
with the exact band-geometry error. After correction all360 programmes match
expected PCM; both containers produce identical output. The actual audio worker
also verifies all720 movies: original PCM,live-history seeks,forward/backwards
and repeated seeks,either side of core/spatial boundaries,EOF,rewind and packet
timestamps. This includes four-source-AU spatial groups at128 bins/480 samples.
Generation is deterministic across two runs with sockets and child processes
disabled. Ordinary tests never invoke the generator,FFmpeg or network access.

## Checks

Use `CARGO_TARGET_DIR=../target` from this worktree. Commands:

```sh
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline -p fvid-media --lib
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --test aac_ld_nearest_rate --test aac_ld_layout --test aac_ld_mps_ld_core --test aac_alignment
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline --no-default-features --features player --lib nearest_ld_frequency_tables_keep_worker_audio_and_navigation
```

601 pass,0 fail,1 pre-existing ignored. The initial full media run exposed an
old16-kHz refusal expectation; it now checks successful AAC-LD configuration.
Existing invalid zero rate and unsupported core-length guards remain tested.

After rebuilding `aac_spatial_reference`,all50 original ISO inputs decode fully
in Complex mode. The formerly failing16-kHz input produces912 AUs and466944
stereo sample frames. Original checksum inventory and per-file MD5/SHA256,
before/after diagnostics and output sizes are saved in
`aac-ld-nearest-rate-iso-2026-10-31.json`. No ISO media/ASC/AUs enter this repo.

This qualifies frequency-table admission and authored playback/navigation. It
does not establish waveform conformance for all50 inputs,device output or full
LD-SBR/LD-MPS coverage. Phase/timing reference coupling,nominal-rate SSC remap,
unequal HRTF grids and other open boundaries remain in AUDIO_STATUS.md.
