#!/usr/bin/env python3
"""Fetch selected public ISO LD-MPS references; never used by ordinary tests.

Uses bounded HTTP ranges, so the 9 GiB WAV archive is not downloaded whole.
Output stays in the explicitly supplied directory, outside synthetic fixtures.
The original ZIP member paths contain ../; they are never passed to extract().
"""
import io,urllib.request,zipfile,json,pathlib
BASE='https://standards.iso.org/iso-iec/14496/-26/ed-2/en/'
class Remote(io.RawIOBase):
 def __init__(self,name):
  self.url=BASE+name;self.pos=0;self.total=0
  req=urllib.request.Request(self.url,method='HEAD')
  with urllib.request.urlopen(req,timeout=30) as r:self.size=int(r.headers['Content-Length'])
 def seekable(self):return True
 def readable(self):return True
 def tell(self):return self.pos
 def seek(self,n,w=0):
  self.pos=n if w==0 else self.pos+n if w==1 else self.size+n
  if self.pos<0:raise ValueError('negative seek')
  return self.pos
 def read(self,n=-1):
  n=min(n if n>=0 else self.size-self.pos,self.size-self.pos)
  if n==0:return b''
  if n>32*1024*1024 or self.total+n>96*1024*1024:raise ValueError('range admission exceeded')
  start=self.pos;end=start+n-1
  req=urllib.request.Request(self.url,headers={'Range':f'bytes={start}-{end}','Accept-Encoding':'identity'})
  with urllib.request.urlopen(req,timeout=45) as r:
   if r.status!=206 or r.headers.get('Content-Range')!=f'bytes {start}-{end}/{self.size}':raise ValueError('invalid partial response')
   d=r.read(n+1)
  if len(d)!=n:raise ValueError('wrong response length')
  self.total+=n;self.pos+=n;return d

def main():
 import argparse,hashlib
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('output',type=pathlib.Path)
 args=parser.parse_args()
 args.output.mkdir(parents=True,exist_ok=True)
 checksum_name='mpeg4audio-conformance2ndEd_20230125.md5'
 with urllib.request.urlopen(BASE+checksum_name,timeout=30) as response:
  checksum=response.read(2*1024*1024)
 expected='865521883bbc19db177c71c00fcb57bd3619a8ae73a05a21532c3fc22cc13e92'
 if hashlib.sha256(checksum).hexdigest()!=expected:
  raise ValueError('ISO checksum inventory changed; inspect it before updating the pin')
 sums={}
 for line in checksum.decode().splitlines():
  digest,path=line.split(None,1);sums[path.lstrip('./')]=digest
 names={'er_ad_ldmps_tree_5151','er_eld_ldmps_tree_5151','er_eld_ldmps_ldqmf_1','er_eld_ldmps_ldqmf_2'}
 report=[]
 for archive,folder,ext in [('compressedMp4.zip','compressedMp4','.mp4'),('referencesWav.zip','referencesWav','.wav')]:
  remote=Remote(archive)
  found=set()
  with zipfile.ZipFile(remote) as z:
   for info in z.infolist():
    basename=pathlib.PurePosixPath(info.filename).name
    if basename not in {n+ext for n in names}:continue
    if basename in found or info.file_size>32*1024*1024:
     raise ValueError('duplicate or oversized conformance member')
    found.add(basename)
    data=z.read(info)
    digest=hashlib.md5(data).hexdigest()
    if digest!=sums[folder+'/'+basename]:raise ValueError('ISO member checksum mismatch')
    (args.output/basename).write_bytes(data)
    report.append(dict(url=remote.url,member=info.filename,bytes=len(data),sha256=hashlib.sha256(data).hexdigest(),iso_md5=digest))
    print(basename,len(data),'verified',flush=True)
  if found!={n+ext for n in names}:raise ValueError('missing requested conformance members')
 (args.output/'provenance.json').write_text(json.dumps(dict(checksum_url=BASE+checksum_name,checksum_sha256=expected,files=report),indent=2)+'\n')

if __name__=='__main__':main()
