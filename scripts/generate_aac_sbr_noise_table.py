#!/usr/bin/env python3
"""Extract protocol noise constants and cross-check two standard publications.
Explicit local inputs; no decoder code, FFmpeg or network access.
"""
from pathlib import Path
from decimal import Decimal
import re,sys,json,hashlib,struct
root=Path(__file__).resolve().parents[1]
html=Path(sys.argv[1]).read_text();draft=Path(sys.argv[2]).read_text()
a=html.index("Таблица А.91",8_000_000);b=html.index("</table>",a)
values={}
for row in re.findall(r"<tr\b[^>]*>(.*?)</tr>",html[a:b],re.S):
    cells=[re.sub(r"<[^>]+>","",x).strip().replace(",",".") for x in re.findall(r"<td\b[^>]*>(.*?)</td>",row,re.S)]
    if len(cells)==3 and cells[0].isdigit():values[int(cells[0])]=tuple(Decimal(x) for x in cells[1:])
reference={}
a=draft.index("Table 1.A.13 Noise table")
number=r"(-?\d+\.\d+)"
pattern=r"^\s*(\d+)\s+"+number+r"\s+"+number+r"\s+(\d+)\s+"+number+r"\s+"+number+r"\s*$"
for match in re.finditer(pattern,draft[a:],re.M):
    i,re1,im1,j,re2,im2=match.groups();reference[int(i)]=(Decimal(re1),Decimal(im1));reference[int(j)]=(Decimal(re2),Decimal(im2))
assert set(values)==set(range(512)) and set(reference)==set(range(512))
for i in range(512):assert values[i]==reference[i],(i,values[i],reference[i])
source='//! Normative SBR noise table A.91, cross-checked with ISO table 1.A.13.\nuse super::aac_sbr_qmf::Complex;\npub const NOISE: [Complex; 512] = [\n'
source+=''.join('    Complex { re: '+str(values[i][0])+', im: '+str(values[i][1])+' },\n' for i in range(512))+'];\n'
(root/'crates/fvid-media/src/owned_aac/aac_sbr_noise_table.rs').write_text(source)
binary=b''.join(struct.pack('<dd',float(values[i][0]),float(values[i][1])) for i in range(512))
fixtures=root/'tests/fixtures/playback-errors'
(fixtures/'aac-sbr-noise-protocol.f64le').write_bytes(binary)
(fixtures/'aac-sbr-noise-protocol.json').write_text(json.dumps({'source':'GOST A.91 and ISO draft 1.A.13; all 512 complex values equal','sha256':hashlib.sha256(binary).hexdigest()},indent=2)+'\n')
print('Cross-checked all 512 complex noise constants')
