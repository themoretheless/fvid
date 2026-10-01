# High-depth file-to-camera validation

Synthetic 10-bit YUV420 and 16-bit YUV444 Y4M files now exercise the owned
file-to-camera path. Each has two visibly different frames. Tests verify BGRA
publication, alpha 255, held frames, backward seek and loop. The same samples
pass through the Rust API and the C ABI used by the macOS extension.

The expected RGB samples are calculated by an independent integer/rational
inverse BT.601 matrix, rounded once at RGB24 output. Matrix coefficients and
limited-range code conventions are grounded in the primary specification:
[ITU-R BT.601-7](https://www.itu.int/dms_pubrec/itu-r/rec/bt/R-REC-BT.601-7-201103-I%21%21PDF-E.pdf).
All camera RGB samples match this oracle byte-for-byte. An additional saved
FFmpeg reference (nearest chroma sampling, explicit ranges/matrix, dithering
disabled) is checked within three code values. An initial tolerance of two
failed on this reference; disabling dithering did not remove the difference.
The exact rational oracle distinguishes this comparison limit from a playback
failure. No production colour-conversion implementation was changed here.

Generation is separate: `scripts/generate_camera_depth_samples.py` hand-authors
Y4M samples and the rational RGB oracle, and invokes FFmpeg only to save its
additional reference. Ordinary tests invoke neither FFmpeg nor network access.

Validation: two high-depth camera tests, two existing camera resize tests, four
camera-ffi unit tests and the new C ABI integration test passed. Native dependency
checks report no FFmpeg adapter in headless, player or camera bridge builds.

This validates file-to-BGRA APIs, not macOS device registration. At the current
check `systemextensionsctl list` contains no FVid extension. Provisioning profiles
were not found in the standard local locations or the installed FVid bundle.
Signed host/extension provisioning and frame reception in another application
remain unverified requirements.
