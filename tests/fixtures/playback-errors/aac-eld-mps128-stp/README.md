#128-band AAC-ELD / LD-MPS STP regression corpus

Generate own videos explicitly with
`python3 scripts/generate_aac_eld_mps128_stp_fixtures.py`.
396 programmes each have MP4 and Matroska versions (792 acceptance movies).
They cover64/88.2/96kHz,480/512 core frames,all8 trees and both TTT variants,
23/9/4 parameter grids,repeated headers,fragmentation and delayed source windows.
The alternating flags exercise disabled slots with warmed history; additional
in-band STP -> ordinary -> STP switches test history reset/reconstruction.

18 core-only controls and24 late-corruption refusal movies are separate from
acceptance. PCM acceptance covers both owned/frontend MP4 and Matroska exports,
AudioStream playback,forward/backward/middle/EOF navigation,rewind,repeated
ranges,complete and incomplete source windows,and transactional retries.
AudioWorker capture additionally checks live-history seeks and source clocks.
A signal-effect control compares66 ordinary counterpart movies; merely ignoring
the active STP flags must fail.

The pre-fix native failure is `invalid LD-MPS STP geometry`,not a container or
spectral error. The acceptance tests are enabled with the geometry fix.

Freeze the independent component outputs separately with
`python3 scripts/generate_aac_ld_mps_stp128_reference.py /path/to/official.zip`.
This optional research step compiles an audited/adapted ISO STP extraction
under undefined-behavior/bounds sanitizers. It keeps the previous explicit
reference-only fixes for absent channel indices,zeroed low-band invalid BP
indexing and populated dry-group count. It does not prove whole-decoder
conformance. Eight tree sequences,104 slots each,include disabled flags,
silent/low-only/high-only stimuli and history refresh boundaries.
The1,384,448 stored scalars qualify complex full-bank STP including bins64..127;
ordinary tests require only these frozen bytes and owned Rust code.

All video,audio and spatial syntax is synthetic and authored locally. No private
media or copied codec parameter sets. No FFmpeg,network or external process is
used by ordinary tests or own video generation. Raw native-rate playback is
qualified; required profile-specific output adaptation above48kHz,physical
audio output and real-time performance remain separately unqualified.
