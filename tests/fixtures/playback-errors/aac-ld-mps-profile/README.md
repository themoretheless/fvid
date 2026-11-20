# Native LD-MPS profile boundary regressions

Generate explicitly with `python3 scripts/generate_aac_ld_mps_profile_fixtures.py`.
Own ordinary control,initial residual profile and late residual replacement
videos isolate native validation. Absent bands, zero bands and zero downmix
residual also have initial/late movies. A reserved type15 extension is a positive
skip/control case. The late case preserves three valid source
packets before changing the header. Direct native retry checks restore exact
spatial history;public export may retain only valid-prefix PCM.

Source/profile rationale is in `docs/LD_MPS_PROFILE.md`. These are forbidden
profile counterexamples,not unsupported codec acceptance. All media,core PCM
and parameters originate in own synthetic fixtures. No private source material.
Generation/tests require no FFmpeg,network or reference C compilation.
