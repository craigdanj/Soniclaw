#!/usr/bin/env python3
"""Collect dependency attribution and supplied license/notice files for binaries."""
import argparse,json,os,pathlib,shutil,subprocess
ROOT=pathlib.Path(__file__).resolve().parents[1]
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--output',type=pathlib.Path,default=ROOT/'dist/licenses');p.add_argument('--format-deps',type=pathlib.Path);args=p.parse_args();args.output.mkdir(parents=True,exist_ok=True)
 cargo=shutil.which('cargo')or pathlib.Path.home()/'.cargo/bin/cargo';metadata=json.loads(subprocess.check_output([cargo,'metadata','--locked','--format-version','1'],cwd=ROOT,text=True));members=set(metadata['workspace_members']);index=[]
 for package in metadata['packages']:
  if package['id']in members:continue
  source=pathlib.Path(package['manifest_path']).parent;dest=args.output/(package['name']+'-'+package['version']);files=[]
  for folder,dirs,names in os.walk(source):
   dirs[:]=[d for d in dirs if d not in ['.git','target','node_modules']]
   for name in names:
    if any(token in name.lower()for token in ['license','copying','copyright','notice','ofl.txt']):
     f=pathlib.Path(folder)/name
     if f.suffix.lower()in ['.rs','.c','.cpp','.h','.py','.png','.ttf','.otf']:continue
     rel=f.relative_to(source);out=dest/rel;out.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(f,out);files.append(str(rel))
  index.append({'name':package['name'],'version':package['version'],'license':package['license'],'authors':package['authors'],'repository':package['repository'],'supplied_notice_files':files})
 if args.format_deps:
  for name in ['clap-wrapper','clap','vst3sdk','AudioUnitSDK']:
   source=args.format_deps/name
   if not source.exists():continue
   for f in source.glob('*'):
    if f.is_file()and any(x in f.name.lower()for x in ['license','notice','copying']):
     target=args.output/name/f.name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(f,target)
   # SDK submodules and vendored fmt/psl retain their notices too.
   for sub in ['base','pluginterfaces','public.sdk','libs/fmt','libs/psl']:
    folder=source/sub
    if folder.exists():
     for f in folder.glob('*'):
      if f.is_file()and any(x in f.name.lower()for x in ['license','notice','copying']):
       target=args.output/name/sub/f.name;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(f,target)
 (args.output/'dependencies.json').write_text(json.dumps(index,indent=2)+'\n');print(f'Collected supplied attribution for {len(index)} Rust packages in {args.output}')
if __name__=='__main__':main()
