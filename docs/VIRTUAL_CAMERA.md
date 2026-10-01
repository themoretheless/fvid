# FVid virtual camera

Required behavior: select a video file and expose its decoded frames as a
camera input in other applications. File decoding remains in FVid, without
FFmpeg. Physical camera capture is outside this request.

## Current verification (2026-10-01)

The Rust source, C boundary, Swift playback session, CMIO source/sink and host
bundle are implemented. The camera bridge regression runner currently passes
8 Swift suites and 9 AVC/HEVC/VP9/AV1 pixel comparisons, including seek, EOF,
repeat, crop/aspect conversion, timestamps and bounded queues/pools.
The runner rejects `fvid-media`/FFmpeg packages in the FFI dependency graph and
FFmpeg dynamic libraries in every generated Swift test executable.

`/Applications/FVid Camera.app` is signed with a valid Apple Development
identity, TeamIdentifier `U6J9E8HK4D`; strict deep codesign verification passes.
Its host executable has no FFmpeg linkage. `systemextensionsctl list` contains
no FVid extension. Launch through `open` still fails with
`RBSRequestErrorDomain Code=5` and underlying POSIX code `163` (job spawn
failure). No provisioning profiles were found in the standard user profile
directories. This evidence does not establish the cause of the launch failure.
Installed activation, camera visibility and frame delivery in a separate
application remain unverified. The root `media` feature still links the legacy
FFmpeg adapter; camera-only independence does not satisfy that broader goal.

The sections below record implementation history. Their earlier statements
about missing components, developer identities and test counts describe those
stages, rather than the current state above.

## macOS implementation boundary

Use a Core Media I/O camera system extension, embedded in a host application.
The extension publishes a source stream; the FVid producer delivers decoded
frames to the extension. Keep codec/container logic in Rust and the platform
registration/sample-buffer bridge in the macOS target. Do not use a system
video decoder to replace the requested FVid-owned codecs.

The producer must pace presentation timestamps against a monotonic clock,
use bounded buffering, and release old frames when a consumer falls behind.
The camera endpoint must keep a stable advertised format during playback;
resolution changes require conversion to that format. Pause/end/disconnection
must have explicit frame-retention behavior, and loop/seek must rebase the
playback timeline without reversing camera timestamps.

## Acceptance evidence still required

- Build the extension and host application with the required signing and
  entitlements; install using the system-extension activation flow.
- Confirm the named FVid camera appears in another application's camera list.
- Play a known video through it and verify moving frames, dimensions, cadence,
  pause/resume, loop, producer exit and consumer restart.
- Verify the camera producer's dependency/link graph excludes FFmpeg.
- Bound frame memory and queue depth under a slow or disconnected consumer.

Current status: `virtual_camera::CameraClock` implements rational frame-rate
pacing, skipping missed slots, pause/resume, seek and looping independently
of the monotonic camera timeline. Tests cover 30000/1001 cadence and playback
state changes; the full no-default-features suite passes 106 tests. The clock
is not yet connected to a producer. No virtual-camera endpoint is implemented
or installed. The local selected developer directory is CommandLineTools;
Swift is available. Signing and full app/extension build capability have
not yet been checked.

Official platform reference:
https://developer.apple.com/documentation/coremediaio/creating-a-camera-extension-with-core-media-i-o

`LatestFrame` now provides a mutex-protected, single-frame BGRA buffer with a
constructor memory budget. RGB publication replaces stale frames without
allocations, rejects non-increasing camera timestamps, and copies snapshots
into consumer-owned storage. Closing the producer retains the last frame.
A 1,000-publication test verifies replacement, channel order, timestamp
validation, close behavior and allocation bounds. Full suite: 107 tests pass.
This is an in-process buffer, not yet a cross-process camera transport.

`Y4mCameraSource` connects the native file reader to camera ticks and the latest
BGRA buffer. File-frame selection uses the original rational frame rate;
backward seeks rewind/replay and EOF holds the last decoded frame. The source
requires output dimensions to match (resizing remains pending). A pixel-level
test covers black/white frames, EOF and backward seek. Full suite: 108 tests.
This path is still synchronous and in-process, without system-camera exposure.

## macOS bridge implementation

`platform/macos/CameraExtension/CameraProvider.swift` defines the CMIO provider,
device and source stream, validates BGRA submissions, copies rows respecting
CVPixelBuffer stride, wraps frames in timed CMSampleBuffers and sends them to
active camera consumers. `main.swift` starts the service with a fixed initial
1280x720/60 format. The source compiles and links with warnings-as-errors using
the installed macOS SDK; otool reports no FFmpeg libraries for this executable.

This is not yet an installable extension bundle. Host app, signing/entitlements,
frame ingress, bounded CVPixelBuffer pooling and activation remain pending.
The executable has not been run as an installed system extension, and camera
visibility/frame delivery to another application has not been verified.

The macOS sender now allocates through `PixelPool`, with a threshold of three
CVPixelBuffers. Exhaustion drops the incoming frame without an unbounded
allocation fallback. A native test holds all three buffers, checks 100 failed
acquisitions and confirms allocation resumes after one release. The extension
build passes warnings-as-errors; the pool runtime test passes outside the
sandbox (IOSurface allocation failed with -6662 inside the sandbox). This
checks local pool bounds, not consumer delivery or downstream OS allocations.

The macOS host now implements explicit user-triggered activation using
OSSystemExtensionManager, embedded-extension presence checks, pending-request
handling, approval/reboot status and activation errors. It compiles with
warnings-as-errors; its Info.plist and system-extension-install entitlement
pass plutil validation. The activation request has not been submitted. Bundle
assembly, extension metadata, signing and frame ingress remain pending.

## Bundle build

Run `python3 scripts/build_macos_camera.py --output /tmp/FVidCamera.app`
with a new output path. The script compiles both executables with warnings as
errors, embeds the extension under Contents/Library/SystemExtensions, generates
extension metadata and validates both plists. Existing output is never deleted.

Unsigned output is for compilation and package inspection only; it uses a
LOCAL Mach-service prefix and cannot establish installability. To request
signing, explicitly supply `--team-id TEAMID --sign IDENTITY`. The script signs
inside-out and runs codesign verification, but does not install, notarize or
claim activation success. The signing path and entitlement sufficiency have
not been validated against an actual developer identity/provisioning setup.

When supplying `--host-profile` and `--extension-profile`, both profiles must
permit the team, concrete bundle identifier, signing certificate and local Mac.
The build resolves an exact valid keychain identity to its SHA-1 fingerprint
before creating the output bundle. Ambiguous identity names are rejected.
`DeveloperCertificates` must include that certificate. Profiles restricted to
registered devices must include this Mac's Provisioning UDID (Hardware UUID is
only the fallback on Macs without a separate provisioning identifier).
`ProvisionsAllDevices` bypasses only the device check, never the certificate check.
These checks have unit coverage and identity resolution was checked against the
local keychain. A real profile-backed launch and camera consumer delivery remain
unverified. See [Apple's device registration guidance](https://developer.apple.com/help/account/devices/register-a-single-device)
and [provisioning profile anatomy](https://developer.apple.com/documentation/technotes/tn3125-inside-code-signing-provisioning-profiles).

## Native compressed file source

`NativeCameraSource` now accepts `NativeReader`, including the supported
MP4/AVC I/P subset. It chooses frames using exact source-clock intervals,
replays on backward seek, and holds the final frame at EOF. The former Y4M
adapter delegates to this same implementation. Output remains the bounded
`LatestFrame` BGRA buffer; no FFmpeg is used by this path.

`benchmarks/validate_native_playback.py` exercises moving MP4 output at
0/40/400/0 ms, compares selected BGRA pixels exactly, and checks host timestamp
propagation. A Rust regression covers the fractional 30000/1001 boundary,
backward seek and EOF. All 133 no-default-feature tests pass. The bridge from
this buffer to the CMIO extension is still missing; these tests do not prove
that another application can select or consume an installed FVid camera.

## CMIO frame ingress

The extension now advertises a second, sink-direction stream (`FVid Input`)
with the same BGRA format as its source. It permits one producing client,
uses a three-buffer CMIO queue, and keeps one asynchronous consumption request
outstanding. Consumption runs on the provider serial queue at the advertised
frame rate. Stop/disconnect invalidates callbacks from the preceding session.
Incoming image geometry/format/readiness is validated; padded rows are copied
into packed BGRA and submitted through the existing bounded source pixel pool.
Output timestamps come from the CM host clock. Scheduled-output notifications
are sent only when a frame was submitted to source clients.

`CameraSinkTests.swift` verifies padded rows, geometry, pixel format and invalid
sample rejection. It passes locally. The complete host/extension bundle builds
with warnings-as-errors and valid plists (`/tmp/FVidCamera-sink-2.app`). It remains
unsigned and uninstalled. The host-side CMIO producer and Rust frame-source
connection still need implementation; no cross-process/installed-camera claim
is made by these compile and pixel-copy tests.

## Host CMIO producer

`CameraHost/CameraProducer.swift` implements discovery of the fixed FVid device
UID, selection of its output/sink stream, format and three-buffer queue checks,
start/stop, and bounded CMSampleBuffer enqueue. It checks BGRA dimensions and
strictly increasing numeric timestamps. A full queue drops the offered frame;
success transfers one retained sample reference to the consumer, while enqueue
failure releases that reference. Stop drains residual samples after successful
CMIODeviceStopStream. All calls require a single serial producer context.

The class is compiled into the host bundle, but UI playback and Rust FFI are
not wired yet. CameraProducerTests exercises a three-element CMSimpleQueue,
1,000 full-queue rejections, retained dequeue and queue reuse. The test and
host/extension warnings-as-errors build pass (`/tmp/FVidCamera-producer-3.app`).
Real CMIO consumption/stop ownership and device discovery still require an
installed, signed extension; standalone queue tests do not prove those paths.

## Rust/Swift source bridge

`crates/fvid-camera-ffi` exports open/size/frame/close through the checked-in
`FVidCamera.h`; unsafe pointer handling is confined to this boundary crate.
FVid itself retains its unsafe-code prohibition. The caller serializes handle
operations and owns writable output memory. Decoding errors poison the handle,
and panics during open/frame are contained at the ABI boundary. Closing requires
a live uniquely owned handle. The crate uses only fvid -> fvid-cpu, with default
features disabled; the host executable has no FFmpeg linkage.

`NativeVideoSource.swift` owns the handle and reusable BGRA storage. The app build
now compiles and statically links the Rust bridge. NativeVideoSourceTests passes
actual Swift/C/Rust calls for Y4M pixels, seek, EOF, stale timestamps, poisoned
handles and missing input. Its optional MP4/RGB arguments verified frames
0/1/10/0 of the moving MP4 against native RGB bytes. Bundle build and plist checks
pass (`/tmp/FVidCamera-ffi-1.app`). The host still needs the UI/timer/sample-buffer
pipeline connecting this source to CameraProducer, plus signed installation
and a real client-camera test.

## Connected host playback pipeline

The host now has Open video and Stop video controls. CameraSession runs file
opening, decoding, timestamp generation, frame creation and CMIO enqueue on a
serial playback queue. Its 60-fps timer uses the CM host-clock epoch and elapsed
media time; the source holds the final frame. Full CMIO queues skip the tick.
Stop cancels the timer, stops the stream and releases the Rust source.

FVid's allocation-free `fit_bgra` performs nearest-neighbour resizing with
centered opaque-black letterboxing into the camera dimensions. Swift calls it
through the C boundary and reuses its output Data storage. CameraFrameBuilder
uses the same threshold-limited three-buffer PixelPool as the extension.

The full host and extension compile without warnings (`/tmp/FVidCamera-session-1.app`).
All 134 Rust tests pass. CameraPipelineTests runs with actual IOSurface buffers
outside the sandbox and verifies Rust decode/letterbox -> Swift sample buffer
-> sink BGRA extraction, nanosecond timestamps, 100 pool-exhaustion drops, and
reuse after releasing a buffer. This exercises the in-process components, not
CMIO cross-process delivery. Signed installation, real device discovery/stream
lifecycle and consumption by a separate camera application remain unverified.

Signing preflight: `security find-identity -v -p codesigning` returned
`0 valid identities found` on this host. Signed installation and a separate
camera-client test therefore remain unavailable here. No unsigned installation
or system-security workaround was attempted. Codec implementation work is not
blocked by this signing prerequisite.

## Rust clock connected to the host (2026-09-28)

CameraSession now owns a Rust CameraClock through the C bridge instead of
computing file position directly from elapsed host time. The host exposes
Pause / Resume and Restart. Pausing freezes file time while camera timestamps
continue increasing; restarting seeks to zero without resetting camera time.
The clock skips missed slots rather than catching up with queued frames.
The bridge also exposes loop duration; the host does not yet expose a loop UI.

CameraClockTests.swift exercises the actual Swift/C/Rust boundary for skipped
slots, pause, restart while paused, resume, wrapping loop time, backward-clock
rejection and recovery. This does not prove installed camera delivery.
Signing preflight still reports zero valid identities on this host.

The separate `fvid-media` crate currently links FFmpeg through build.rs. The
camera bridge disables FVid default features and does not depend on that crate.
Removing FFmpeg from `media` remains outstanding; a camera-only dependency
check must not be interpreted as proving that wider requirement.

## Repeat control (2026-09-29)

The host now exposes Repeat video. The source bridge reports the native
container duration in nanoseconds; the host passes it to Rust CameraClock.
Looping rebases only file position, preserving increasing camera timestamps.
The preference survives opening another file. If duration is unavailable,
the host reports that repeat cannot be enabled for that file and holds EOF.
It does not scan an entire unknown-duration file during opening.

CameraRepeatTests.swift uses an actual moving MP4 through Swift/C/Rust, checks
that a middle frame differs, that the loop returns exactly the first frame,
and that disabling repeat holds the last frame. The test and unsigned app
build pass. The live host exposes repeat/pause/restart; activation remains
unverified, and the unsigned host reports the missing system-extension-install
entitlement. No installed cross-process camera delivery is claimed.

## Pixel aspect in camera fitting

The source bridge retains the reader's pixel aspect and uses Rust
`fit_bgra_aspect` when producing square-pixel camera frames. Matching coded and
output dimensions no longer bypass that conversion. CameraAspectTests.swift
checks a nonsquare-pixel Y4M through the actual Swift/C/Rust path, including
black borders, white samples and opaque alpha at two output sizes. The unsigned
bundle builds successfully; installed delivery still requires signing.

Run the clock, repeat and aspect bridge regression suites with:

```sh
python3 scripts/test_macos_camera_bridge.py
```

The runner rebuilds the Rust static library, compiles Swift with warnings as
errors and executes all four suites. The aspect suite also patches the known
MP4 fixture with a 90-degree track matrix and compares every fitted pixel and
opaque side border against the rotated source. These tests run without system
extension activation; they do not replace an installed-camera client test.


### Shared 60 fps camera cadence

`Shared/CameraFormat.swift` now defines 60 fps for both the host sample duration
and extension format. Host and sink timer periods round upward to avoid polling
before an exact rational clock slot. The host opens the Rust clock through the
new rate-explicit FFI function; the old 30 fps constructor remains ABI-compatible.
Clock tests verify every 60 fps slot through one second and suppress duplicate
polls. Pipeline tests check the 1/60 sample duration, exact BGRA, timestamps and
bounded pool reuse, alongside repeat and pixel-aspect suites. All four passed
with host access (the sandbox could not allocate IOSurface, error -6662).

An unsigned bundle built at `/tmp/FVidCamera-60fps-20260929.app`. Current signing
inspection still found zero valid identities; installed extension and external
CMIO client delivery remain unverified. Configuring 60 fps is not a throughput
measurement or interpolation of lower-rate source video.

## Current bridge verification (2026-09-29)

The standalone camera and media lockfiles now include the shared `fvid-control`
crate; the camera build had failed under `--locked` after that dependency was
introduced. Linux CI now checks the standalone camera manifest with `--locked`
as well as the root package. This CI definition has been added; the remote run
has not yet been observed.

The bridge runner now executes all eight existing suites: clock, end-to-end
in-process pipeline, repeat, pixel aspect, source errors/reset, producer queue
ownership/backpressure, pixel-pool limits, and sink row/format validation. All
eight passed locally with host access. Producer test sample duration now uses
the shared 60 fps format instead of a stale 30 fps literal.

`/private/tmp/FVidCamera-current-00fcf08.app` was built successfully as an unsigned
bundle. The camera dependency graph contains only `fvid`, `fvid-camera-ffi`,
`fvid-control` and `fvid-cpu`; neither host nor extension links FFmpeg libraries.
Signing inspection still reported zero valid identities. The bundle has not
been installed, and external application camera selection/frame delivery remain
unverified. In-process suites do not establish sustained 60 fps throughput.

### Source error diagnostics (2026-09-29)

The Rust bridge now preserves the concrete source-open, frame-decode and BGRA
conversion error instead of reducing it to a null handle or `-1`. The additive
`fvid_camera_error` ABI copies up to 4096 UTF-8 bytes from a thread-local diagnostic;
callers query/read it immediately on the failing thread. Successful source
operations clear the previous diagnostic. Swift captures it synchronously into
`NativeVideoError`, whose description is displayed by the camera session status.
A frame error still poisons the source handle and requires reopening.

Two Rust ABI tests verify bounded UTF-8, thread isolation and invalid-pointer
rejection; all eight Swift/Rust bridge suites pass with diagnostic propagation
and success-clearing checks. The host still requires a valid signing identity
and macOS extension activation for installed third-party camera delivery; no
valid signing identity was available during this verification.

### Compressed-frame bridge comparison (2026-09-29)

The regular `scripts/test_macos_camera_bridge.py` run now also compares AVC,
HEVC Main and HEVC Main10 source frames through the Swift/Rust boundary. A direct
owned software decode produces RGB and exact presentation timestamps; the Swift
source is asked for first/next/middle/last frames and then rewound, and its BGRA
bytes must match the corresponding RGB with channel swizzle and opaque alpha.
This avoids an assumed 25 fps cadence and covers 10-bit-to-RGB delivery.

All eight bridge suites plus three compressed-source comparisons passed. These
comparisons validate transport, pixel packing and frame selection against direct
software decoding; they do not replace the independent codec conformance tests
or prove delivery from an installed CMIO extension to another application.
The RGB fixture helper accepts optional `TIMESTAMPS.json` output and explicitly
uses software decoding even when built with hardware-related features.

The producer admission check is shared with an install-free test. It rejects
invalidated or unready samples, non-BGRA pixels, mismatched dimensions,
invalid/indefinite/negative timestamps and timestamps at or before the previous
accepted frame. Queue capacity and retained sample transfer are checked
separately; backpressure does not advance the accepted timestamp.

## Container crop at the camera boundary

Camera publication now clips container-stored crop borders after the native
reader's display rotation. The C ABI reports visible dimensions; its BGRA
buffer contains only that picture. RGB-to-BGRA conversion clips directly into
the existing fixed-size destination buffer without a second frame allocation.
Pixel aspect is retained for the existing fit/letterbox stage. Plain native
reader RGB remains the full display-oriented frame, so playback and raw decode
contracts are unchanged. A source that changes visible dimensions is rejected
rather than silently resizing the fixed camera source format.

Regression tests compare exact BGRA bytes to independently sliced native RGB
for cropped Matroska and rotated VP9, including EOF and rewind, and verify
malformed crop/buffer requests leave published pixels intact. These tests prove
source/ABI behavior; they do not establish installed CMIO consumer delivery.
