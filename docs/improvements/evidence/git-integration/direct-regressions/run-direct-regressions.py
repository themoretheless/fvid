from pathlib import Path
import subprocess,os,json,hashlib,time
root=Path.cwd();out=Path('/private/tmp/fvid-integration-tests');out.mkdir(exist_ok=True)
rustc=Path('/Users/themoretheless/.rustup/toolchains/1.98.1-aarch64-apple-darwin/bin/rustc')
lib=root/'target/debug/deps/libfvid-8617aa0177c95e51.rlib';serde=root/'target/debug/deps/libserde_json-0e748d5774ba5ef5.rlib'
assert lib.exists() and serde.exists()
records=[]
for name in ['mp4_signed_offset','mp4_fragment_description','av1_chroma_grain','av1_lossy_palette','av1_superres_segmentation','av1_tile_list','av1_separate_frame','av1_model_trace']:
 cmd=[str(rustc),'--edition=2024','--test',str(root/'tests'/f'{name}.rs'),'--crate-name',name,'--cfg','feature="player"','-L','dependency='+str(root/'target/debug/deps'),'--extern','fvid='+str(lib),'--extern','serde_json='+str(serde),'-o',str(out/name)]
 env={**os.environ,'CARGO_MANIFEST_DIR':str(root)};start=time.time()
 with (out/f'{name}-compile.log').open('wb') as log:r=subprocess.run(cmd,env=env,stdout=log,stderr=subprocess.STDOUT)
 row={'target':name,'command':cmd,'compile_exit':r.returncode}
 if r.returncode==0:
  with (out/f'{name}-test.log').open('wb') as log:r=subprocess.run([str(out/name),'--nocapture'],env=env,stdout=log,stderr=subprocess.STDOUT)
  row['test_exit']=r.returncode
 records.append(row);(out/'results.json').write_text(json.dumps(records,indent=2)+'\n');print(name,row,flush=True)
 if r.returncode:break
