"""Explicit metadata-only check against the frozen official ISO insert.
Ordinary tests never invoke this compiler/reference validation workflow.
"""
import hashlib,json,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from io import BytesIO
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-config'
def sha(data):return hashlib.sha256(data).hexdigest()
def run(archive):
    known=json.loads((ROOT/'docs/validation/aac-ld-qmf-source-2026-10-30.json').read_text());archive_hash=sha(archive.read_bytes());assert archive_hash==known['archive_sha256']
    manifest=json.loads((CORPUS/'manifest.json').read_text());members={};rows=[]
    with tempfile.TemporaryDirectory(prefix='fvid-ld-mps-config-reference-')as tmp:
        work=Path(tmp)
        with ZipFile(archive)as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
                    assert '..'not in Path(name).parts
                    data=z.read(name);dest=work/name;dest.parent.mkdir(parents=True,exist_ok=True);dest.write_bytes(data);members[name]=sha(data)
            with ZipFile(BytesIO(z.read('audio.zip')))as a:
                name='audio/natural/mp4AudVm_Rewrite/src_frame/flex_mux.c';members['audio.zip:'+name]=sha(a.read(name))
        # No audio I/O is used. Only the pointer type is required by the ISO
        # decoder's aggregate structure declaration; source files stay unchanged.
        (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n')
        driver=ROOT/'scripts/support/aac_ld_mps_config_reference_probe.c';exe=work/'probe';src=work/'mp4spatialdec/src'
        args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(src/'sac_bitdec.c'),str(src/'sac_bitinput.c')]
        args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
        subprocess.run(args+['-lm','-o',str(exe)],check=True,timeout=60,capture_output=True)
        for row in manifest['cases']:
            r=subprocess.run([str(exe),str(CORPUS/row['file'])],capture_output=True,text=True,check=True,timeout=5);actual=json.loads(r.stdout);assert actual==row['expected'],(row['name'],actual,row['expected']);rows.append(dict(name=row['name'],payload_sha256=sha((CORPUS/row['file']).read_bytes()),metadata_sha256=sha(json.dumps(actual,sort_keys=True).encode())))
    report=dict(archive_sha256=archive_hash,reference_members=members,driver_sha256=sha(driver.read_bytes()),checks=len(rows),scope='Configuration metadata only; no spatial frame/PCM/playback acceptance claimed.',alignment_evidence='ISO audio.zip flex_mux.c reads length bytes of ELD extension payload into a separate byte array. Spatial configuration alignment is relative to that byte-array origin.',results=rows)
    (ROOT/'docs/validation/aac-ld-mps-config-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print('Matched',len(rows),'configuration metadata records against unmodified ISO reference sources.')
if __name__=='__main__':run(Path(sys.argv[1]))
