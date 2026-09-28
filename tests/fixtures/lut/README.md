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

## The same grid with a pre-LUT in front of it

`preluted-17.cube` is `grade-17.cube` with the header a grading suite writes
ahead of a grid: two comment lines, then `LUT_TYPE 3D`,
`VERTEX_FORMAT UNSIGNED_BYTE`, `NUM_SAMPLES 100`,
`LUT_3D_OUTPUT_RANGE 0.0 0.0 0.0`, a `LUT_1D_SHAPER_SIZE 3` curve, a
`LUT_PRELUT_1D_SIZE 3` pair that maps white to black and a
`LUT_PRELUT_3D_SIZE 5` grid of zeros.

```sh
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=preluted-17.cube:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    -f rawvideo ffmpeg-preluted-$m.rgb
  cmp ffmpeg-preluted-$m.rgb ffmpeg-$m.rgb && echo "$m: sections moved nothing"
done
```

All three compare equal, so `ffmpeg` read the file and dropped every one of
those sections — the pre-LUT that maps white to black included. The committed
hashes say so as plainly as the `cmp` does; they are the same three numbers as
above, byte for byte:

```
34339e05641f7d99c52038cbcce1aa9c47a48611fe58adc4160eae9dc138ee65  preluted-17.cube
b661856c00a38aa82af0a476148a492871d77db87b9f9ca31f96d22b0b1fb585  ffmpeg-preluted-nearest.rgb
e693f4e55d381e2d2b43866455fb747dd92083ba9936e04e50d36b6b4af4e80a  ffmpeg-preluted-trilinear.rgb
b951cb32f78210de397efd326463681c06a75af25d887ecabbddefec639ab56b  ffmpeg-preluted-tetrahedral.rgb
```

OpenColorIO 2.5.2 does not read this file at all. `ociochecklut` tries both
cube readers and reports each: `'resolve_cube' failed with: Error parsing
Resolve .cube file (preluted-17.cube). At line (3): 'LUT_TYPE 3D'. Malformed
color triples specified`, and the same sentence for `VERTEX_FORMAT`,
`NUM_SAMPLES`, `LUT_3D_OUTPUT_RANGE`, `LUT_PRELUT_1D_SIZE`,
`LUT_PRELUT_3D_SIZE` and `LUT_1D_SHAPER_SIZE` one at a time. So the tolerance
of those keys follows FFmpeg, and the refusal of them is not evidence that a
file is broken — it is one reader's grammar being narrower than the other's.

## What the domain keys are worth

`DOMAIN_MIN`/`DOMAIN_MAX` were measured above, and OCIO reads them; `ffmpeg`
does not read them at all. The two other spellings of the same idea are worth
measuring on a file OCIO's Resolve reader accepts, which the crafted 2-node
cubes above are not (there OCIO answers through its Iridas reader). Take
`grade-17.cube` with its own `DOMAIN_` lines removed, and ask `ociochecklut`
for 0.25 0.25 0.25:

| file | first line added | OCIO at 0.25 |
| --- | --- | --- |
| bare | — | 0.251519 |
| pair | `LUT_3D_INPUT_RANGE 0.0 0.5` | 0.531026 |
| triple | `DOMAIN_MIN 0 0 0` + `DOMAIN_MAX 0.5 0.5 0.5` | 0.531026 |
| six | `LUT_3D_INPUT_RANGE 0 0 0 0.5 0.5 0.5` | 0.251519 |
| other table | `LUT_1D_INPUT_RANGE 0.0 0.5` | 0.251519 |

The pair under the grid's own key moves the answer to 0.531026, which is what
the bare file gives at 0.5 — so half the declared range is the whole grid, and
`DOMAIN_MAX` says the identical thing. Six numbers on that line are not the
pair and are left unread, and the pair written under the *1D* key does not
grid; it belongs to a 1D table, where over a 17-entry ramp OCIO answers 0.25
with 0.5 and 0.5 with 1.0, the ramp read at half its own length. `ffmpeg`
returns the input byte — 64 for a 64 probe — for every row of that table, its
reader knowing neither spelling. `Lut::from_cube` follows OCIO, which is what it
already did for `DOMAIN_MIN`/`DOMAIN_MAX`.

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

## What real `.3dl` files look like

The twin above is written the way the specification-ish descriptions say, with a
bare size line on line 1. Thirty-six files from public repositories — Babylon.js
and three.js assets, Color Finesse and Photoshop preset dumps, Truelight and
Autodesk 3D Studio exports, a Blender export — were measured against the reader
instead, and not one of them is in that shape. They are not committed: they are
third-party work, and what matters is their shape rather than their pixels.

| shape | files |
| --- | --- |
| grid declared by a mesh line of 17 input codes | 30 |
| grid declared by a mesh line of 16 codes | 4 |
| grid declared by a mesh line of 7 codes (one Photoshop export) | 1 |
| nothing declares the grid at all | 1 |
| Color Finesse headers: `3DMESH` + `Mesh 4 12` | 11 |
| Color Finesse headers: `3DMESH` + `Mesh 4 16` | 1 |
| other keywords, `LUT8` and `gamma`, always beside the two above | 5 each |
| bare size line | 0 |

A mesh line lists one input code value per node along an axis, always on a
0…1023 scale whatever the grid — `0 64 128 … 960 1023` for a 17-grid — so it
carries no output information beyond the count of values, which is the size. The
second number of `Mesh 4 12` is the output depth: 35 of the 36 files hold values
up to 4095, and the one that declares `Mesh 4 16` holds values up to 65 535.
Every file has at least a `#` comment; the Color Finesse ones put their keywords
on lines 3–4, in front of the mesh line.

Before a mesh line or a keyword line was understood, 0 of the 36 parsed: 12 died
on `3DMESH`, which was read as a size, and 24 died for want of a size, because
the mesh line had been quietly folded into the rows. Reading the mesh line and
skipping the keywords took it to 35 of 36, with grid sides `{7: 1, 16: 4,
17: 30}`. The last one, `toru-ver4_sip__hoge.fuga.3dl`, declares nothing at all:
4 913 rows behind a comment and a `3DMESH` keyword, no size line and no mesh
line. Its row count is the only statement of the grid in it — an exact cube,
17 — and counting it is what makes the census 36 of 36 parse.

What that leaves wrong about the same file is not its shape but its scale: it is
the one `Mesh 4 16` among them, and its nodes reach 65 535. The second number of
`Mesh <in> <out>` is the depth the output codes are written at — Color Finesse's
writer puts a constant 12 there, and its grid sizes are 17 — so Fvid reads it as
the divisor, `2^out − 1`, and falls back on 4095 when no `Mesh` line is present.

That file turns out to be an identity grid, which settles the divisor on the
spot. Divided by 65 535, all 4 913 of its nodes come back within 0.00001 of the
ramp they stand for. Divided by 4095 — which is what `ffmpeg` does, whatever the
file declares — 4 912 of the 4 913 clip to white and only black survives.
`a_3dl_divides_by_the_depth_its_header_declares` asserts both halves of that on a
generated 7-node identity, since the file itself is third-party and is not
committed.

OpenColorIO reaches the same numbers by another road, and on these 36 files the
two agree everywhere: it infers the depth from the largest code in the file with
a 2× overshoot window (12-bit for anything in 2048…8191), so the 12 files that
declare come out 12 or 16 exactly as their headers say, and the 24 with no
`Mesh` line all max out at 4095, which is 12 either way. The header is what
Fvid follows, because the inference is not sound on its own — a legitimate
12-bit look that never rises above half scale would be read as 10-bit and come
out four times too bright. The census has one near miss of that shape already:
CH_Faded's nodes top out at 3743, and a darker preset would cross the 2048 line.

There is no oracle for any of this: `ffmpeg` refuses the Color Finesse headers
outright, so these three readings — the mesh line, the row count and the
declared depth — are held by Fvid's own tests and by the numbers above rather
than by a second implementation's bytes.

`ffmpeg` cannot arbitrate any of that: it refuses the keyword files outright.
What it does settle is that a mesh line is *only* a size declaration, which is
what `a_3dl_that_declares_its_mesh_gives_ffmpeg_the_same_table` relies on.
Rewriting the committed twin that way leaves its references untouched:

```sh
python3 - <<'PY'
t = open("grade-17.3dl").read().splitlines()
mesh = " ".join(str(round(i * 1023 / 16)) for i in range(17))
open("mesh-17.3dl", "w").write(mesh + "\n" + "\n".join(t[1:]) + "\n")
PY
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=mesh-17.3dl:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    -f rawvideo mesh-$m.rgb
  cmp mesh-$m.rgb ffmpeg-3dl-$m.rgb && echo "$m: mesh form == size form"
done
```

All three modes print their line: the mesh-declared file and the size-declared
file are the same input to `ffmpeg`, byte for byte, so the counts recorded in
the section above — 6 564, 6 635, 6 621 — are asserted against the mesh form
too, and are the same numbers.

One ambiguity the census cannot resolve: a 3-node grid's mesh line is three
values wide, which is exactly a row, and nothing in the file tells them apart.
Fvid reads such a line as a row, which leaves the file without a size and it is
refused. No writer seen does this; the smallest real grid measured is 7.


## Values past the ends, and what real `.cube` files carry

A LUT's numbers are not display codes. A size-2 cube whose red nodes read `1.5`
rather than `1` says what the reader is supposed to do with that:

```sh
cd /tmp/cubeover
python3 - <<'PY'
rows = [f"{1.5*r} {g} {b}" for b in (0, 1) for g in (0, 1) for r in (0, 1)]
open("over.cube", "w").write("LUT_3D_SIZE 2\n" + "\n".join(rows) + "\n")
open("ramp256.rgb", "wb").write(bytes(v for v in range(256) for _ in range(3)))
PY
ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 256x1 -i ramp256.rgb \
  -vf "lut3d=file=over.cube:interp=trilinear" -frames:v 1 -pix_fmt rgb24 out.rgb
```

`out.rgb` reads `(0,0,0)` (255,255,255) at the ends and `(192,128,128)` at input
128 — the red channel is `1.5 · 0.502`, so the node reached the interpolator
whole and only the byte it emits is decided by the range. A reader that clamps
the node on the way in returns `(128,128,128)` there and cannot be told apart by
any nearest-neighbour check, only by one that mixes two nodes.

`over-17.cube` is that instrument at the size real files use: a per-channel ramp
whose ends sit at exactly the minima and maxima of the six D-LUT exports in the
census below (red −0.0227 to 1.0695, green −0.0107 to 1.0588, blue −0.0038 to
1.0192).

```sh
cd tests/fixtures/lut
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=over-17.cube:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    "ffmpeg-over-$m.rgb"
done
```

`ffmpeg` 9.0.2. Sizes and hashes:

| file | bytes | sha256 |
| --- | --- | --- |
| `over-17.cube` | 133 718 | `200524c02b6bd272102939a41097d5fadec3c08e7304bb317917ade692961e3b` |
| `ffmpeg-over-nearest.rgb` | 12 288 | `710c1588cb9f0f48b641a15fe41b915773c1e64461d5c44cfb4286878cf631d5` |
| `ffmpeg-over-trilinear.rgb` | 12 288 | `9ba5181e5aa05eb44e1a1f7acff16f328391cef67b1cc2a5e6d730e2ba373c39` |
| `ffmpeg-over-tetrahedral.rgb` | 12 288 | `9ba5181e5aa05eb44e1a1f7acff16f328391cef67b1cc2a5e6d730e2ba373c39` |

The last two are the same bytes: the table is a ramp per channel, and both
interpolators reproduce a linear function exactly, so this fixture separates the
clamp question from everything else rather than testing the two interpolators
against each other. Held to the raw nodes, Fvid is within one code on all 12 288
channels — 6 400 of them by exactly one at nearest, 5 888 at each of the others,
and `ffmpeg`'s byte is the saturated floor of Fvid's float in every mode. Read
with the ends cut off before the table, as `from_cube` used to do, 384 channels
move by more than one code through trilinear and through tetrahedral alike, the
worst by four, and 192 channels `ffmpeg` clips to white and 64 to black come back
inside the range instead. Nearest is untouched by the clamp, which is why the
defect shows up in a graded picture and not in a node-by-node check.

## Domains: no oracle, and the file that states it

`ffmpeg` ignores `DOMAIN_MIN`/`DOMAIN_MAX` — for `.3dl` and for `.cube` both. A
size-2 cube whose nodes are seven red nodes and one blue, run once with
`DOMAIN_MAX 0.1 0.1 0.1` and once with the two DOMAIN lines stripped, writes the
same 16 pixels byte for byte: the switch stays at the ramp's own midpoint, not at
0.1. So the domain cannot be measured against `ffmpeg`, and there is no oracle at
all for the one-dimensional case, because `lut3d` refuses a 1D `.cube` outright
("3D LUT is empty").

Two authorities are enough without it. The first is a real file: abpy's
negative-printing table opens with `# input: log10/density, output: linear`,
declares `DOMAIN_MIN 0` / `DOMAIN_MAX 4` and lists 1024 rows whose every value is
`10^(in − 2)` for `in` over that declared range — the largest error over the
whole table is 5e-9. Its values run from 0.01 to 100, which is a *linear light*
output, not an input: the domain belongs to the index, and a reader that divides
the rows by it pins 358 of the 1024 to white. The second is OpenColorIO's
`FileFormatResolveCube.cpp`, which for both kinds of table stores the numbers
unrescaled and puts a `MinMaxOp` with the declared range *in front* of the LUT
op.

## What real `.cube` files look like

Sixty `.cube` files pulled from GitHub for measurement, not committed, in
`/tmp/dlcube/samples`. Counting a colour LUT as a file with a size line:

| shape | files |
| --- | --- |
| 3D, size 13 | 13 |
| 3D, size 15 | 6 |
| 3D, size 16 | 3 |
| 3D, size 17 | 10 |
| 3D, size 2 / 4 | 2 / 1 |
| 1D, length 2 / 4 | 1 / 1 |
| 1D, length 512 | 1 |
| 1D, length 1024 | 5 |
| 1D, length 4096 | 1 |
| both tables in one file | 0 |
| non-default `DOMAIN_MIN`/`MAX` | 5 |
| CRLF line endings | 2 |
| any value outside 0..1 | 10 |
| no size line at all | 16 |

All 44 with a size line import, and they did not before this section's fixes:
seven of the nine 1D tables are longer than 128 entries — the Apple Log to Linear
export at 4096 and six density/inverse tables at 512 and 1024 — and one bound for
both size lines refused every one of them. A grid's bound is the grid's memory;
a list's length its own rows settle. OpenColorIO keeps the two apart at 129 for a
side and 300 000 for a table, which is where Fvid's ceilings now sit.

Ten of the 44 carry values past the ends, which is what makes the clamp a look
question rather than a validation one: the six D-LUT grids (`-0.023..1.07`), the
Apple log-to-linear table (`-0.056..12.0`, so 3 753 of its 12 288 numbers are
above white and 1 851 below black), and three of abpy's density tables.

The 16 files with no size line share the extension without the format: ten
Gaussian volumetric dumps (`Psi4 Gaussian Cube File.`), one `chem.cr` spectral
cube, one git-lfs pointer and three prose and type-signature notes. Fifteen of
them are refused. Each first line says what the file is, and it is quoted in the
refusal, because a user typing `--lut` has no way to know that the extension is
taken by chemistry output.

The sixteenth deserves the record rather than a footnote: `XUANTIE-RV` ships an
ISP configuration table of 4 913 integer rows, which is exactly `17³`, so the
row-count fallback added for undeclared `.3dl` files reads it as a 17-grid — the
two shapes are not distinguishable from the inside. Nothing sensible comes of it
(the codes reach 16 303, so the grid divides to white), but it is imported, and
that is the price of the rule recorded above.

## The SPI tables

Two files from OpenColorIO's own test data, taken byte for byte (`cmp` says so)
from `https://raw.githubusercontent.com/AcademySoftwareFoundation/OpenColorIO/main/tests/data/files/`,
where they are unmodified test inputs under that project's BSD-3-Clause licence:

| file | bytes | sha256 |
| --- | --- | --- |
| `sRGB_to_linear.spi1d` | 96 311 | `4240e29bd6638d46d68bdcb83ea321428c1a00e4360e44dd84864f78b86ff0e9` |
| `lut3d_bizarre.spi3d` | 1 246 | `1bb96707129632920e50237696feade4907dc27081076ad0ca3741fcc9a2916e` |

Neither is read by `tests/lut_ffmpeg.rs`, because `ffmpeg lut3d` has no SPI
reader to agree or disagree with — measured on 9.0.2, the same one-liner that
accepts `grade-17.cube` fails the grid:

```console
$ ffmpeg -f lavfi -i color=gray:s=8x8 -vf lut3d=file=lut3d_bizarre.spi3d -f null -
[AVFilterGraph] Error initializing filters ... Invalid argument
```

Both are read by the module tests in `src/color/lut.rs`.

OpenColorIO does read both, and since it authored these two files it is the
reader to ask when FFmpeg cannot answer. Measured on 2.5.2, the CLI evaluates a
colour through each file:

```console
$ ociochecklut sRGB_to_linear.spi1d 0.25 0.25 0.25
0.05087609 0.05087609 0.05087609
$ ociochecklut sRGB_to_linear.spi1d 0.5 0.5 0.5
0.2140411 0.2140411 0.2140411
$ ociochecklut sRGB_to_linear.spi1d 0.75 0.75 0.75
0.5225216 0.5225216 0.5225216
$ ociochecklut lut3d_bizarre.spi3d 1.0 0.0 0.0
1.622678 -0.04887585 -0.09775171
$ ociochecklut lut3d_bizarre.spi3d 0.0 0.0 1.0
0 0.09775171 1.17302
$ ociochecklut lut3d_bizarre.spi3d 0.5 0.5 0.5
0.3714565 0.3910069 0.3910069
```

Every answer is the sRGB decode of the input for the shaper, and for the grid it
is the node the file's own `r g b` indices point at — the red 1.622 678 at
[1, 0, 0] and the green 0.097 751 71 at [0, 0, 1] are the two digits a reader
that walked the rows by position would get wrong. `ociochecklut` also prints the
range it parsed for the grid as `minrgb=[-0.0782014, -0.0977517, -0.0977517]`,
`maxrgb=[1.62268, 1.75953, 1.17302]`, so the overshoot the tests keep
uncoloured is what the reference reader itself sees.

Note that the CLI takes its probe as separate arguments: passing `"0.5 0.5 0.5"`
as one word answers `ERROR: Expecting either RGB or RGBA pixel`, which reads
like a refusal of the file and is not one.

`sRGB_to_linear.spi1d` is the sRGB shaper: `From -0.125 1.125`, `Length 4101`,
`Components 1`. The pair is the input range, so entry 410 is the EOTF at signal
0.0 and entry 3690 at 1.0, while the ends of the table run on to −0.0096 and
1.3083 past them — the headroom a shaper space is there to carry, and what the
test compares against fvid's own `Transfer::Srgb` curve
(`a_real_spi1d_is_the_srgb_curve_over_the_range_it_declares`).

`lut3d_bizarre.spi3d` is a 3³ grid whose rows are deliberately not in the order
the table stores a node: the file lists blue fastest and each row states its own
`r g b` ahead of its values, and its nodes reach 1.76 above white and −0.098
below black. Read by row position instead of by those indices, the grid is a
different look — which is the mistake the file exists to catch
(`a_spi3d_row_places_its_node_by_its_own_indices`).

## The Iridas `.dat`

`ffmpeg`'s `parse_dat` is the only implementation of this format, and between
them its grammar is four lines: one optional `3DLUTSIZE <side>` first line (33 if
the file states none), then side³ rows of three floats, blank and `#` lines
skipped anywhere, the values taken as written and never divided by anything. What
the grammar does not say is which axis the rows run fastest, so the same
instrument that settled the `.3dl` was run against it — a 17-grid whose every row
records its own file position, one digit per channel, so the output byte triple
decodes back to the row `ffmpeg` sampled for that node:

```sh
python3 - <<'PY'
N = 17
with open("pos-17.dat", "w") as f:
    f.write("3DLUTSIZE 17\n")
    for d in range(N**3):      # the row records its own position, one digit per channel
        f.write("{} {} {}\n".format((d % N) / 16, ((d // N) % N) / 16, (d // (N * N)) / 16))
codes = [round(255 * j / (N - 1)) for j in range(N)]
px = bytearray()
for jr in range(N):
    for jg in range(N):
        for jb in range(N):
            px += bytes([codes[jr], codes[jg], codes[jb]])
open("probe-17.rgb", "wb").write(bytes(px))
PY
ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 4913x1 -i probe-17.rgb \
  -vf "lut3d=file=pos-17.dat:interp=nearest" -frames:v 1 -pix_fmt rgb24 -f rawvideo out-pos.rgb
```

Every one of the 4 913 nodes answers to `b + 17·(g + 17·r)`, and none to the
red-fastest order — a clean sweep rather than the 289-node diagonals the `.3dl`
run left over, because this grid states all three digits instead of one count. A
`.dat` lists its grid blue fastest, the same way round as a `.3dl` and the
reverse of a `.cube`, so `from_dat` transposes through the same helper `from_3dl`
uses.

`grade-17.dat` is that reading of the same look as `grade-17.cube`, and it was
chosen out of all six assignments of the file's digit places to red, green and
blue. Each candidate was run through `ffmpeg` and compared with the committed
`.cube` references — same probe, same interpolation, byte for byte:

| digit order (fastest first) | nearest | trilinear | tetrahedral |
| --- | --- | --- | --- |
| blue, green, red | **0** | **0** | **0** |
| red, green, blue | 10 601 | 11 101 | 11 082 |
| red, blue, green | 12 026 | 12 166 | 12 162 |
| green, red, blue | 12 053 | 12 184 | 12 178 |
| green, blue, red | 10 874 | 11 533 | 11 516 |
| blue, red, green | 10 542 | 11 156 | 11 168 |

One file of six is the look; the other five disagree with it on 10 542 to 12 184
of the probe's 12 288 bytes. That the winner needs no reference files of its own
is the point: it is byte-identical to the cube's, so the oracle is shared.

```sh
for m in nearest trilinear tetrahedral; do
  ffmpeg -v error -y -f rawvideo -pix_fmt rgb24 -s 64x64 -i probe.rgb \
    -vf "lut3d=file=grade-17.dat:interp=$m" -frames:v 1 -pix_fmt rgb24 \
    -f rawvideo dat-$m.rgb
done
```

| file | bytes | sha256 |
| --- | --- | --- |
| `grade-17.dat` | 132 664 | `331064f3db3a27b7b6bc816094dc0fd0720f3f72392b300a8c20880990e75e13` |

`tests/lut_ffmpeg.rs` holds the agreement: the table Fvid builds from the `.dat`
is the same 4 913 nodes it builds from the `.cube` — every value bit-identical,
since both files carry the same six decimals — and its samples meet `ffmpeg`
within one code on all 12 288 channels in each mode, 4 826 / 5 354 / 5 298 of
them by exactly one, which is the cube's own residue, with `ffmpeg`'s byte still
the floor of Fvid's float. Taken in the order a `.cube` lists instead, the
identical rows move 9 696 channels by more than one code at nearest (10 061 and
10 056 through trilinear and tetrahedral), the worst by the full 255
(`a_dat_of_the_same_look_walks_with_ffmpeg`,
`a_dat_read_red_fastest_is_a_different_look_not_a_rounding_gap`).

Three places where Fvid is tighter than the reference, each of them a file
`ffmpeg` reads and this one refuses — measured on 9.0.2, not inferred:

* rows behind the grid it needs: accepted there (`... 0 0 0` past a 2³ stops the
  loop), refused here;
* a row with a fourth number: accepted there, since its scan reads three,
  refused here;
* no `3DLUTSIZE` line at all: `ffmpeg` then fixes the file at 33 nodes and says
  `Unexpected EOF` for the 35 929 rows it is short of, where an exact row count
  is read as the declaration the way a headerless `.3dl` is.

The other bound goes the other way: `parse_dat` accepts a grid up to 256 nodes a
side (257 answers `Too large or invalid 3D LUT size`), where every grid format
here stops at 128 — 201 MiB of nodes rather than 25, in a file of 16 777 216
lines.
