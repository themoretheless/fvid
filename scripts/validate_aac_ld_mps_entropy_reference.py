"""Explicit ISO entropy comparison, including a retained reference defect control.
No compiler, archive, process or network is required by ordinary Rust tests.
"""
import hashlib,json,struct,subprocess,sys,tempfile
from pathlib import Path
from zipfile import ZipFile
from extract_aac_ld_mps_huffman_facts import load
ROOT=Path(__file__).resolve().parents[1]
CORPUS=ROOT/'tests/fixtures/playback-errors/aac-ld-mps-entropy'
def sha(b):return hashlib.sha256(b).hexdigest()
def run(archive):
    assert sys.byteorder=='little','Research probe record format requires a little-endian host'
    tables,provenance=load(archive);assert tables==json.loads((CORPUS/'codewords.json').read_text())
    manifest=json.loads((CORPUS/'manifest.json').read_text());blob=(CORPUS/'packets.bin').read_bytes();records=bytearray();members={};reports=[]
    for c in manifest['cases']:
        n=len(c['history']);packet=blob[c['offset']:c['offset']+c['bytes']]+b'\x5a\xa5'
        records.extend(struct.pack('<8I',{'CLD':0,'ICC':1,'CPC':2}[c['kind']],c['coarse'],c['paired'],c['independent'],c['set_index'],n,len(packet),(23-n)//2));records.extend(struct.pack('<'+'i'*n,*c['history']));records.extend(packet)
    with tempfile.TemporaryDirectory(prefix='fvid-ld-mps-entropy-reference-')as tmp:
        work=Path(tmp)
        with ZipFile(archive)as z:
            for name in z.namelist():
                if name.startswith(('mp4spatialdec/src/','mp4spatialdec/include/','general/'))and name.endswith(('.c','.h')):
                    assert '..'not in Path(name).parts
                    data=z.read(name);p=work/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(data);members[name]=sha(data)
        src=work/'mp4spatialdec/src';original=(src/'sac_nlc_dec.c').read_bytes()
        before=b'int esc_data[MAXBANDS][2] = {{0}};';after=b'int esc_data[2][MAXBANDS] = {{0}};';assert original.count(before)==1
        corrected=original.replace(before,after);(src/'escape-control.c').write_bytes(corrected)
        (work/'libtsp.h').write_text('typedef struct AFILE AFILE;\n');(work/'cases.bin').write_bytes(records)
        driver=ROOT/'scripts/support/aac_ld_mps_entropy_reference_probe.c'
        for control in ['original','escape-shape-control']:
            exe=work/control;source=src/('sac_nlc_dec.c'if control=='original'else 'escape-control.c')
            args=['cc','-O2','-DLD_MPS','-ffunction-sections','-fdata-sections','-I',str(work),'-I',str(src),'-I',str(work/'mp4spatialdec/include'),str(driver),str(source),str(src/'sac_bitinput.c')]+[str(src/f'sac_nodes_{kind}.c')for kind in ['cld','icc','cpc','pt0']]
            args+=['-Wl,-dead_strip']if sys.platform=='darwin'else['-Wl,--gc-sections']
            subprocess.run(args+['-lm','-o',str(exe)],check=True,capture_output=True,timeout=60)
            result=subprocess.run([str(exe),str(work/'cases.bin')],check=True,capture_output=True,text=True,timeout=30);actual=[json.loads(line)for line in result.stdout.splitlines()];assert len(actual)==len(manifest['cases']);mismatches=[];matches=[]
            for c,a in zip(manifest['cases'],actual):
                assert a['success']==1 and a['bits']==c['bits']and a['history']==c['history'],(c['name'],a)
                if a['values']!=c['values']:mismatches.append(dict(name=c['name'],expected=c['values'],actual=a['values'],bits=a['bits']))
                else:matches.append(c['name'])
            expected={c['name']for c in manifest['cases']if c['reference_escape_overlap']}if control=='original'else set()
            assert {c['name']for c in mismatches}==expected,(control,mismatches[:2])
            reports.append(dict(control=control,matched=len(matches),mismatched=len(mismatches),mismatches=mismatches,stdout_sha256=sha(result.stdout.encode()),matched_names_sha256=sha(json.dumps(matches).encode())))
    report=dict(archive_sha256=provenance['archive_sha256'],reference_members=members,driver_sha256=sha(driver.read_bytes()),input_sha256=sha(records),cases=len(manifest['cases']),reference_escape_control=dict(original_sha256=sha(original),control_sha256=sha(corrected),before=before.decode(),after=after.decode(),scope='Research C comparison only. Original CLD escape scratch rows overlap for three or more escaped pairs. The original result is retained; production Rust uses its own vectors and grouped radix reconstruction.'),results=reports,scope='Parameter-pair entropy only. No spatial frame playback, upmix or seek acceptance claimed.')
    (ROOT/'docs/validation/aac-ld-mps-entropy-reference-2026-10-30.json').write_text(json.dumps(report,indent=2)+'\n');print([(r['control'],r['matched'],r['mismatched'])for r in reports])
if __name__=='__main__':run(Path(sys.argv[1]))
