"""Runs against the compiled Rust CLI in CI, skipped on machines without Rust."""
import json
import os
from pathlib import Path
import subprocess
import unittest

BINARY = os.getenv('SQL_LINEAGE_REAL_BIN')

@unittest.skipUnless(BINARY and Path(BINARY).is_file(), 'Compiled Rust binary not provided')
class RealEngineTests(unittest.TestCase):
    def run_engine(self, sql, *flags):
        result = subprocess.run([BINARY, '--format', 'json', *flags, '-'], input=sql,
                                capture_output=True, text=True, check=False, timeout=8)
        self.assertEqual(result.returncode, 0, result.stderr)
        return json.loads(result.stdout)

    def test_column_lineage(self):
        report = self.run_engine('CREATE VIEW v AS SELECT t.a * t.b AS value FROM raw.orders t')
        pairs = {(v['source'], v['target']) for v in report['columns']}
        self.assertIn(('raw.orders.a','v.value'), pairs)
        self.assertIn(('raw.orders.b','v.value'), pairs)

    def test_cte(self):
        report = self.run_engine('CREATE VIEW v AS WITH x AS (SELECT a.id FROM raw.orders a) SELECT x.id FROM x')
        self.assertEqual(report['statements'][0]['sources'], ['raw.orders'])

    def test_impact(self):
        report = self.run_engine('CREATE VIEW b AS SELECT a.id FROM a; CREATE VIEW c AS SELECT b.id FROM b',
                                 '--impact', 'a')
        self.assertEqual(report['impact']['affected'], ['b', 'c'])

    def test_unsupported(self):
        p = subprocess.run([BINARY, '--format', 'json', '-'], input='DROP TABLE users',
                           capture_output=True, text=True, check=False, timeout=8)
        self.assertNotEqual(p.returncode, 0)
