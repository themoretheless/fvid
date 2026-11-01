#!/usr/bin/env python3
"""Explicit offline AAC LD-QMF synthesis phase check against official sources.

No production linkage or fixture generation from a foreign decoder. Archived
sources remain unmodified; a separate PI control isolates constant rounding.
"""
import argparse,hashlib,json,math,struct,subprocess,sys
from io import BytesIO
from pathlib import Path
from zipfile import ZipFile
ROOT=Path(__file__).resolve().parents[1]
PIN='b6b27e2f9af6ee8f950af8d7074afc467e66d93372328c0ec55282fff71d1f83'
def sha(data):return hashlib.sha256(data).hexdigest()
def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--source-archive',type=Path,required=True);p.add_argument('--work-dir',type=Path,required=True);p.add_argument('--report',type=Path,required=True);a=p.parse_args()
    archive=a.source_archive.read_bytes();assert sha(archive)==PIN,'reference archive pin mismatch'
    work=a.work_dir.resolve();assert not work.is_relative_to(ROOT/'tests/fixtures');work.mkdir(parents=True,exist_ok=True)
    members={}
    with ZipFile(BytesIO(archive))as z,ZipFile(BytesIO(z.read('audio.zip')))as audio:
        prefix='audio/natural/mp4AudVm_Rewrite/src_tf/'
        for name in['ct_polyphase.c','ct_polyphase.h','ct_sbrconst.h','ct_sbrdecoder.h']:
            data=audio.read(prefix+name);(work/name).write_bytes(data);members[prefix+name]=sha(data)
    (work/'malloc.h').write_text('#include <stdlib.h>\n')
    driver=ROOT/'scripts/support/aac_ld_sbr_synthesis_reference_probe.c';outputs={}
    for precise in[False,True]:
        label='precise'if precise else'original';exe=work/label;output=work/(label+'.bin')
        command=['cc','-O2','-DAAC_ELD','-ffunction-sections','-fdata-sections','-I',str(work),str(driver)]
        if precise:command+=['-DFVID_PRECISE_PI']
        command+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
        subprocess.run(command+['-lm','-o',str(exe)],capture_output=True,check=True)
        subprocess.run([str(exe),str(output)],check=True);outputs[label]=[v[0]for v in struct.iter_unpack('<d',output.read_bytes())]
    model=[];wrong=[]
    for bands in[64,32]:
        shift=31 if bands==32 else 255
        for n in range(2*bands):
            for k in range(bands):
                phase=math.pi/(2*bands)*(k+.5)*(2*n-shift);old=math.pi/(2*bands)*(k+.5)*(2*n-4*bands+1)
                model.extend([math.cos(phase)/64,math.sin(phase)/64]);wrong.extend([math.cos(old)/64,math.sin(old)/64])
    assert len(model)==len(outputs['precise'])==20480
    maximum=max(abs(a-b)for a,b in zip(model,outputs['precise']));assert maximum<1e-13
    start=16384
    report=dict(archive_sha256=PIN,reference_members=members,driver_sha256=sha(driver.read_bytes()),validator_sha256=sha(Path(__file__).read_bytes()),coefficients=len(model),precise_formula_max_error=maximum,original_pi_max_error=max(abs(a-b)for a,b in zip(model,outputs['original'])),old_generic32_max_error=max(abs(a-b)for a,b in zip(wrong[start:],outputs['precise'][start:])),dual_rate_control_max_error=max(abs(a-b)for a,b in zip(wrong[:start],outputs['precise'][:start])),scope='Original AAC InitSbrSynFilterbank FBMODE_LDQMF: single-rate32 N uses2n-31;dual-rate64 uses2n-255. Separate PI control only;source files unchanged. This is not the generic standalone MPS32 PCM bank convention.')
    a.report.write_text(json.dumps(report,indent=2)+'\n');print(report)
if __name__=='__main__':main()
