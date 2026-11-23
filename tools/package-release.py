#!/usr/bin/env python3
"""Stage an existing plugin, checksum it, and optionally make a native installer.
No plugin-folder installation, network publishing or automatic signing occurs.
"""
import argparse,hashlib,json,pathlib,platform,shutil,subprocess,sys,zipfile
ROOT=pathlib.Path(__file__).resolve().parents[1]
def run(args):subprocess.run(list(map(str,args)),check=True)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('artifact',type=pathlib.Path);p.add_argument('--output',type=pathlib.Path,default=ROOT/'dist/releases');p.add_argument('--version',default='0.146.0');p.add_argument('--name',default='SoniclawReference');p.add_argument('--installer',action='store_true');p.add_argument('--helper',type=pathlib.Path,help='Explicit matching isolation helper to include in this developer release');p.add_argument('--licenses',type=pathlib.Path,help='Directory produced by collect-licenses.py');p.add_argument('--sign-identity',help='macOS codesign identity (explicit opt-in)');args=p.parse_args()
 if not args.artifact.exists():p.error('artifact does not exist')
 for value in [args.name,args.version]:
  if not value or not all(c.isalnum()or c in '.-_' for c in value)or value in ['.','..']:p.error('name/version must be a plain safe filename component')
 suffix=args.artifact.suffix
 if suffix not in ['.clap','.vst3','.component']:p.error('pass a built .clap, .vst3 or .component artifact')
 args.output.mkdir(parents=True,exist_ok=True);release=args.output/f'{args.name}-{args.version}-{sys.platform}-{platform.machine()}'
 if release.exists():p.error(f'{release} already exists; choose another output')
 release.mkdir();staged=release/args.artifact.name
 if args.artifact.is_dir():shutil.copytree(args.artifact,staged)
 else:shutil.copy2(args.artifact,staged)
 for name in ['LICENSE-MIT','LICENSE-APACHE']:shutil.copy2(ROOT/name,release/name)
 shutil.copy2(ROOT/'docs/THIRD_PARTY.md',release/'THIRD_PARTY.md')
 if args.licenses:shutil.copytree(args.licenses,release/'licenses')
 if args.sign_identity:
  if sys.platform!='darwin':p.error('--sign-identity requires macOS')
  run(['codesign','--force','--options','runtime','--timestamp','--sign',args.sign_identity,staged]);run(['codesign','--verify','--strict',staged])
 helper=None
 if args.helper:
  if not args.helper.is_file():p.error('--helper must be a built native executable')
  helper='vst3-host-helper'+('.exe'if sys.platform=='win32'else'');shutil.copy2(args.helper,release/helper)
 hashes={str(f.relative_to(release)).replace('\\','/'):hashlib.sha256(f.read_bytes()).hexdigest()for f in release.rglob('*')if f.is_file()}
 (release/'manifest.json').write_text(json.dumps({'schema':2,'name':args.name,'version':args.version,'platform':sys.platform,'machine':platform.machine(),'signed':bool(args.sign_identity),'helper':helper,'files_sha256':hashes},indent=2)+'\n')
 archive=str(release)+'.zip'
 with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED,strict_timestamps=False) as z:
  for file in release.rglob('*'):
   if file.is_file():z.write(file,file.relative_to(release.parent))
 print(archive)
 if args.installer:
  if sys.platform=='linux':
   if platform.machine()not in ['x86_64','aarch64']:p.error('no Debian architecture mapping for this host')
   package=release/'deb-root';control=package/'DEBIAN';control.mkdir(parents=True);plugin_dir=package/'usr/lib'/('clap' if suffix=='.clap' else 'vst3' if suffix=='.vst3' else 'unsupported')
   if suffix=='.component':p.error('AU components cannot be packaged for Linux')
   plugin_dir.mkdir(parents=True);target=plugin_dir/staged.name
   if staged.is_dir():shutil.copytree(staged,target)
   else:shutil.copy2(staged,target)
   architecture='amd64'if platform.machine()=='x86_64'else'arm64';(control/'control').write_text(f'Package: soniclaw-{args.name.lower().replace("_","-")}\nVersion: {args.version}\nArchitecture: {architecture}\nMaintainer: Soniclaw contributors\nSection: sound\nPriority: optional\nDescription: Soniclaw reference audio plug-in\n')
   docs=package/'usr/share/doc'/('soniclaw-'+args.name.lower().replace('_','-'));docs.mkdir(parents=True)
   for name in ['LICENSE-MIT','LICENSE-APACHE','THIRD_PARTY.md']:shutil.copy2(release/name,docs/name)
   if args.licenses:shutil.copytree(args.licenses,docs/'licenses')
   run(['dpkg-deb','--root-owner-group','--build',package,args.output/(release.name+'.deb')])
  elif sys.platform=='darwin':
   root=release/'pkg-root';folder=root/'Library/Audio/Plug-Ins'/({'.clap':'CLAP','.vst3':'VST3','.component':'Components'}[suffix]);folder.mkdir(parents=True)
   if staged.is_dir():shutil.copytree(staged,folder/staged.name)
   else:shutil.copy2(staged,folder/staged.name)
   run(['pkgbuild','--root',root,'--identifier',f'org.soniclaw.{args.name.lower()}','--version',args.version,args.output/(release.name+'.pkg')])
  else:p.error('Windows installer: use packaging/windows/Soniclaw.iss with Inno Setup')
if __name__=='__main__':main()
