# STP scalar energy/gain reference

These 20 synthetic subband programmes qualify only the STP scalar energy/gain
stage. They are not video files and do not claim STP playback acceptance. Existing
synthetic STP movie refusal/rollback coverage remains enabled.

`reference.json` holds the captured normalized input energies and output gains
from the ISO reference prefix, for trees 0..4, four signal patterns and 100 slots
per programme. This crosses the refresh boundaries at slots 33, 65 and 97. The
signals are generated arithmetic sequences with pulses, silence and changing
relative dry/wet energy. They contain no private media or codec parameters.

Explicit regeneration:

```sh
python3 scripts/generate_aac_ld_mps_stp_energy_reference.py /path/to/SAOC_IS7_ReferenceSoftware__20160603.zip
```

Generation compiles the pinned reference energy/gain prefix with bounds and
undefined-behavior sanitizers. Ordinary tests read the frozen JSON only. The
reference's final LD frequency-mixing loop is excluded because it contains
invalid indices; it is not silently repaired. Tree 7 is excluded because its
prefix reads uninitialized channel indices. Trees 5/6 read uninitialized surplus
dry-energy rows in this reference and are also excluded.

FVid's scalar tracker takes explicit, validated energy groups and owns its
history per instance. LD frequency weighting, output grouping and final mixing
still need independent qualification before native STP playback is enabled.
