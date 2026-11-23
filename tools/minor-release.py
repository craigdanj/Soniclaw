#!/usr/bin/env python3
"""Maintain, verify and package documented minor-release feature series."""
from pathlib import Path
import argparse,json,hashlib,zipfile,subprocess,time,re,os,datetime
ROOT=Path(__file__).resolve().parents[1]
EXCLUDE={'target','build','dist','vendor','node_modules','__pycache__','.git','validation-local'}
def source_files():
 for directory,children,files in os.walk(ROOT):
  children[:]=[name for name in children if name not in EXCLUDE]
  for name in files:yield Path(directory)/name

def run(args):
 version=args.version;series='workflow-features'if int(version.split('.')[1])>=60 else('studio-features'if int(version.split('.')[1])>=52 else('eight-features'if int(version.split('.')[1])>=44 else'twenty-features'));index={'workflow-features':'WORKFLOW_FEATURES','studio-features':'STUDIO_FEATURES','eight-features':'EIGHT_FEATURES','twenty-features':'TWENTY_FEATURES'}[series];entry=next(x for x in json.loads((ROOT/f'docs/{series}.json').read_text())if x['version']==version);tag=('v'+version.replace('.','_'))if int(version.split('.')[1])>=60 else('v'+''.join(version.split('.')[:2]));date=os.environ.get('SONICLAW_RELEASE_DATE',datetime.datetime.now(datetime.timezone.utc).date().isoformat())
 if args.action=='begin':
  old=re.search(r'version = "([0-9.]+)"',(ROOT/'Cargo.toml').read_text()).group(1)
  assert int(version.split('.')[1])==int(old.split('.')[1])+1,(old,version)
  for p in source_files():
   if not p.is_file()or (EXCLUDE|{'benchmarks','validation','third-party'}).intersection(p.relative_to(ROOT).parts):continue
   if p.suffix in {'.rs','.toml','.py','.iss'}or p.name=='CMakeLists.txt':
    s=p.read_text();t=s.replace(old,version)
    if s!=t:p.write_text(t)
  guide=entry['guide'];title=entry['title'];description=args.description
  assert (ROOT/f'docs/{guide}.md').exists(),'write guide first'
  p=ROOT/'README.md';s=p.read_text().replace('## Added in v','## Earlier additions in v');i=s.index('## Earlier additions in v');p.write_text(s[:i]+f'## Added in v{version}\n\n**{title}.** {description}\n\n[Feature guide](docs/{guide}.md) · [Release notes](docs/releases/v{version}.md) · [Verification](docs/validation/{tag}/REPORT.md)\n\n'+s[i:])
  p=ROOT/'docs/releases/README.md';p.write_text(p.read_text().replace('|---|---|---|',f'|---|---|---|\n| [{version}](v{version}.md) | {date} | {title} |',1))
  (ROOT/f'docs/releases/v{version}.md').write_text(f'# Soniclaw {version} — {title}\n\nRelease date: {date}.\n\n## Added\n\n{description}\n\nRead the [feature guide](../{guide}.md) for workflows, public API, runnable examples, lifecycle/thread ownership, persistence/migration, bounds, errors and recovery.\n\n## Compatibility\n\nWorkspace/internal package metadata advances to {version}. Existing product identities remain stable. This remains pre-1.0; feature-specific schema changes and dependencies are described in the guide.\n\n## Validation\n\n[Executed checks and limitations](../validation/{tag}/REPORT.md). A source release does not imply publication, installation or unexecuted platform/hardware/DAW acceptance.\n\nPart of the [feature series](../{index}.md).\n')
  p=ROOT/'CHANGELOG.md';s=p.read_text();i=s.index('\n## ');p.write_text(s[:i]+f'\n## [{version}] — {date}\n\n- {title}: {description}\n- Detailed API, migration and validation: `docs/{guide}.md`.\n'+s[i:])
  subprocess.run(['cargo','check','-p','soniclaw','--offline'],cwd=ROOT,check=True)
 elif args.action=='seal':
  out=ROOT/f'docs/validation/{tag}';out.mkdir(parents=True,exist_ok=True);checks=[]
  scope=['--workspace'] if not args.scoped else sum((['-p',p] for p in args.package), [])
  if not scope:raise ValueError('--scoped requires at least one --package')
  commands=[['cargo','fmt','--all','--check'],['cargo','clippy',*scope,'--all-targets','--all-features','--locked','--offline','--','-D','warnings'],['cargo','test',*(['--workspace']if not args.package else sum([['-p',p]for p in args.package],[])),'--all-features','--locked','--offline'],['cargo','doc',*scope,'--all-features','--no-deps','--locked','--offline']]
  for i,command in enumerate(commands):
   started=time.monotonic();env=os.environ.copy()
   if command[1]=='doc':env['RUSTDOCFLAGS']='-D warnings'
   with(out/f'{i+1}.log').open('w')as log:p=subprocess.run(command,cwd=ROOT,env=env,stdout=log,stderr=subprocess.STDOUT)
   checks.append(dict(command=command,exit_code=p.returncode,seconds=round(time.monotonic()-started,2),log=f'{i+1}.log'));(out/'checks.json').write_text(json.dumps(checks,indent=2)+'\n');print(checks[-1],flush=True)
   if p.returncode:raise SystemExit(p.returncode)
  count=sum(map(int,re.findall(r'test result: ok\. (\d+) passed;',(out/'3.log').read_text())))
  (out/'REPORT.md').write_text(f'# Soniclaw {version} verification\n\nExecuted {date} on Linux x86_64. {count} selected Rust tests passed. Formatting, all-target/all-feature Clippy and rustdoc passed with warnings denied within the scope recorded below. [Exact commands and timings](checks.json).\n\n{args.evidence}\n\nPhysical audio/MIDI, external DAW, Windows/macOS runtime and broad third-party product acceptance remain pending unless additional evidence explicitly records execution. This report is not a performance or JUCE-parity claim.\n\n[Feature guide](../../{entry["guide"]}.md).\n')
  items=json.loads((ROOT/f'docs/{series}.json').read_text());next(x for x in items if x['version']==version)['status']='Complete';(ROOT/f'docs/{series}.json').write_text(json.dumps(items,indent=2)+'\n')
  p=ROOT/f'docs/{index}.md';p.write_text(p.read_text().replace(f"| {version} | {entry['title']} | Planned |",f"| {version} | [{entry['title']}]({entry['guide']}.md) | Complete |"))
 elif args.action=='pack':
  files=sorted(p for p in source_files()if p.is_file()and not EXCLUDE.intersection(p.relative_to(ROOT).parts)and p.name!='SOURCE_MANIFEST.json')
  manifest={'name':'Soniclaw','version':version,'files_sha256':{str(p.relative_to(ROOT)):hashlib.sha256(p.read_bytes()).hexdigest()for p in files}};m=ROOT/'SOURCE_MANIFEST.json';m.write_text(json.dumps(manifest,indent=2)+'\n');files.append(m)
  dest=Path(args.output).resolve()if args.output else ROOT.parent.parent/f'soniclaw-rust-audio-framework-v{version}-source.zip'
  with zipfile.ZipFile(dest,'w',zipfile.ZIP_DEFLATED)as z:
   for p in files:z.write(p,'soniclaw/'+str(p.relative_to(ROOT)))
  with zipfile.ZipFile(dest)as z:
   assert z.testzip()is None
   for p,h in manifest['files_sha256'].items():assert hashlib.sha256(z.read('soniclaw/'+p)).hexdigest()==h
  print(json.dumps({'path':str(dest),'bytes':dest.stat().st_size,'files':len(files),'sha256':hashlib.sha256(dest.read_bytes()).hexdigest()}),flush=True)
if __name__=='__main__':
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('action',choices=['begin','seal','pack']);p.add_argument('version');p.add_argument('--description',default='');p.add_argument('--package',action='append');p.add_argument('--scoped',action='store_true');p.add_argument('--evidence',default='See feature-specific tests in the adjacent log.');p.add_argument('--output');run(p.parse_args())
