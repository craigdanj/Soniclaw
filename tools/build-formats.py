#!/usr/bin/env python3
"""Build the shared Rust processor as CLAP and statically wrapped VST3/AUv2.
Never installs plug-ins into user folders or signs artifacts implicitly.
"""
import argparse,json,os,pathlib,platform,shutil,subprocess,sys
ROOT=pathlib.Path(__file__).resolve().parents[1]
def run(args,**kwargs):subprocess.run(list(map(str,args)),check=True,**kwargs)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--protocol-fixture',action='store_true',help='Build independent Linux musical host fixture');p.add_argument('--fetch',action='store_true',help='Fetch the explicitly pinned upstream sources');p.add_argument('--deps',type=pathlib.Path,default=ROOT/'vendor/formats');p.add_argument('--build-dir',type=pathlib.Path,default=ROOT/'build/formats');p.add_argument('--au',action='store_true');p.add_argument('--no-editor',action='store_true');p.add_argument('--jobs',type=int,default=2);args=p.parse_args()
 if args.au and sys.platform!='darwin':p.error('AUv2 requires macOS and Xcode command-line tools')
 if args.jobs<1:p.error('--jobs must be positive')
 deps=args.deps.resolve();build=args.build_dir.resolve();lock=json.loads((ROOT/'formats/dependencies.json').read_text());identities={}
 for name,item in lock.items():
  if item.get('macos_only') and not args.au:continue
  path=deps/name
  if not path.exists():
   if not args.fetch:p.error(f'missing {path}; pass --fetch to retrieve the pinned sources')
   path.parent.mkdir(parents=True,exist_ok=True);run(['git','init',path]);run(['git','-C',path,'remote','add','origin',item['url']]);run(['git','-C',path,'fetch','--depth','1','origin',item['revision']]);run(['git','-C',path,'checkout','--detach','FETCH_HEAD'])
  actual=subprocess.check_output(['git','-C',path,'rev-parse','HEAD'],text=True).strip();expected=subprocess.check_output(['git','-C',path,'rev-parse',item['revision']+'^{commit}'],text=True).strip() if not all(c in '0123456789abcdef' for c in item['revision']) else item['revision']
  if actual!=expected:raise RuntimeError(f'{name}: checkout does not match pinned revision')
  if item.get('submodules') and args.fetch:run(['git','-C',path,'submodule','update','--init','--depth','1',*item['submodules']])
  identities[name]=actual
 cargo=shutil.which('cargo') or pathlib.Path.home()/'.cargo/bin/cargo';cmake=shutil.which('cmake') or pathlib.Path.home()/'.local/bin/cmake'
 cmd=[cargo,'build','--release','-p','soniclaw-reference'];
 if args.no_editor:cmd+=['--no-default-features']
 run(cmd,cwd=ROOT)
 target=pathlib.Path(os.environ.get('CARGO_TARGET_DIR',ROOT/'target'));target=target if target.is_absolute() else ROOT/target
 lib=target/'release'/('soniclaw_reference.lib' if sys.platform=='win32' else 'libsoniclaw_reference.a')
 run([cmake,'-S',ROOT/'formats','-B',build,'-DCMAKE_BUILD_TYPE=Release',f'-DSONICLAW_DEPS={deps}',f'-DSONICLAW_STATIC_LIBRARY={lib}',f'-DSONICLAW_BUILD_PROTOCOL_FIXTURE={"ON" if args.protocol_fixture else "OFF"}',f'-DSONICLAW_BUILD_AU={"ON" if args.au else "OFF"}'])
 run([cmake,'--build',build,'--config','Release','--parallel',args.jobs])
 (build/'soniclaw-format-build.json').write_text(json.dumps({'system':platform.platform(),'upstreams':identities,'editor':not args.no_editor,'au_requested':args.au},indent=2)+'\n')
 print(f'Built format artifacts under {build}; install only after validation.')
if __name__=='__main__':main()
