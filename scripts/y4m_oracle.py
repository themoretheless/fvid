"""Independent byte-level planar crop/reflection oracle; no FVid or codecs."""
import io
import re

SUBSAMPLING = {b"420": (2, 2), b"422": (2, 1), b"444": (1, 1)}


def transform(data, crop=None, horizontal=False, vertical=False):
    source = io.BytesIO(data)
    header = source.readline()
    if not header.startswith(b"YUV4MPEG2 "):
        raise ValueError("not Y4M")
    width = int(re.search(rb" W(\d+)", header)[1])
    height = int(re.search(rb" H(\d+)", header)[1])
    chroma = re.search(rb" C(420|422|444)(?:\s|$)", header)
    if chroma is None:
        raise ValueError("oracle supports planar 8-bit 420/422/444")
    sx, sy = SUBSAMPLING[chroma[1]]
    if width <= 0 or height <= 0 or width % sx or height % sy:
        raise ValueError("oracle requires positive aligned plane dimensions")
    x, y, output_width, output_height = crop or (0, 0, width, height)
    if x < 0 or y < 0 or output_width <= 0 or output_height <= 0 or x + output_width > width or y + output_height > height:
        raise ValueError("crop outside source")
    if x % sx or y % sy or output_width % sx or output_height % sy:
        raise ValueError("unaligned chroma crop")
    header = re.sub(rb" W\d+", b" W" + str(output_width).encode(), header)
    header = re.sub(rb" H\d+", b" H" + str(output_height).encode(), header)
    output = bytearray(header)
    geometry = [(1, 1), (sx, sy), (sx, sy)]
    while marker := source.readline():
        if not marker.startswith(b"FRAME") or not marker.endswith(b"\n"):
            raise ValueError("invalid frame marker")
        output.extend(marker)
        for dx, dy in geometry:
            plane_width, plane_height = width // dx, height // dy
            plane = source.read(plane_width * plane_height)
            if len(plane) != plane_width * plane_height:
                raise ValueError("truncated Y4M plane")
            rows = range(y // dy, (y + output_height) // dy)
            if vertical:
                rows = reversed(rows)
            for row in rows:
                begin = row * plane_width + x // dx
                pixels = plane[begin:begin + output_width // dx]
                output.extend(pixels[::-1] if horizontal else pixels)
    return bytes(output)
