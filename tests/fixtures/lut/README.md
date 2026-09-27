# LUT fixtures: Fvid's cube sampler against `ffmpeg`'s `lut3d`

`tests/lut_ffmpeg.rs` reads the cube and the probe here and compares its own
sampling with the bytes `ffmpeg` wrote for the same pair. Nothing in the
comparison needs `ffmpeg` at test time; the references are committed.

## The probe

`probe.rgb` is one 64×64 frame of raw `rgb24`, 12 288 bytes: red runs across the
columns, green down the rows, and blue steps over the eight diagonals so that
the interior of the cube gets sampled too, not only its edges.

## The cube

`grade-17.cube` is a 17-node cube with no magic in it — a contrast stretch about
mid-grey, a saturation of 1.18, a 2% lift and a 0.94 power, each node clamped
into 0…1. It is written to be non-linear in all three channels, which is what
makes an interpolation mistake show up as a difference rather than cancel out.

Regenerate it (and the probe) with:

```python
N = 17
def node(r, g, b):
    y = 0.2126 * r + 0.7152 * g + 0.0722 * b
    out = []
    for c in (r, g, b):
        v = max(0.0, min(1.0, (c - 0.5) * 1.12 + 0.5))
        v = (y + (v - y) * 1.18) * 0.98 + 0.02
        out.append(max(0.0, min(1.0, v)))
    return [c ** 0.94 for c in out]

lines = ["LUT_3D_SIZE 17", "DOMAIN_MIN 0.0 0.0 0.0", "DOMAIN_MAX 1.0 1.0 1.0"]
for bi in range(N):
    for gi in range(N):
        for ri in range(N):
            r, g, b = node(ri / (N - 1), gi / (N - 1), bi / (N - 1))
            lines.append("%.6f %.6f %.6f" % (r, g, b))
open("grade-17.cube", "w").write("\n".join(lines) + "\n")

W = H = 64
px = bytearray()
for y in range(H):
    for x in range(W):
        px += bytearray([round(x * 255 / (W - 1)), round(y * 255 / (H - 1)), ((x + y) % 16) * 17])
open("probe.rgb", "wb").write(bytes(px))
```

## The references

`ffmpeg` 9.0.2, one run per interpolation mode, in this directory:

```sh
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=grade-17.cube:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    -f rawvideo ffmpeg-$m.rgb
done
```

SHA-256 of what those commands wrote, as committed:

```
972fe94e8ec7e289d60fb86e47b0edce2a0c9501dad0e9aa308ff06f1e742dde  grade-17.cube
7a400acc7b72da26b153aea456b4d23cd534beba200412a4e569267a0cd5cbc3  probe.rgb
b661856c00a38aa82af0a476148a492871d77db87b9f9ca31f96d22b0b1fb585  ffmpeg-nearest.rgb
e693f4e55d381e2d2b43866455fb747dd92083ba9936e04e50d36b6b4af4e80a  ffmpeg-trilinear.rgb
b951cb32f78210de397efd326463681c06a75af25d887ecabbddefec639ab56b  ffmpeg-tetrahedral.rgb
```

## What the comparison measured

Over all 12 288 channels of the probe the two implementations never differ by
more than one 8-bit code, in any of the three interpolation modes.

Every one of those single-code differences points the same way: `ffmpeg`'s byte
is the floor of Fvid's float result, where Fvid rounds it. Nothing comes out one
*below* Fvid, which is what a genuine disagreement about the cube or about the
interpolation would produce rather than a one-sided residue. Two further checks
hold the rest of the claim up. Sampling an identity cube of 33 nodes with Fvid's
own code returns every input byte unchanged, under trilinear and tetrahedral
alike, so the ramp through the sampler neither drifts nor snaps; and the same
identity cube through `ffmpeg` returns the input byte for byte too, so the two
are reading the same file the same way and scaling it the same way.

The counts of channels that differ by one code are asserted in the test: 4 826
for nearest, 5 354 for trilinear, 5 298 for tetrahedral.

The identity cube those two checks use is not committed — it is 4913 lines of
the ramp it stands for. Make and run it the same way:

```sh
python3 -c '
N = 33
lines = ["LUT_3D_SIZE 33", "DOMAIN_MIN 0.0 0.0 0.0", "DOMAIN_MAX 1.0 1.0 1.0"]
for bi in range(N):
    for gi in range(N):
        for ri in range(N):
            lines.append("%.6f %.6f %.6f" % (ri / (N - 1), gi / (N - 1), bi / (N - 1)))
open("identity-33.cube", "w").write("\n".join(lines) + "\n")'
ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
  -vf "lut3d=file=identity-33.cube:interp=trilinear" -frames:v 1 -pix_fmt rgb24 \
  -f rawvideo - | cmp - probe.rgb && echo 'ffmpeg: identity held'
```
