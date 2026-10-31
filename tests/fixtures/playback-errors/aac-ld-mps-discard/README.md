# Discarded split-TTT grouped parameters

756 own MP4/MKV programmes and84 unit-stride controls cover trees2/3/4,
480/512 LD/ELD cores, PCM32/64/128 and shared LD-SBR QMF32/64 ingress.
Empty low TTT regions carry CLD or CPC+ICC; the active high region uses the
same kind and has nonzero first-band history. Its coarse/fine domain alternates.
All programmes exercise strides2/5/23, PCM/Huffman1D/2D, paired/unpaired,
independent/dependent frames and fine/coarse parameter coding. Dependent
Huffman cases deliberately require a nonzero neighboring history to reconstruct
an in-range dummy index; that index belongs to no output band.

Original C getStrideMap/ecDataDec maps the one datum to [0,0). Entropy symbols
are read before numeric history reconstruction; CPC LSB reads have fixed count.
Empty mapping writes no history. Thus only syntax, cursor and quantization/data
mode flags are observable. Owned code validates codewords, grouped PCM values
and all suffix bits, then discards numeric indices for zero-width groups.
Nonempty parameter index bounds remain enforced.

936 standalone units have10,920 original-C comparisons across seeded histories
and previous quantization flags. The unmodified reference is compiled with
UBSan and aborts on any undefined behavior. Fine-CPC raw history -20 is excluded
from this numerical reference grid: some C dummy MSBs would be negative and
left-shifted. No modified C arithmetic is used as an oracle. These exclusions
restrict reference evidence,not native playback admission. The selected C grid
contains1,632 dummy values outside the current quantization bounds while bit
positions remain unchanged. This is component evidence,not independent
whole-codec PCM conformance.

84 late malformed movies insert unused fine CLD codeword31 or CPC codeword63
and recompute the envelope CRC. Tests require that specific PCM syntax error,
then retry the correct packet and verify full native state/output. They are
separate from positive playback,seek/rewind,checkpoint/reset and worker tests.

The original split-TTT refusal movie in `aac-ld-mps-empty-groups` is now an
acceptance case. Its payload and parent packet blob stay byte-identical;only
the filename and acceptance metadata change. No private source videos,frames,
audio or codec parameter sets are copied. All seeds and inputs are authored.

Generate explicitly,offline;ordinary tests never generate fixtures:

```sh
python3 scripts/generate_aac_ld_mps_discard_fixtures.py
```

Optional reference compilation needs the pinned official ISO source archive
and a compiler with UBSan. Neither is required by ordinary tests:

```sh
python3 scripts/validate_aac_ld_mps_discard_reference.py /path/to/saoc-reference.zip
cargo test --offline --no-default-features --features player --test aac_ld_mps_discard
```

This does not establish complete Annex B/profile conformance,physical output,
realtime performance,unequal HRTF grids or changed-frequency/layout support.
