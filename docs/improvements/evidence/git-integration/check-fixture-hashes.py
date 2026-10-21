import hashlib,json
from pathlib import Path
root=Path('tests/fixtures/playback-errors');cache={};checked=0;errors=[];parse=[]
def digest(p):
 if p not in cache:cache[p]=hashlib.sha256(p.read_bytes()).hexdigest()
 return cache[p]
def visit(o,base,manifest):
 global checked
 if isinstance(o,dict):
  for filekey,hashkey in [('file','sha256'),('reference','reference_sha256'),('webm','webm_sha256')]:
   name=o.get(filekey);want=o.get(hashkey)
   if isinstance(name,str) and isinstance(want,str) and len(want)==64:
    p=base/name
    if not p.is_file() and filekey=='file' and isinstance(o.get('source'),str):p=base/o['source']
    if not p.is_file():
     if (root/name).is_file():p=root/name
     else:errors.append({'manifest':str(manifest),'file':name,'error':'missing file'});continue
    checked+=1;got=digest(p)
    if got!=want:errors.append({'manifest':str(manifest),'file':name,'expected':want,'actual':got})
  for v in o.values():visit(v,base,manifest)
 elif isinstance(o,list):
  for v in o:visit(v,base,manifest)
for p in root.rglob('*.json'):
 try:obj=json.loads(p.read_bytes())
 except Exception as e:parse.append({'manifest':str(p),'error':str(e)});continue
 visit(obj,p.parent,p)
report={'hash_checks':checked,'unique_files_hashed':len(cache),'hash_or_missing_errors':errors,'json_errors':parse}
Path('/private/tmp/fvid-fixture-hash-audit.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps({'hash_checks':checked,'unique_files':len(cache),'errors':len(errors),'json_errors':len(parse)}))
