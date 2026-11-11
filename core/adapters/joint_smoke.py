"""Two new cross-branch checks in one process, without reexecuting the original suites."""
import sys, json, unittest
from pathlib import Path
root=Path.cwd().resolve()
sys.path.insert(0,str(root))
import h11
if Path(h11.__file__).resolve().parents[1]!=root:
    raise RuntimeError('wrong package origin')
from h11._headers import normalize_and_validate
from h11._util import LocalProtocolError
from h11._readers import ChunkedReader
from h11._receivebuffer import ReceiveBuffer
class Joint(unittest.TestCase):
    def test_bad_content_length_and_normal_chunk(self):
        with self.assertRaises(LocalProtocolError):
            normalize_and_validate([('Content-Length','1'*21)])
        self.assertEqual(normalize_and_validate([('Content-Length','20')]),[(b'content-length',b'20')])
        buf=ReceiveBuffer();buf+=b'3\r\nabc\r\n0\r\n\r\n'
        reader=ChunkedReader();seen=[]
        while True:
            event=reader(buf)
            if event is None:break
            seen.append(type(event).__name__)
            if type(event).__name__=='EndOfMessage':break
        self.assertEqual(seen,['Data','EndOfMessage'])
    def test_two_invalid_framings_same_runtime(self):
        with self.assertRaises(LocalProtocolError):normalize_and_validate([('Content-Length','9'*21)])
        buf=ReceiveBuffer();buf+=b'3\r\nxxx__1a\r\n'
        reader=ChunkedReader();reader(buf)
        with self.assertRaises(LocalProtocolError):reader(buf)
if __name__=='__main__':
 suite=unittest.defaultTestLoader.loadTestsFromTestCase(Joint)
 result=unittest.TextTestRunner(verbosity=1).run(suite)
 if not result.wasSuccessful():raise SystemExit(1)
 print(json.dumps({'new_joint_tests':result.testsRun,'failed':len(result.failures),'errors':len(result.errors),'python':sys.version.split()[0]}))
