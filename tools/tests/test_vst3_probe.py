import json,pathlib,subprocess,sys,tempfile,unittest
ROOT=pathlib.Path(__file__).resolve().parents[2]
class ProbeTests(unittest.TestCase):
 def run_probe(self,body):
  with tempfile.TemporaryDirectory()as directory:
   d=pathlib.Path(directory);sdk=d/'sdk';sdk.write_text('#!/usr/bin/env python3\nimport sys,json,time,pathlib\n'+body+'\n');sdk.chmod(0o700);report=d/'report.json'
   p=subprocess.run([sys.executable,str(ROOT/'tools/probe-vst3.py'),str(d/'plugin.vst3'),str(report),'--sdk',str(sdk),'--timeout','0.2'],capture_output=True,timeout=10)
   return p.returncode,json.loads(report.read_text())
 def test_success_keeps_metadata(self):
  code,report=self.run_probe('pathlib.Path(sys.argv[3]).write_text(json.dumps({"classes":[]}))');self.assertEqual(code,0);self.assertEqual(report['metadata'],{'classes':[]});self.assertFalse(report['is_security_sandbox'])
 def test_failure_preserves_exit_and_error(self):
  code,report=self.run_probe('print("native failure",file=sys.stderr)\nsys.exit(7)');self.assertEqual(code,1);self.assertEqual(report['exit_code'],7);self.assertIn('native failure',report['stderr'])
 def test_hang_becomes_timeout(self):
  code,report=self.run_probe('time.sleep(5)');self.assertEqual(code,1);self.assertEqual(report['status'],'timed_out')
if __name__=='__main__':unittest.main()
