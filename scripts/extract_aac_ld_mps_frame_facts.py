"""Freeze reshape numeric codeword facts, explicit external ISO archive step."""
import hashlib, json, sys
from pathlib import Path
from zipfile import ZipFile
from extract_aac_ld_mps_huffman_facts import tree, words
ROOT = Path(__file__).resolve().parents[1]
def generate(archive):
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    assert digest == json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text())['archive_sha256']
    member = 'mp4spatialdec/src/sac_nodes_reshape.c'
    with ZipFile(archive) as z: raw = z.read(member)
    rows = words(tree(raw.decode(), 'huffReshapeNodes')[0])
    assert len(rows) == 40 and {r[2] for r in rows} == {(v << 4) + run for v in range(5) for run in range(8)}
    out = ROOT/'tests/fixtures/playback-errors/aac-ld-mps-frame'
    out.mkdir(exist_ok=True)
    (out/'reshape-codewords.json').write_text(json.dumps(rows,indent=2)+'\n')
    code = '//! Numeric GES reshape prefix facts: length, code, value/run symbol.\n'
    code += 'pub(super) const RESHAPE: &[(u8, u32, i16)] = &[\n'+''.join(f'    ({n}, {c}, {v}),\n' for n,c,v in rows)+'];\n'
    (ROOT/'crates/fvid-media/src/owned_aac/aac_ld_mps_frame_facts.rs').write_text(code)
    report = dict(archive_sha256=digest,member=member,member_sha256=hashlib.sha256(raw).hexdigest(),codewords=len(rows),scope='Numeric prefix facts only; no foreign decoder implementation.')
    (ROOT/'docs/validation/aac-ld-mps-frame-facts-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n')
if __name__ == '__main__': generate(Path(sys.argv[1]))
