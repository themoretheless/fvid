#!/usr/bin/env python3
"""Explicit ISO scalar STP oracle extraction. Never run by ordinary tests.

Only the energy/gain prefix is executed. The invalid LD final frequency-mixing
loop is excluded, not corrected. Tree 7 is excluded because its prefix reads
uninitialized channel indices. Trees 5/6 have uninitialized surplus downmix
energy rows. This oracle makes no claim about these trees or PCM playback.
"""
import hashlib
import json
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MEMBER = 'mp4spatialdec/src/sac_TPprocess.c'


def sha(b):
    return hashlib.sha256(b).hexdigest()


def main(archive):
    archive = Path(archive).read_bytes()
    if sha(archive) != 'b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83':
        raise ValueError('Unexpected ISO reference archive')
    import io
    with zipfile.ZipFile(io.BytesIO(archive)) as z:
        raw = z.read(MEMBER)
    source = raw.decode().replace('\r\n', '\n')
    source = source[source.index('#define BP_SIZE'):]
    boundary = source.index('  for (ch=0; ch<self->numOutputChannels; ch++) {\n\n    no_scaling = 1;')
    source = source[:boundary]
    # Instrument pre-normalization energies; retain reference float arithmetic.
    for kind in ('Dry', 'Wet'):
        token = f'    run{kind}Ener[ch] ='
        assert source.count(token) == 1
        source = source.replace(token, f'    captured_{kind.lower()}[ch] = {kind}Ener[ch] / (32768. * 32768.);\n' + token)
    source += 'for(ch=0;ch<self->numOutputChannels;ch++) captured_gain[ch]=scale[ch];\n}\n'
    prefix = '''#include <math.h>
#include <stdio.h>
#include <stdlib.h>
#define LD_MPS 1
#define MAX_INPUT_CHANNELS 8
#define MAX_OUTPUT_CHANNELS 8
#define MAX_NUM_QMF_BANDS 64
#define ABS_THR (1e-9f * 32768 * 32768)
typedef struct { int treeConfig,numInputChannels,numOutputChannels,qmfBands;
float hybOutputRealDry[8][1][64],hybOutputImagDry[8][1][64];
float hybOutputRealWet[8][1][64],hybOutputImagWet[8][1][64]; } spatialDec;
static double captured_dry[8],captured_wet[8],captured_gain[8];
'''
    driver = r'''
int main(int argc,char **argv) {
 spatialDec s={0}; int t,ch,b,slot,pattern;
 if(argc!=3) return 2;
 t=atoi(argv[1]); pattern=atoi(argv[2]);
 s.treeConfig=t; s.numInputChannels=t<2?1:2;
 s.numOutputChannels=t<3?6:8; s.qmfBands=32;
 puts("[");
 for(slot=0;slot<100;slot++) {
  for(ch=0;ch<s.numOutputChannels;ch++) for(b=0;b<32;b++) {
   float pulse=pattern==0?1.f:pattern==1?(slot%17<3?3.f:.125f):pattern==2?(slot<40?0.f:1.f):(slot%2?8.f:.03125f);
   float d=32768.f*pulse*((ch*7+b*3+slot)%19-9)/32.f;
   float w=32768.f*((ch*11+b*5+slot*3)%23-11)/64.f;
   if(pattern==2 && slot>=40 && slot<65) w=0;
   s.hybOutputRealDry[ch][0][b]=d;
   s.hybOutputImagDry[ch][0][b]=d*.25f;
   s.hybOutputRealWet[ch][0][b]=w;
   s.hybOutputImagWet[ch][0][b]=-w*.5f;
   if(b<12) {
    s.hybOutputRealDry[ch][0][b]+=w;
    s.hybOutputImagDry[ch][0][b]-=w*.5f;
    s.hybOutputRealWet[ch][0][b]=0;
    s.hybOutputImagWet[ch][0][b]=0;
   }
  }
  subbandTP(&s,0);
  if(slot) puts(",");
  printf("{\"dry\":[");
  for(ch=0;ch<s.numInputChannels;ch++) printf("%s%.17g",ch?",":"",captured_dry[ch]);
  printf("],\"wet\":[");
  for(ch=0;ch<s.numOutputChannels;ch++) printf("%s%.17g",ch?",":"",captured_wet[ch]);
  printf("],\"gain\":[");
  for(ch=0;ch<s.numOutputChannels;ch++) printf("%s%.17g",ch?",":"",captured_gain[ch]);
  printf("]}");
 }
 puts("]"); return 0;
}
'''
    cases = []
    with tempfile.TemporaryDirectory(prefix='fvid-stp-energy-') as tmp:
        p = Path(tmp)
        (p/'probe.c').write_text(prefix + source + driver)
        subprocess.run(['cc', '-O1', '-ffp-contract=off', '-fsanitize=undefined,bounds',
                        '-fno-sanitize-recover=all', str(p/'probe.c'), '-lm', '-o', str(p/'probe')], check=True)
        for tree in range(5):
            for pattern in range(4):
                result = subprocess.run([str(p/'probe'), str(tree), str(pattern)], check=True,
                                        capture_output=True, text=True, timeout=30)
                cases.append(dict(tree=tree, pattern=pattern, slots=json.loads(result.stdout)))
    out = ROOT/'tests/fixtures/playback-errors/aac-ld-mps-stp-energy'
    out.mkdir(exist_ok=True)
    payload = (json.dumps(dict(scope='Extracted ISO scalar energy/gain stage only; no final LD mixing or PCM oracle.',
                               cases=cases), separators=(',', ':'))+'\n').encode()
    (out/'reference.json').write_bytes(payload)
    report = dict(archive_sha256=sha(archive), member=MEMBER, member_sha256=sha(raw),
                  extracted_instrumented_source_sha256=sha((prefix+source+driver).encode()),
                  generator_sha256=sha(Path(__file__).read_bytes()), output_sha256=sha(payload),
                  programmes=len(cases), slots=sum(len(c['slots']) for c in cases),
                  reference_gains=sum(len(s['gain']) for c in cases for s in c['slots']),
                  scope='Unchanged scalar reference operations with input-energy/output-gain capture; final LD mixing excluded. Sanitized extraction for trees 0..4 only. No playback qualification.')
    (ROOT/'docs/validation/aac-ld-mps-stp-energy-reference-2026-10-30.json').write_text(json.dumps(report, indent=2)+'\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main(sys.argv[1])
