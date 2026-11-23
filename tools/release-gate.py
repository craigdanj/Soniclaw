#!/usr/bin/env python3
"""Audit a staged native release without executing/installing its payload."""
from __future__ import annotations
import argparse,datetime,hashlib,importlib.util,json,platform,re,struct,sys
from pathlib import Path,PurePosixPath
ROOT=Path(__file__).resolve().parents[1]
ARCH={62:'x86_64',183:'aarch64',0x8664:'x86_64',0xaa64:'aarch64',0x01000007:'x86_64',0x0100000c:'aarch64'}
def digest(path):
 h=hashlib.sha256()
 with Path(path).open('rb')as f:
  for block in iter(lambda:f.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
def native_image(path):
 with Path(path).open('rb')as f:head=f.read(1024*1024)
 if head.startswith(b'\x7fELF'):
  if len(head)<64 or head[4]!=2 or head[5]not in(1,2):raise ValueError('unsupported/truncated ELF header')
  machine=struct.unpack_from('<H'if head[5]==1 else'>H',head,18)[0]
  if machine not in(62,183):raise ValueError('unsupported ELF machine')
  return {'platform':'linux','architectures':[ARCH[machine]],'format':'ELF64'}
 if head.startswith(b'MZ'):
  if len(head)<64:raise ValueError('truncated DOS header')
  off=struct.unpack_from('<I',head,60)[0]
  if off<64 or off+24>len(head)or head[off:off+4]!=b'PE\0\0':raise ValueError('invalid/truncated PE header')
  machine=struct.unpack_from('<H',head,off+4)[0]
  if machine not in(0x8664,0xaa64):raise ValueError('unsupported PE machine')
  return {'platform':'windows','architectures':[ARCH[machine]],'format':'PE'}
 magic=head[:4]
 if magic in(b'\xcf\xfa\xed\xfe',b'\xfe\xed\xfa\xcf'):
  if len(head)<32:raise ValueError('truncated Mach-O header')
  cpu=struct.unpack_from('<I'if magic[0]==0xcf else'>I',head,4)[0]
  if cpu not in(0x01000007,0x0100000c):raise ValueError('unsupported Mach-O CPU')
  return {'platform':'macos','architectures':[ARCH[cpu]],'format':'Mach-O64'}
 if magic in(b'\xca\xfe\xba\xbe',b'\xca\xfe\xba\xbf'):
  if len(head)<8:raise ValueError('truncated universal Mach-O')
  count=struct.unpack_from('>I',head,4)[0];entry=32 if magic[-1]==0xbf else 20
  if not 1<=count<=8 or len(head)<8+count*entry:raise ValueError('invalid universal architecture table')
  cpus=[struct.unpack_from('>I',head,8+i*entry)[0]for i in range(count)]
  if any(c not in(0x01000007,0x0100000c)for c in cpus)or len(set(cpus))!=len(cpus):raise ValueError('unsupported/duplicate universal CPU')
  return {'platform':'macos','architectures':sorted(ARCH[c]for c in cpus),'format':'Mach-O universal'}
 return None

def safe_path(root,relative):
 p=PurePosixPath(relative)
 if not relative or '\\'in relative or p.is_absolute()or any(x in('..','.')for x in p.parts):raise ValueError('unsafe manifest path')
 path=root.joinpath(*p.parts)
 if not path.is_file()or not path.resolve().is_relative_to(root.resolve()):raise ValueError('missing/escaped payload path')
 return path

def verify_stage(root,expected_platform,architecture):
 root=Path(root);manifest=json.loads((root/'manifest.json').read_text());version=manifest.get('version')
 if manifest.get('schema')!=2 or not re.fullmatch(r'\d+\.\d+\.\d+',str(version)):raise ValueError('requires complete schema2 release manifest and release version')
 declared={'darwin':'macos','win32':'windows'}.get(manifest.get('platform'),manifest.get('platform'))
 if declared!=expected_platform:raise ValueError('manifest platform mismatch')
 records=manifest.get('files_sha256');
 if not isinstance(records,dict)or not 1<=len(records)<=20000:raise ValueError('invalid release file inventory')
 files={str(p.relative_to(root)).replace('\\','/')for p in root.rglob('*')if p.is_file()and p!=root/'manifest.json'}
 if files!=set(records):raise ValueError('release inventory mismatch')
 images=[]
 for relative,sha in records.items():
  path=safe_path(root,relative)
  if not re.fullmatch('[0-9a-f]{64}',str(sha))or digest(path)!=sha:raise ValueError('payload checksum mismatch: '+relative)
  image=native_image(path)
  if image:
   expected={'x86_64','aarch64'}if architecture=='universal2'else{architecture}
   if image['platform']!=expected_platform or not expected.issubset(image['architectures']):raise ValueError('native platform/architecture mismatch: '+relative)
   images.append({'path':relative,'sha256':sha,**image})
 if not images:raise ValueError('release has no recognized native payload')
 for name in('LICENSE-MIT','LICENSE-APACHE','THIRD_PARTY.md'):
  if name not in records:raise ValueError('required license/notice missing: '+name)
 if manifest.get('helper'):
  helper=manifest['helper']
  if not any(i['path']==helper for i in images):raise ValueError('declared helper is absent/not a matching native image')
 return {'version':version,'files':len(records),'manifest_sha256':digest(root/'manifest.json'),'images':images,'signed_claim':bool(manifest.get('signed')),'signature_verified':False}

def acceptance(doc,manifest_sha,expected_platform):
 spec=importlib.util.spec_from_file_location('soniclaw_compat',ROOT/'tools/compatibility-suite.py');module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
 result=module.validate_acceptance(doc)
 if doc.get('platform')!=expected_platform:raise ValueError('acceptance platform mismatch')
 if doc.get('release_manifest_sha256')!=manifest_sha:raise ValueError('acceptance does not attest this exact release manifest')
 if result['status']=='passed':
  stamp=datetime.datetime.fromisoformat(doc['tested_at'].replace('Z','+00:00'))
  if stamp.tzinfo is None:raise ValueError('acceptance timestamp needs explicit timezone')
 return result

def main(argv=None):
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('stage',type=Path);p.add_argument('--platform',choices=['linux','macos','windows'],required=True);p.add_argument('--architecture',choices=['x86_64','aarch64','universal2'],required=True);p.add_argument('--acceptance',type=Path);p.add_argument('--require-acceptance',action='store_true');p.add_argument('--report',type=Path,required=True);a=p.parse_args(argv)
 report={'schema':1,'environment':platform.platform(),'automated_status':'failed','acceptance_status':'pending'};code=1
 try:
  report['audit']=verify_stage(a.stage,a.platform,a.architecture);report['automated_status']='passed';code=0
  if a.acceptance:report['acceptance']=acceptance(json.loads(a.acceptance.read_text()),report['audit']['manifest_sha256'],a.platform);report['acceptance_status']=report['acceptance']['status']
  if report['acceptance_status']=='failed':code=1
  elif a.require_acceptance and report['acceptance_status']!='passed':code=2
 except Exception as e:
  report['error']=str(e);code=1
  if report['automated_status']=='passed'and a.acceptance:report['acceptance_status']='failed'
 a.report.parent.mkdir(parents=True,exist_ok=True);a.report.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report));return code
if __name__=='__main__':raise SystemExit(main())
