#!/usr/bin/env python3
"""Protocol client qualification of Fvid's stdio and Streamable HTTP transports."""
import argparse, datetime, hashlib, http.client, json, os, pathlib, re, selectors, subprocess, tempfile, time
import validate_media as media
ROOT=pathlib.Path(__file__).resolve().parents[1]
parser=argparse.ArgumentParser();parser.add_argument('--binary',default=str(ROOT/'target/debug/fvid'));parser.add_argument('--gpu',choices=['metal','cuda','vulkan','dx12','gl']);parser.add_argument('--transport',choices=['stdio','http','both'],default='both');parser.add_argument('--jobs',type=int,choices=[1,2],default=1);args=parser.parse_args();binary=pathlib.Path(args.binary).resolve()
checks=[];sessions=[]
def record(name):checks.append(name);print(name,flush=True)
class Stdio:
    def __init__(self,root):
        self.log=tempfile.TemporaryFile();self.p=subprocess.Popen([binary,'mcp','--root',root,'--jobs',str(args.jobs),'--stdio'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log)
        self.buffer=b'';self.sel=selectors.DefaultSelector();self.sel.register(self.p.stdout,selectors.EVENT_READ)
    def send(self,value):self.p.stdin.write(json.dumps(value).encode()+b'\n');self.p.stdin.flush()
    def read(self):
        deadline=time.monotonic()+120
        while b'\n' not in self.buffer:
            if not self.sel.select(max(0,deadline-time.monotonic())):raise TimeoutError('MCP response timeout')
            chunk=os.read(self.p.stdout.fileno(),65536)
            if not chunk:self.log.seek(0);raise RuntimeError(self.log.read().decode())
            self.buffer+=chunk
        line,self.buffer=self.buffer.split(b'\n',1);return json.loads(line)
    def request(self,value):self.send(value);return self.read()
    def notify(self,value):self.send(value)
    def close(self):
        self.p.stdin.close()
        try:self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:self.p.kill();self.p.wait()
        self.p.stdout.close();self.sel.close();self.log.close()
class Http:
    TOKEN='fvid-integration-test-token-only'
    def __init__(self,root):
        self.log=tempfile.TemporaryFile();env=dict(os.environ,FVID_MCP_TOKEN=self.TOKEN)
        self.p=subprocess.Popen([binary,'mcp','--root',root,'--jobs',str(args.jobs),'--http','127.0.0.1:0'],stdout=subprocess.DEVNULL,stderr=self.log,env=env)
        deadline=time.monotonic()+20
        while True:
            self.log.seek(0);text=self.log.read().decode();match=re.search(r'http://127.0.0.1:(\d+)/mcp',text)
            if match:self.port=int(match[1]);break
            if self.p.poll() is not None or time.monotonic()>deadline:raise RuntimeError(text or 'HTTP startup failed')
            time.sleep(.02)
    def raw(self,value=None,method='POST',headers=None):
        h={'Authorization':'Bearer '+self.TOKEN,'Content-Type':'application/json','Accept':'application/json, text/event-stream','MCP-Protocol-Version':'2025-11-25'}
        if headers:h.update(headers)
        connection=http.client.HTTPConnection('127.0.0.1',self.port,timeout=120)
        connection.request(method,'/mcp',body=json.dumps(value) if value is not None else None,headers=h)
        response=connection.getresponse();body=response.read();status=response.status;connection.close();return status,body
    def request(self,value):
        status,body=self.raw(value);assert status==200,(status,body);return json.loads(body)
    def notify(self,value):status,body=self.raw(value);assert status==202,(status,body)
    def close(self):self.p.terminate();self.p.wait(timeout=10);self.log.close()
def rpc(method,params=None,id=1):
    value={'jsonrpc':'2.0','id':id,'method':method}
    if params is not None:value['params']=params
    return value
def call(client,name,arguments):
    reply=client.request(rpc('tools/call',{'name':name,'arguments':arguments}))
    assert 'error' not in reply,reply
    result=reply['result'];assert not result.get('isError'),result
    assert result['content'] and result['content'][0]['type']=='text'
    return result['structuredContent']
def rejected(client,name,arguments):
    reply=client.request(rpc('tools/call',{'name':name,'arguments':arguments}));assert 'error' in reply or reply['result'].get('isError'),reply
with tempfile.TemporaryDirectory(prefix='fvid-mcp-') as directory:
    w=pathlib.Path(directory);root=w/'root';root.mkdir();outside=w/'outside';outside.mkdir()
    source=root/'source.mp4';audio=root/'audio.wav';y4m=root/'source.y4m'
    media.ff('-f','lavfi','-i','testsrc2=size=128x72:rate=25','-t','2','-c:v','libx264','-bf','0','-g','25','-sc_threshold','0',source)
    media.ff('-f','lavfi','-i','sine=sample_rate=48000:frequency=997','-t','2','-c:a','pcm_s16le',audio)
    media.ff('-i',source,'-pix_fmt','yuv420p','-f','yuv4mpegpipe',y4m)
    (root/'playlist.m3u8').write_text('#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2,\n../outside/secret.ts\n#EXT-X-ENDLIST\n')
    (outside/'secret.wav').write_bytes(audio.read_bytes());(root/'outside-link').symlink_to(outside,target_is_directory=True)
    for transport in (['stdio','http'] if args.transport=='both' else [args.transport]):
        client=Stdio(root) if transport=='stdio' else Http(root)
        prefix=transport+'-'
        try:
            reply=client.request(rpc('initialize',{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'fvid-qualification-client','version':'1'}}))
            assert reply['result']['capabilities']['tools'] is not None
            assert reply['result']['serverInfo']['name']=='fvid'
            client.notify({'jsonrpc':'2.0','method':'notifications/initialized'})
            assert client.request(rpc('ping'))['result']=={}
            tools=client.request(rpc('tools/list'))['result']['tools'];assert len(tools)==11
            assert all(t['inputSchema']['additionalProperties'] is False for t in tools)
            record(prefix+'initialize, ping, 11 tool schemas')
            capabilities=call(client,'fvid_capabilities',{});assert 'library' in capabilities
            session={'transport':transport,'server':reply['result'],'tools':[tool['name'] for tool in tools],'library_version':capabilities['library']['library_version']}
            sessions.append(session)
            assert call(client,'fvid_devices',{})['cpu']
            assert call(client,'fvid_probe',{'input':'source.mp4'})['streams'][0]['codec']=='h264'
            record(prefix+'capabilities, devices, probe')
            output=prefix+'remux.mkv';call(client,'fvid_remux',{'input':'source.mp4','output':output});assert media.hashes(root/output)==media.hashes(source)
            output=prefix+'trim.mp4';call(client,'fvid_trim',{'input':'source.mp4','output':output,'from':'1','to':'2'});assert media.hashes(root/output)==media.hashes(source)[25:]
            output=prefix+'concat.mp4';call(client,'fvid_concat',{'inputs':['source.mp4','source.mp4'],'output':output});assert media.hashes(root/output)==media.hashes(source)*2
            record(prefix+'remux, strict trim, concat packet equality')
            output=prefix+'lossless.mkv';call(client,'fvid_transcode_lossless',{'input':'source.mp4','output':output,'crop':[2,2,64,48],'hflip':True});assert media.pixels(root/output)==media.pixels(source,'crop=64:48:2:2:exact=1,hflip')
            output=prefix+'h264.mp4';call(client,'fvid_transcode',{'input':'source.mp4','output':output,'encoder':'libx264','encoder_options':{'crf':'0','preset':'fast'}});assert media.pixels(root/output)==media.pixels(source)
            record(prefix+'lossless and explicit encoder pixel equality')
            output=prefix+'pcm.wav';call(client,'fvid_trim_pcm',{'input':'audio.wav','output':output,'from':'0.125','to':'0.525'});assert media.audio(root/output)==media.audio(audio)[12000:50400]
            output=prefix+'decoded.wav';call(client,'fvid_decode_audio',{'input':'audio.wav','output':output});assert media.audio(root/output)==media.audio(audio)
            record(prefix+'PCM trim and decode sample equality')
            output=prefix+'cpu.y4m';stats=call(client,'fvid_process_y4m',{'input':'source.y4m','output':output,'crop':[2,2,64,48],'vflip':True,'memory_mib':8});assert stats['stats']['backend']=='cpu';assert media.pixels(root/output)==media.pixels(source,'crop=64:48:2:2:exact=1,vflip')
            record(prefix+'Rust Y4M pixel equality')
            if args.gpu:
                output=prefix+'gpu.y4m';stats=call(client,'fvid_process_y4m',{'input':'source.y4m','output':output,'crop':[2,2,64,48],'hflip':True,'backend':args.gpu,'stages':[{'vflip':True}],'memory_mib':16})
                assert stats['stats']['backend']==args.gpu
                assert stats['stats']['transfers']['uploads']==50 and stats['stats']['transfers']['downloads']==50 and stats['stats']['transfers']['filter_passes']==100
                assert media.pixels(root/output)==media.pixels(source,'crop=64:48:2:2:exact=1,hflip,vflip')
                session['gpu_result']=stats
                record(prefix+args.gpu+' resident chain through MCP pixel equality')

            for name,arguments in [
                ('fvid_probe',{'input':'playlist.m3u8'}),
                ('fvid_probe',{'input':'../outside/secret.wav'}),('fvid_probe',{'input':'outside-link/secret.wav'}),
                ('fvid_remux',{'input':'audio.wav','output':'outside-link/escape.wav'}),
                ('fvid_remux',{'input':'audio.wav','output':'../outside/escape.wav'}),
                ('fvid_remux',{'input':'source.mp4','output':prefix+'remux.mkv'}),
                ('fvid_probe',{'input':'source.mp4','unexpected':True}),('fvid_probe',{}),
                ('fvid_transcode',{'input':'source.mp4','output':'bad.mkv','encoder':'libx264','encoder_options':{'stats':'/tmp/escape'}}),
                ('fvid_process_y4m',{'input':'source.y4m','output':'bad.y4m','memory_mib':0}),
                ('missing_tool',{}),
            ]:rejected(client,name,arguments)
            assert not (outside/'escape.wav').exists() and not (root/'bad.mkv').exists() and not (root/'bad.y4m').exists()
            assert not list(root.glob('.fvid-*.tmp'))
            record(prefix+'path escapes, symlinks, no-overwrite, malformed args, encoder option restrictions')
            if isinstance(client,Stdio):
                ids=list(range(31,32+args.jobs))
                for id in ids:client.send(rpc('tools/call',{'name':'fvid_transcode','arguments':{'input':'source.mp4','output':f'concurrent-{id}.mkv','encoder':'libx264','encoder_options':{'preset':'veryslow','crf':'0'}}},id))
                client.send(rpc('ping',id=99))
                replies=[client.read() for _ in range(len(ids)+1)]
                assert sorted(r['id'] for r in replies)==ids+[99]
                work=[r for r in replies if r['id']!=99]
                assert sum(bool(r.get('result',{}).get('isError')) for r in work)==1,replies
                for r in work:
                    if not r['result'].get('isError'):
                        assert media.ff('-i',root/f"concurrent-{r['id']}.mkv",'-f','rawvideo','-').stdout==media.ff('-i',source,'-f','rawvideo','-').stdout
                assert not next(r for r in replies if r['id']==99).get('error')
                record(f'stdio-{args.jobs} concurrent operations, excess busy, ping, exact output pixels')
                if args.jobs==2:
                    for id in [61,62]:client.send(rpc('tools/call',{'name':'fvid_transcode','arguments':{'input':'source.mp4','output':'collision.mkv','encoder':'libx264','encoder_options':{'preset':'veryslow','crf':'0'}}},id))
                    collision=[client.read(),client.read()]
                    assert sum(bool(r['result'].get('isError')) for r in collision)==1,collision
                    assert media.ff('-i',root/'collision.mkv','-f','rawvideo','-').stdout==media.ff('-i',source,'-f','rawvideo','-').stdout
                    assert not list(root.glob('.fvid-*.tmp'))
                    record('stdio-concurrent same-output publication has one winner and no partial file')
            else:
                assert client.raw(rpc('ping'),headers={'Authorization':'Bearer wrong'})[0]==401
                assert client.raw(rpc('ping'),headers={'Origin':'https://untrusted.example'})[0]==403
                assert client.raw(rpc('ping'),headers={'Host':'untrusted.example'})[0]==403
                assert client.raw(rpc('ping'),headers={'MCP-Protocol-Version':'not-a-version'})[0]==400
                record('http-authentication, Origin, Host and protocol version rejection')
        finally:client.close()
report={'created_utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'transport':args.transport,'jobs':args.jobs,'gpu':args.gpu,'sessions':sessions,'checks':checks,'status':'passed','binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'source_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in [*ROOT.joinpath('src').rglob('*.rs'),*ROOT.joinpath('crates/fvid-media/src').glob('*.rs'),ROOT/'Cargo.toml',ROOT/'Cargo.lock',ROOT/'crates/fvid-media/Cargo.toml',ROOT/'crates/fvid-media/build.rs',ROOT/'scripts/validate_media.py',pathlib.Path(__file__)]}}
(ROOT/f'benchmarks/mcp-validation-{args.transport}{"-jobs2" if args.jobs==2 else ""}{"-"+args.gpu if args.gpu else ""}.json').write_text(json.dumps(report,indent=2)+'\n');print('MCP validation passed',flush=True)
