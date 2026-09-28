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

## The `.3dl` twin

`grade-17.3dl` is the same look written in the other format: 12-bit integers
instead of floats, one bare size line instead of directives, and the grid listed
with the **blue** axis varying fastest, which is the reverse of a `.cube`. The
file was generated from `grade-17.cube` by that rule alone:

```python
N = 17
cube = []
for line in open("grade-17.cube"):
    f = line.split()
    if len(f) == 3 and (f[0][0].isdigit() or f[0][0] in ".-"):
        cube.append([float(x) for x in f])
with open("grade-17.3dl", "w") as out:
    out.write(f"{N}\n")
    for r in range(N):          # position p = b + N·(g + N·r) holds node (r, g, b)
        for g in range(N):
            for b in range(N):
                v = cube[r + N * (g + N * b)]
                out.write("{} {} {}\n".format(*(round(x * 4095) for x in v)))
```

Read back, the two files agree node for node: 0 of the 4 913 mismatch under the
blue-fastest layout, while 4 600 of them would land on the wrong colour if the
`.3dl` were laid out the way a cube lists itself.

`ffmpeg` 9.0.2, one run per mode, in this directory:

```sh
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=grade-17.3dl:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    -f rawvideo ffmpeg-3dl-$m.rgb
done
```

```
b8964cfb81791ee3ff0ed5c4115104fe530a5350e86010ed2cb797f677e9b968  grade-17.3dl
a70371ec95a3ff5acbd44bb176199850fe430b3b0901148a28fc65cb3d94117d  ffmpeg-3dl-nearest.rgb
a1864f49c646629cef19839c4620c2de1148bf3c59f0e22b4f57519966e07d8f  ffmpeg-3dl-trilinear.rgb
eaa869f094f947f1f62c1d1176bd9a4c2389b18796d9a9c7f1a92ff8ac11b776  ffmpeg-3dl-tetrahedral.rgb
```

Over all 12 288 channels Fvid's sampler of the `.3dl` never leaves `ffmpeg`'s
byte by more than one code: 6 564 channels at nearest, 6 635 at trilinear, 6 621
at tetrahedral. Those are more than the cube's counts because the nodes
themselves are quantised to 1/4095, so a value sitting on an 8-bit rounding
boundary is decided by six bits the picture does not have. The direction is the
same as the cube's and just as strict: `ffmpeg` is never the higher byte, in
none of the three modes — which is why this test pins the count and the sign
instead of the truncation identity the cube test asserts, that one no longer
holds once the nodes are rounded (Fvid reads a node as 4095/4095 exactly, and
`ffmpeg` paints 254 where the float says 255).

## Which axis varies fastest

The axis order is settled by an instrument rather than by the twin above: a grid
whose every node stores its own position in the file. Not committed — a size-17
`.3dl` is 4 914 lines, and only the reading of it matters:

```sh
python3 - <<'PY'
N = 17
with open("pos-17.3dl", "w") as f:
    f.write(f"{N}\n")
    for i in range(N**3):      # node i carries its own index on all three channels
        f.write("{} {} {}\n".format(*([i * 150] * 3)))
codes = [round(255 * j / (N - 1)) for j in range(N)]
px = bytearray()
for jr in range(N):
    for jg in range(N):
        for jb in range(N):
            px += bytes([codes[jr], codes[jg], codes[jb]])
open("probe-17.rgb", "wb").write(bytes(px))
PY
ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 4913x1 -i probe-17.rgb \
  -vf "lut3d=pos-17.3dl:interp=nearest" -frames:v 1 -pix_fmt rgb24 out-pos.rgb
```

Each output byte is `round(i · 150 · 255 / 4095)`, so it decodes back to the file
position `ffmpeg` sampled for that probe colour. For all 4 913 nodes that
position is `b + 17·(g + 17·r)`; the five other orders fit 289, 289, 17, 17 and
289 nodes respectively — the diagonals, where the orders coincide. The same
conclusion is in Autodesk's own reader, `FileFormat3DL.cpp` in OpenColorIO: "The
3dl format stores the LUT entries in blue-fastest order."

Two format quirks the runs above also showed, neither of them a Fvid problem:
`ffmpeg`'s `.3dl` reader refuses any grid below 17 (`Unexpected EOF` for 2, 3,
5, 8, 9, 15 and 16, and it works for 17, 20, 33), and it will not read a 1D
`.cube` at all — `lut3d` says "3D LUT is empty" — so a one-dimensional look has
no oracle here beyond `lut1d`, which has a different syntax again.

