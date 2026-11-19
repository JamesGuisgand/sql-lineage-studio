"""Integration tests of the public HTTP API using a controlled, fake Rust executable.
These tests don't replace the original cargo test / Docker build.
"""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]


class APITest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.fake = Path(cls.temp.name) / 'fake_engine.py'
        cls.fake.write_text('''#!/usr/bin/env python3
import json,sys
sql=sys.stdin.read()
fmt=sys.argv[sys.argv.index('--format')+1]
if sql.strip().upper().startswith('DROP'):
 sys.stderr.write('Erreur : Instruction 1 : Instruction SQL non prise en charge\\n')
 sys.exit(1)
if fmt == 'json':
 impact=None
 if '--impact' in sys.argv:
  target=sys.argv[sys.argv.index('--impact')+1]
  impact={'start':target,'affected':['reporting.sales'],'paths':[{'target':'reporting.sales','distance':1,'path':[target,'reporting.sales']}],'cycles':[]}
 print(json.dumps({'statements':[{'kind':'create_view','target':'reporting.sales','sources':['crm.customers','raw.orders']}], 'columns':[{'source':'raw.orders.amount','target':'reporting.sales.revenue','confidence':'resolved'}], 'impact':impact,'warnings':[], 'dot':'digraph lineage { \"raw.orders\" -> \"reporting.sales\"; }'}))
else:
 print('digraph lineage { \"raw.orders\" -> \"reporting.sales\"; }')
''')
        cls.fake.chmod(0o755)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            cls.port = sock.getsockname()[1]
        env = os.environ.copy()
        env['PORT'] = str(cls.port)
        env['SQL_LINEAGE_BIN'] = str(cls.fake)
        cls.server = subprocess.Popen([sys.executable, str(ROOT / 'server.py')],
                                      cwd=ROOT, env=env, stdout=subprocess.PIPE,
                                      stderr=subprocess.PIPE)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                c = http.client.HTTPConnection('127.0.0.1', cls.port, timeout=1)
                c.request('GET','/healthz')
                c.getresponse().read()
                c.close()
                break
            except OSError:
                time.sleep(.04)
        else:
            raise RuntimeError('Server failed to start')

    @classmethod
    def tearDownClass(cls):
        cls.server.terminate()
        try: cls.server.communicate(timeout=4)
        except subprocess.TimeoutExpired: cls.server.kill()
        cls.temp.cleanup()

    def request(self, method, path, data=None, headers=None):
        conn = http.client.HTTPConnection('127.0.0.1', self.port, timeout=5)
        conn.request(method, path, data, headers or {})
        response = conn.getresponse()
        payload = response.read()
        result = (response.status, dict(response.getheaders()), payload)
        conn.close()
        return result

    def test_healthz(self):
        code, headers, body = self.request('GET', '/healthz')
        self.assertEqual(code, 200)
        self.assertEqual(json.loads(body), {'status': 'ok'})
        self.assertIn('nosniff', headers['X-Content-Type-Options'])

    def test_static_html(self):
        code, headers, body = self.request('GET', '/')
        self.assertEqual(code, 200)
        self.assertIn(b'SQL Lineage Studio', body)
        self.assertIn('Content-Security-Policy', headers)
        self.assertIn('text/html', headers['Content-Type'])

    def test_example(self):
        code, _, body = self.request('GET', '/example.sql')
        self.assertEqual(code, 200)
        self.assertIn(b'CREATE VIEW', body)

    def test_success_json_and_dot(self):
        body = json.dumps({'sql': 'CREATE VIEW reporting.sales AS SELECT * FROM raw.orders;'}).encode()
        code, _, response = self.request('POST', '/api/analyze', body,
                                         {'Content-Type': 'application/json'})
        self.assertEqual(code, 200, response)
        result = json.loads(response)
        self.assertEqual(result['stats']['sources'], 2)
        self.assertEqual(result['stats']['relations'], 2)
        self.assertEqual(result['stats']['targets'], 1)
        self.assertIn('digraph', result['dot'])
        self.assertEqual(len(result['columns']), 1)

    def test_impact(self):
        body = json.dumps({'sql':'SELECT id FROM raw.orders', 'impact_table':'raw.orders'}).encode()
        code, _, response = self.request('POST','/api/analyze', body,
                                         {'Content-Type':'application/json'})
        self.assertEqual(code, 200, response)
        report = json.loads(response)
        self.assertEqual(report['impact']['affected'], ['reporting.sales'])

    def test_dialect_validated(self):
        body = b'{"sql":"SELECT 1", "dialect":"unknown"}'
        code, _, response = self.request('POST','/api/analyze', body,
                                         {'Content-Type':'application/json'})
        self.assertEqual(code, 400)

    def test_impact_name_validated(self):
        body = b'{"sql":"SELECT 1", "impact_table":"a; DROP TABLE x"}'
        code, _, response = self.request('POST','/api/analyze', body,
                                         {'Content-Type':'application/json'})
        self.assertEqual(code, 400)

    def test_unsupported_query(self):
        body = b'{"sql":"DROP TABLE users;"}'
        code, _, response = self.request('POST', '/api/analyze', body,
                                         {'Content-Type': 'application/json'})
        self.assertEqual(code, 422)
        self.assertIn('Instruction SQL non prise en charge', json.loads(response)['error'])

    def test_size_limit(self):
        body = json.dumps({'sql': 'a' * 132000}).encode()
        code, _, _ = self.request('POST', '/api/analyze', body,
                                  {'Content-Type': 'application/json'})
        self.assertEqual(code, 413)

    def test_wrong_mime(self):
        code, _, _ = self.request('POST', '/api/analyze', b'foo',
                                  {'Content-Type': 'text/plain'})
        self.assertEqual(code, 415)

    def test_no_path_traversal(self):
        code, _, _ = self.request('GET', '/../../server.py')
        self.assertEqual(code, 404)

    def test_invalid_json(self):
        code, _, _ = self.request('POST', '/api/analyze', b'{foo',
                                  {'Content-Type': 'application/json'})
        self.assertEqual(code, 400)


if __name__ == '__main__':
    unittest.main()
