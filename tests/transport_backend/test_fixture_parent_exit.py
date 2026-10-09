"""The Python fixtures end when the process that started them does.

A test run that is killed never drops its guards, and on 2026-10 `key_agent.py`
and `password_sshd.py` were found serving for ever with their parent gone.
"""

import json
import os
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent

# Starts the fixture, prints its pid, and exits without waiting for it.
STARTER = """
import subprocess, sys
child = subprocess.Popen([sys.executable, "-B", sys.argv[1], sys.argv[2]],
                         stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                         stderr=subprocess.DEVNULL)
child.stdout.readline()
print(child.pid, flush=True)
"""


def alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    state = subprocess.run(
        ["ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True
    ).stdout.strip()
    return bool(state) and not state.startswith("Z")


class FixtureParentExit(unittest.TestCase):
    @unittest.skipUnless(os.name == "posix", "a parent's death is seen as adoption on Unix")
    def test_password_sshd_exits_when_its_parent_does(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.json"
            config.write_text(
                json.dumps(
                    {
                        "password": "p",
                        "methods": ["password"],
                        "authorized": [],
                        "log": str(Path(directory) / "log"),
                    }
                )
            )
            started = subprocess.run(
                [sys.executable, "-c", STARTER, str(HERE / "password_sshd.py"), str(config)],
                capture_output=True,
                text=True,
                timeout=30,
                check=True,
            )
        pid = int(started.stdout.strip())
        try:
            deadline = time.monotonic() + 10
            while alive(pid) and time.monotonic() < deadline:
                time.sleep(0.1)
            self.assertFalse(alive(pid), f"the fixture (pid {pid}) outlived its parent")
        finally:
            if alive(pid):
                os.kill(pid, 9)


if __name__ == "__main__":
    unittest.main()
