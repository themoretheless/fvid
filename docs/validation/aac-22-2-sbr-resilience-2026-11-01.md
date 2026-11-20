# AAC22.2 combined LD-SBR and resilience qualification

Twelve authored four-AU video programmes exercise indexed configuration13,
480/512-sample ELD cores, RVLC flags2/3/6/7, HCR/section combinations,
LD-SBR double-rate/CRC, two additional TNS+flags7 programmes, and two
single-rate/no-CRC flags7 programmes. Both LFE histories remain independent.

The typed EP1 API receives reversed transmission order with explicit dependency
chains, exact bit bounds, nonzero ignored padding and differing equivalent
rational clocks (including negative initial PTS). Each AU has108 instances in54
groups, or132 instances in66 groups with TNS. Tests check invalid-input retry,
checkpoint replay and exact PCM equivalence with ordinary AAC transport.

A separate scalar ELD cosine/window/TNS oracle feeds independently implemented
CLDFB analysis, HF reconstruction and synthesis. All1,047,552 float scalars
match within1e-7 full scale; observed maximum absolute difference is
3.105913e-21. No gain/time/channel fitting is used. The new combination exposed
an oracle defect: the ELD generator filtered eight LFE bins despite max_sfb1
transmitting only four. It now bounds TNS by the authored band endpoint.
No production decoder change was needed for these combinations.

EP1 MP4 samples use category order; ordinary control samples use flat syntax
order. These differ with TNS. Exported PCM matches exactly; live reader playback,
rewind and sub-frame/frame/EOF/backward seeks match the uninterrupted suffix.
This qualifies single-track MP4 and typed grouped EP1, not multi-track transport
or Matroska EP1.

Seven tests in `aac_22_2`, `aac_22_2_esc` and
`aac_22_2_sbr_resilience` pass, zero failures/ignored. Four actual macOS
`audio_probe --play-device` runs cover both core lengths with TNS/double-rate
and resilience/single-rate. All16 full/two-seek/rewind phases drain completely.
Device output is stereo routing, not physical24-speaker or subjective quality
qualification. Raw device evidence is in the adjacent device JSON.

The39-file corpus regenerates identically with subprocess and socket creation
blocked. Ordinary tests only read saved authored media/reference PCM and require
neither generation nor FFmpeg/network.

```sh
python3 scripts/generate_aac_22_2_sbr_resilience_fixtures.py
cargo --config profile.dev.package.fvid-media.opt-level=2 test --offline \
  --no-default-features --features player --test aac_22_2 \
  --test aac_22_2_esc --test aac_22_2_sbr_resilience
```

This closes the combined resilience/LD-SBR22.2 qualification listed as open in
the preceding report. It does not close unequal-grid LD-MPS HRTF conversion,
other-tree EBQ, inherited cross-AU replacement, every codec/profile combination,
or the user's full codec-gap objective. Full ISO23003-2 AnnexB/TableA21 has not
been recovered; failed downloads/HTML responses are not normative evidence.
