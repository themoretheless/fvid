#!/usr/bin/env python3
"""Explicit frozen ISO C comparison corpus; never invoked by ordinary tests."""
import hashlib,json,pathlib,struct,subprocess,sys,tempfile,zipfile
from extract_aac_ld_mps_prepare_facts import load
ROOT=pathlib.Path(__file__).resolve().parents[1]
OUT=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-tree-matrix'
def sha(b):return hashlib.sha256(b).hexdigest()
def main(archive):
    facts,provenance=load(archive)
    cases=[];data=bytearray()
    for tree,boxes,aux in ((0,5,[0,1,3,2]),(1,5,[0,1,3,4]),(5,2,[0,1]),(6,2,[0,1]),(7,1,[0])):
        for trial in range(16):
            for mask in range(1<<len(aux)):
                for active in ((False,True)if tree in (0,1)else(False,)):
                    parameters=[]
                    for b in range(boxes):
                        cld=(trial*7+b*11)%31-15;icc=(trial*3+b*5)%8
                        parameters.append(dict(cld=cld,icc=icc,residual=b in aux and bool(mask&(1<<aux.index(b)))))
                    cases.append(dict(tree=tree,lfe_active=active,parameters=parameters))
    data.extend(struct.pack('<I',len(cases)))
    for c in cases:
        data.extend(struct.pack('<III',c['tree'],c['lfe_active'],len(c['parameters'])))
        for p in c['parameters']:data.extend(struct.pack('<ddI',facts['dequantCLD'][p['cld']+15],facts['dequantICC'][p['icc']],p['residual']))
    with tempfile.TemporaryDirectory(prefix='fvid-tree-matrix-')as tmp:
        work=pathlib.Path(tmp);members={}
        with zipfile.ZipFile(archive)as z:
            for n in z.namelist():
                if n.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and n.endswith(('.c','.h')):
                    assert '..'not in pathlib.Path(n).parts
                    b=z.read(n);dest=work/n;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(b);members[n]=sha(b)
        src=work/'mp4spatialdec/src';(work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');driver=ROOT/'scripts/support/aac_ld_mps_tree_matrix_reference_probe.c';exe=work/'probe'
        args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver)]
        args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
        subprocess.run(args+['-lm','-o',str(exe)],check=True,capture_output=True,timeout=60)
        (work/'input.bin').write_bytes(data);subprocess.run([str(exe),str(work/'input.bin'),str(work/'output.bin')],check=True,timeout=60);expected=(work/'output.bin').read_bytes()
    offset=0
    for c in cases:
        inputs=6 if c['tree']in(5,6)else 1;outputs=2 if c['tree']==7 else 8 if inputs==6 else 6;columns=8 if inputs==6 else 2 if c['tree']==7 else 5
        c.update(inputs=inputs,outputs=outputs,columns=columns,offset=offset);offset+=(columns*inputs+2*outputs*columns)*8
    assert offset==len(expected)
    OUT.mkdir(parents=True,exist_ok=True);(OUT/'expected.bin').write_bytes(expected);(OUT/'manifest.json').write_text(json.dumps(dict(scope='OTT-only tree matrix component comparison, not spatial movie/PCM acceptance.',cases=cases),separators=(',',':'))+'\n')
    report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),generator_sha256=sha(pathlib.Path(__file__).read_bytes()),input_sha256=sha(data),expected_sha256=sha(expected),cases=len(cases),scope='Unmodified original ISO tree matrix routines; no production external implementation or ordinary-test compiler requirement.')
    (ROOT/'docs/validation/aac-ld-mps-tree-matrix-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print(len(cases),len(expected))
if __name__=='__main__':main(pathlib.Path(sys.argv[1]))
