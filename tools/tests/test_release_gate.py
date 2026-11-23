import importlib.util,json,struct,tempfile,unittest
from pathlib import Path
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('release_gate',ROOT/'tools/release-gate.py');gate=importlib.util.module_from_spec(spec);spec.loader.exec_module(gate)
class GateTests(unittest.TestCase):
 def stage(self,d):
  root=Path(d);head=bytearray(64);head[:6]=b'\x7fELF\x02\x01';struct.pack_into('<H',head,18,62);(root/'test.clap').write_bytes(head)
  for name in('LICENSE-MIT','LICENSE-APACHE','THIRD_PARTY.md'):(root/name).write_text('notice')
  m={'schema':2,'version':'0.146.0','platform':'linux','files_sha256':{p.name:gate.digest(p)for p in root.iterdir()}}
  (root/'manifest.json').write_text(json.dumps(m));return root,m
 def test_complete_inventory_and_architecture(self):
  with tempfile.TemporaryDirectory()as d:
   root,_=self.stage(d);report=gate.verify_stage(root,'linux','x86_64');self.assertEqual(report['files'],4)
   with self.assertRaises(ValueError):gate.verify_stage(root,'linux','aarch64')
   with self.assertRaises(ValueError):gate.verify_stage(root,'windows','x86_64')
 def test_tamper_extra_and_escape_are_rejected(self):
  with tempfile.TemporaryDirectory()as d:
   root,m=self.stage(d);(root/'extra').write_text('unexpected')
   with self.assertRaises(ValueError):gate.verify_stage(root,'linux','x86_64')
   (root/'extra').unlink();(root/'test.clap').write_bytes(b'changed')
   with self.assertRaises(ValueError):gate.verify_stage(root,'linux','x86_64')
   with self.assertRaises(ValueError):gate.safe_path(root,'../outside')
 def test_bad_native_headers(self):
  with tempfile.TemporaryDirectory()as d:
   p=Path(d)/'bad';p.write_bytes(b'MZ')
   with self.assertRaises(ValueError):gate.native_image(p)
 def test_acceptance_must_attest_exact_manifest(self):
  spec=importlib.util.spec_from_file_location('compat',ROOT/'tools/compatibility-suite.py');compat=importlib.util.module_from_spec(spec);spec.loader.exec_module(compat)
  doc=compat.acceptance_template();doc['platform']='linux';doc['release_manifest_sha256']='a'*64
  self.assertEqual(gate.acceptance(doc,'a'*64,'linux')['status'],'pending')
  with self.assertRaises(ValueError):gate.acceptance(doc,'b'*64,'linux')
if __name__=='__main__':unittest.main()
