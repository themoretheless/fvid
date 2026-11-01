# LD-MPS alignment is a profile boundary

ISO/IEC14496-26:2010/Amd3:2014,7.26.1.2.1.9 (LDSacDataFrame),requires
`ldsacTimeAlign` to be zero. Its flag has no additional restriction,so a present
flag with a zero value is valid. This differs from ordinary MPEG Surround's
separate transport requirements and must not be inferred from those rules.

Primary authored normative source (public preview):
https://cdn.standards.iteh.ai/samples/62645/53b3f02e4abe4a959b9ddb1e24312b41/ISO-IEC-14496-26-2010-Amd-3-2014.pdf

SHA256 `8c5df9bd20d46fea1d74977894de60f26d9031038bc70cb505608e4ca622a868`.

The source's relevant table is available in this preview; the missing full
23003-2 AnnexB transformation rules are not needed to enforce this restriction.
Nonzero alignment therefore belongs to invalid-profile diagnostics,not the
list of unfinished LD-MPS decoder features. This finding does not close
ordinary MPEG Surround/HQ integration or any other valid codec gap.

## Change and regression

Native Session previously rejected nonzero values with `LD-MPS in-band time
alignment is not yet implemented`. It now reports `nonzero LD-MPS time alignment
is not permitted by the low-delay profile` via the invalid-profile path.
Both existing error constructors use the same BitstreamError wrapper; this is
a correction of policy/diagnostics,not a new typed-error ABI. Valid PCM,
timestamp handling and admission are unchanged.

The bounded `Config::read_in_band` syntax reader still returns its parsed field
for diagnostics. Semantic enforcement belongs to Session. Its existing
nonzero-field raw-reader tests must continue to pass.

Five authored three-AU movies share the same own ELD core,spatial parameters
and explicit alignment flags. The middle AU carries raw16-bit values
0,1,32767,32768 or65535. These cover both sign-bit patterns without relying on
their interpretation: every nonzero representation violates the zero rule.
No private/ISO media,frames,audio or codec parameter sets enter the fixtures.

The zero movie plays completely. The four negative movies require the specific
profile error. Direct native tests submit a bad middle packet,check retained
memory,then retry the correct packet and compare all PCM with an uninterrupted
decoder through EOF. These are profile-refusal regressions,not acceptance tests
for forbidden nonzero alignment.

Before the fix the new test fails specifically on the old unimplemented-feature
message,after successful zero-alignment playback. The generated seven files
are byte-identical across a second generation. Source/artifact hashes are in
the JSON report. No broad conformance or whole-codec completion is claimed.

## Reproduce

```sh
python3 scripts/generate_aac_ld_mps_alignment_profile_fixtures.py
cargo test --offline --no-default-features --features player \
  --test aac_ld_mps_alignment_profile --test aac_ld_mps_inband \
  --test aac_ld_mps_hrtf_grids
```

Generation is explicit and separate. Ordinary tests read committed artifacts;
they do not invoke FFmpeg or network services.
