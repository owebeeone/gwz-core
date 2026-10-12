"""The core sources the transport consumer loads by `#[path]` stand alone.

tests/transport_consumer is a crate of its own that depends on the published gwz-transport and nothing else, and it
loads core's `src/git/endpoint/stream_io.rs` into a test (README, "Blocking Git adapter qualification"). A change that
gives that file a `super::` sibling or a `cfg_if` use compiles in core and fails only in the hosted candidate job's
"Consumer archive proof" step, which cargo-packages the transport and builds the consumer offline (T7's
`super::setup_retry` did, from 2026-10-11). This is the local, stdlib-only check of the same property.
"""
from pathlib import Path
import re
import unittest

CORE = Path(__file__).resolve().parents[2]
CONSUMER = CORE / 'tests/transport_consumer'
LOADED = re.compile(r'#\[path = "([^"]+)"\]')


def loaded_core_endpoint_sources():
    found = []
    for source in [*CONSUMER.glob('src/*.rs'), *CONSUMER.glob('tests/*.rs')]:
        for target in LOADED.findall(source.read_text(encoding='utf-8')):
            path = (source.parent / target).resolve()
            if path.is_relative_to(CORE / 'src/git'):
                found.append(path)
    return found


class ConsumerBridge(unittest.TestCase):
    def test_the_consumer_loads_the_blocking_adapter(self):
        self.assertIn(CORE / 'src/git/endpoint/stream_io.rs', loaded_core_endpoint_sources())

    def test_a_loaded_core_source_names_nothing_outside_std_and_the_transport(self):
        for path in loaded_core_endpoint_sources():
            text = path.read_text(encoding='utf-8')
            self.assertNotIn('cfg_if', text, f'{path.name}: the consumer has no cfg-if')
            self.assertNotIn('crate::', text, f'{path.name}: the consumer is not gwz-core')
            self.assertEqual(re.findall(r'\bsuper::(?!\*)\w+', text), [], f'{path.name}: the consumer has no sibling modules')


if __name__ == '__main__':
    unittest.main()
