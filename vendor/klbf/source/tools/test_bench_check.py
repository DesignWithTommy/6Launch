"""bench-check.py accepts the game's own BM-v1 result and rejects tampered ones."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('bench_check', HERE / 'bench-check.py')
bc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bc)

# Written byte for byte by the game's test (crates/victory-gpu/src/klbf_bench.rs,
# result_matches_the_checker_fixture).
FIXTURE = HERE.parent / 'crates' / 'victory-gpu' / 'tests' / 'fixtures' / 'KLBF-Bench-BM-v1-3b8db536.json'
VALID = json.loads(FIXTURE.read_text(encoding='utf-8'))


def tampered(fps=None, settings=None, **changes):
    """VALID with changes, re-fingerprinted: only the rule under test can catch it."""
    d = copy.deepcopy(VALID)
    d.update(changes)
    d['fps'].update(fps or {})
    d['settings'].update(settings or {})
    old, d['serial'] = d['serial'], bc.serial(d)
    d['share'] = d['share'].replace(old, d['serial'])
    return d


class BenchCheckTests(unittest.TestCase):
    def test_game_fixture_passes(self):
        self.assertEqual(VALID['serial'], '3b8db536')
        self.assertEqual(bc.check(VALID, FIXTURE.name), [])
        self.assertEqual(bc.check(VALID, 'KLBF-Bench-BM-v1-20261126-2104-3b8db536.json'), [])

    def test_serial_is_sha256_of_canonical_json(self):
        rest = {k: v for k, v in VALID.items() if k not in ('serial', 'share')}
        canon = json.dumps(rest, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode('utf-8')
        self.assertEqual(VALID['serial'], hashlib.sha256(canon).hexdigest()[:8])

    def test_tampered_samples_fail(self):
        edited = copy.deepcopy(VALID)
        edited['fps']['avg'] = 90.0
        cases = {
            'edited, serial kept': edited,
            'avg raised': tampered(fps={'avg': 90.0}),
            'short run': tampered(duration_s=200.0, stop='F10'),
            'partial': tampered(stamp='PARTIAL'),
            'black frames': tampered(black_frames=500),
            'min not the slowest second': tampered(fps={'min': 50.0}),
            'max not the fastest second': tampered(fps={'max': 70.0}),
            'per-second figures edited': tampered(fps_per_second=[60.0, 59.0, 61.2]),
            'other version': tampered(test_id='BM-v2'),
            'no schema': tampered(schema=None),
        }
        for label, d in cases.items():
            with self.subTest(label):
                self.assertNotEqual(bc.check(d), [])
        self.assertNotEqual(bc.check(VALID, 'KLBF-Bench-BM-v1-00000000.json'), [])

    def test_player_must_be_the_submitter(self):
        self.assertEqual(bc.check(VALID, login='someone-else'), [])  # anonymous
        named = tampered(player='PastiiJay')
        self.assertEqual(bc.check(named, login='pastiijay'), [])
        self.assertNotEqual(bc.check(named, login='someone-else'), [])

    def test_cli_prints_one_row(self):
        script = str(HERE / 'bench-check.py')
        card = 'https://github.com/user-attachments/assets/x'
        out = subprocess.run([sys.executable, '-I', script, str(FIXTURE), '--card', card],
                             capture_output=True, text=True, encoding='utf8')
        self.assertEqual(out.returncode, 0, out.stderr)
        self.assertEqual(len(out.stdout.strip().splitlines()), 1)
        self.assertIn(f'[card]({card})', out.stdout)
        self.assertIn('profile S · FSR · textures ≤ 1024 px · 12 pipelines skipped', out.stdout)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / FIXTURE.name
            path.write_text(json.dumps(tampered(fps={'avg': 90.0})), encoding='utf8')
            bad = subprocess.run([sys.executable, '-I', script, str(path)], capture_output=True, text=True, encoding='utf8')
            self.assertEqual(bad.returncode, 1)

    def test_settings_column(self):
        self.assertEqual(bc.settings(VALID), 'profile S · FSR · textures ≤ 1024 px · 12 pipelines skipped · '
                                             '0 pipelines denied · 4321.5 draws/frame · 0.25 % draws dropped')
        self.assertEqual(bc.settings({}), '')


if __name__ == '__main__':
    unittest.main()
