# AAC-LD with attached LD-MPS

Generate separately with `python3 scripts/generate_aac_ld_mps_ld_core_fixtures.py`.
Ordinary tests read committed bytes; no FFmpeg, network, encoder or compiler is
used by generation. Every movie contains an own synthetic video track. AAC core
syntax and sparse-cosine reference PCM come from the own `aac-ld-layout` corpus;
spatial parameters are authored by this generator. No private media is copied.

318 acceptance movies cover AOT23, 480/512 samples, 24/48 kHz, all eight trees,
TTT variants, repeat headers, delayed/burst and fragmented messages, STP, GES,
23→9→4 parameter grids, escaped SSC lengths, absent SBR/PS discovery, interior
packet padding, and a 3 kHz container clock. The 480-sample/48 kHz cases combine
two source packets per spatial frame. Source spans retain each packet's own
identity, duration and padding through playback and export.

Two flag-off controls retain core-only output. Two malformed ASC movies reject
empty/oversized configurations. Four late-corruption movies reach source packet3
and exercise transactional retry (spatial trailing bytes/in-band rate changes).
These six rejection cases are not playback acceptance.

`tests/aac_ld_mps_ld_core.rs` checks native and public MP4 PCM, the actual reader
packet-ID protocol, forward/backward/EOF seeks, rewind/reset, repeated ranges,
source clocks and rollback. The AudioWorker test in
`src/playback/audio/ep1_worker_tests.rs` additionally captures real worker output,
including exact packet timestamps after live-history seeks and rewind.
Expected PCM combines a frozen analytical AAC-LD
core oracle with the separately qualified spatial session; it is not an
independent full LD-MPS decoder. The verified maximum error is 7.258e-12 over
8,300,064 samples (test limit 1e-9).

Before the change, the plain acceptance input stopped at ASC parsing with
`unsupported AAC trailing extension`. Admitting ASC alone exposed a second
failure: `delayed MP4 source lookahead exceeded`, because a spatial response
covered two core packets. Both failures are covered by playback acceptance.

This corpus qualifies EP0 AAC-LD without SBR at the listed rates/geometries.
It does not qualify all protection modes, residual domains, arbitrary profiles,
independent full-decoder conformance, device audio output or real-time speed.
