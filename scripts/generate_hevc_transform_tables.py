#!/usr/bin/env python3
"""Extract H.265 (08/2021) equations 8-319/8-321 from pypdf text."""
import pathlib
import re
import sys

source = pathlib.Path(sys.argv[1]).read_text().replace("−", "-")
halves = []
for name in ["transMatrixCol0to15 =", "transMatrixCol16to31 ="]:
    section = source.split(name, 1)[1]
    rows = re.findall(r"\{\s*((?:-?\d+\s*)+)\}", section)[:32]
    values = [list(map(int, row.split())) for row in rows]
    assert len(values) == 32 and all(len(row) == 16 for row in values)
    halves.append(values)
matrix = [a + b for a, b in zip(*halves)]
assert matrix[0] == [64] * 32
assert all(row[31-i] == (-1 if k % 2 else 1) * row[i]
           for k, row in enumerate(matrix) for i in range(16))
output = "// Generated from H.265 (08/2021), equations 8-319 and 8-321.\n"
output += "pub(super) const DCT: [[i16; 32]; 32] = [\n"
output += "".join("    [" + ", ".join(map(str, row)) + "],\n" for row in matrix)
output += "];\n"
pathlib.Path(sys.argv[2]).write_text(output)
