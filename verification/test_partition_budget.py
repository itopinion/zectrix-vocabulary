"""Offline regressions for consensus using cached chunks at the call budget."""
import sys
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'workers'))
from spelling import segmented_replay


class IntervalModel:
    def __init__(self, outputs):
        self.outputs = outputs
        self.calls = []

    def generate(self, chunk, **kwargs):
        # The synthetic input stores its sample index; preprocessing only pads
        # each disjoint interval with 3200 leading and 4800 trailing zeros.
        start = int(chunk[3200]) - 1
        end = start + len(chunk) - 8000
        interval = (start, end)
        self.calls.append(interval)
        if len(self.calls) > 18:
            raise AssertionError('Segment call budget exceeded')
        return SimpleNamespace(text=self.outputs[interval])


class PartitionBudget(unittest.TestCase):
    def run_case(self, skipped_routes=1, alternative_start='A', cached_final=True):
        end = 20
        # Every route shares cut 4 with the first route, so the existing
        # diversity ordering preserves this list's order.
        routes = [(0, 4, end), (0, 1, 2, 4, end), (0, 2, 3, 4, end)]
        outputs = {
            (0, 4): 'A B', (4, end): 'C',
            (0, 1): 'X', (1, 2): 'Y', (2, 4): 'B',
            (0, 2): alternative_start, (2, 3): 'Z', (3, 4): 'Q',
        }
        # Eight initial calls plus five two-call failures consume exactly 18.
        for cut in range(5, 10):
            routes.append((0, 4, cut, end))
            outputs[(4, cut)] = 'D'
            outputs[(cut, end)] = 'Unclear'
        for cut in range(10, 10 + skipped_routes):
            routes.append((0, 4, cut, end))
        routes.append((0, 2, 4, end) if cached_final else (0, 2, 4, 16, end))
        model = IntervalModel(outputs)
        audio = np.arange(1, end + 1, dtype=np.float32)
        with patch('spelling.pause_boundaries', return_value=list(range(end + 1))), \
             patch('spelling.candidate_partitions', return_value=routes):
            result, passes = segmented_replay(model, audio)
        self.assertEqual(len(model.calls), 18)
        self.assertEqual(len(set(model.calls)), 18)
        self.assertEqual(sum(p['method'] == 'disjoint_segment' for p in passes), 18)
        return result, passes

    def test_cached_route_completes_after_budget_exhaustion(self):
        result, passes = self.run_case()
        self.assertEqual(result, 'A B C')
        matches = [p for p in passes
                   if p['method'] == 'complete_partition' and p['text'] == result]
        self.assertEqual(len(matches), 2)
        self.assertNotEqual(matches[0]['cuts'], matches[1]['cuts'])

    def test_skipped_routes_do_not_hide_cached_consensus_after_route_limit(self):
        # The second matching route is 14th, beyond the usual 12-route limit.
        result, _ = self.run_case(skipped_routes=5)
        self.assertEqual(result, 'A B C')

    def test_disagreeing_cached_route_stays_unresolved(self):
        result, _ = self.run_case(skipped_routes=5, alternative_start='D')
        self.assertIsNone(result)

    def test_route_requiring_new_chunk_stays_within_call_budget(self):
        result, _ = self.run_case(skipped_routes=5, cached_final=False)
        self.assertIsNone(result)


if __name__ == '__main__':
    unittest.main()
