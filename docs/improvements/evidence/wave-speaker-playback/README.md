# FVID-0523 and FVID-0524 — speaker metadata and native Duration navigation

These are two independently demonstrated changes. FVID-0523 retains a validated WAVE_EXTENSIBLE speaker mask for the existing player stereo device router. FVID-0524 adds unquantized native Duration seeking across the three bare window readers, WAVE, AIFF and AU. They share a fixture stand but neither fixes the other.

## Baseline and independence

Original production source is commit `6920f9838479089b3e8cb1c6a740d8ff4a36c9a0`. Final `all-original` source hashes match that commit for all three adapters. The identical final six-test suite has these outcomes:

| Production variant | Passed | Failed | Specific result |
| --- | ---: | ---: | --- |
| Original | 1 | 5 | Native PCM/video/refusal control passes; both new behaviors absent |
| Mask only | 2 | 4 | Exact speaker coefficients pass; precise native and worker seeks still fail |
| Precise Duration only | 4 | 2 | Native/coded precise seeks pass; speaker mask and stereo-only worker startup still fail |
| Both changes | 6 | 0 | Every acceptance test passes |

`independence-proof.json`, each variant's source hashes, result JSON and complete logs preserve this comparison. Source restoration was guarded against concurrent edits. Initial `before*`, `candidate.log`, `mask-only.log`, and four-test `final-before`/`precise-only` observations are retained as intermediate evidence. `first-wave-qualification/` is the earlier four-test WAVE stand, not a second set of accepted improvements or extra final test executions.

## Media and checks

The standalone generator `scripts/generate_wave_speaker_playback.py` uses no encoder or network. It writes exact analytic PCM16/float32 interleaved probes, handcrafted RIFF/AIFF/AU headers and an authored three-frame16x16 VP9 companion with exact picture-plane oracles. Ordinary tests only read checked-in files. Sixteen WAVE cases include14 extensible masks and two classic controls; three additional AIFF/AU PCM controls share an independent exact float oracle. The malformed WAVE speaker-count declaration is tested as a specific refusal, separately from successful playback.

The [Microsoft WAVEFORMATEXTENSIBLE reference](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ksmedia/ns-ksmedia-waveformatextensible) defines increasing mask-bit channel order, permits unnamed excess channels and reserves positions beyond the defined set. Device routing retains its existing supported-position boundary; reserved-bit speaker layouts have no successful-playback qualification here.

Defined WAVE roles, rear-only stereo, LFE-only mono, explicit unassigned3/9 channels and partial assignment follow the existing router's policies. No new channel weights, PCM reordering, parser admission change or guessed classic-WAVE mask is introduced. Routing budgets and partial decoded-frame refusal are checked independently.

Twelve existing authored predictive sources in `coded-seek-history` cover MACE3/6 mono/stereo, GSM, TrueSpeech, G722, Creative and all four G726 rates. Their unchanged full native output is the control for exact cold/warm seek suffixes and restored decoder history; no new decoder correctness claim is made. New fixture generation stays separate from tests and never copies private media.

The final suite covers native rational clocks,1ns/fractional/window boundaries,raw-coordinate follow-up,rewind,EOF/MAX and143 actual AudioPlayback worker sessions:80 WAVE stereo-sink sessions,15 AIFF/AU native PCM sessions,and48 predictive sessions. Native PCM and all three VP9 picture planes are checked exactly,not by metadata alone. CPAL callback/ring and owned-router suites also pass; no OS audio device is opened by the new regression. Physical device output remains unqualified and two existing hardware smoke tests remain ignored.

Nine final gates pass178 Rust executions,core/player builds,three six-test repeats and six binary executions with nonexistent PATH/FFMPEG/FFPROBE. The final source is frozen across2948 paths. All42 new fixture files regenerate byte-identically; final commands,logs and hashes are preserved beside this README. Earlier intermediate runs are excluded from these counts. No performance claim is made.

## Duration behavior and migration

Before FVID-0524 these readers inherited the documented default tick-quantized `AudioStream::seek_time`. Arbitrary times rounded down, except recovery of floored public sample boundaries. The new override deliberately selects the first native sample at or after the unquantized Duration request, matching precise native navigation elsewhere. At44.1kHz,1ns now selects sample1 rather than sample0. Requests at/beyond the extent clamp to EOF before narrowing. Predictive decoders still restore their complete preceding history and trim presentation to the requested native point.

This is an explicit navigation capability upgrade, not a claim that the previous documented rounding policy was an invalid parser. `seek_to(i64)` retains its existing sample-coordinate behavior. Callers requiring coordinate navigation should call that method with their explicitly chosen integer sample coordinate. Public signatures, Pcm construction, payload bytes, speaker policies, raw seek and rewind behavior remain unchanged.
