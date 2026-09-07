#!/usr/bin/env python3
"""Controlled loopback SSH capability experiment; never uses the operator's keys.

Requires OpenSSH executables and the separately built ssh_identity_probe example.
Every process, key, authorized-keys file and agent socket is temporary. Results
contain public fingerprints and authentication outcomes, never key material.
"""
import argparse
import base64
import getpass
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import socket
import subprocess
import tempfile
import time


def run(*args, env=None):
    return subprocess.check_output(args, env=env, stderr=subprocess.STDOUT, text=True).strip()


def key(directory, name, password=''):
    path = directory / name
    run('ssh-keygen', '-q', '-t', 'ed25519', '-N', password, '-C', name, '-f', str(path))
    public = path.with_suffix('.pub').read_text()
    raw = base64.b64decode(public.split()[1])
    digest = hashlib.sha256(raw).digest()
    return path, public, 'SHA256:' + base64.b64encode(digest).decode().rstrip('='), digest.hex()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    processes = []
    with tempfile.TemporaryDirectory(prefix='gwz-ssh-probe-') as temporary:
        directory = Path(temporary)
        host = key(directory, 'host')
        identity_a = key(directory, 'identity-a')
        identity_b = key(directory, 'identity-b')
        wrong = key(directory, 'wrong')
        encrypted = key(directory, 'encrypted', 'probe-passphrase')
        authorized = directory / 'authorized_keys'
        authorized.write_text(identity_a[1] + identity_b[1] + encrypted[1])
        bare = directory / 'remote.git'
        run('git', 'init', '--bare', '--quiet', str(bare))
        listener = socket.socket()
        listener.bind(('127.0.0.1', 0))
        port = listener.getsockname()[1]
        listener.close()
        config = directory / 'sshd_config'
        config.write_text(f'''Port {port}
ListenAddress 127.0.0.1
HostKey {host[0]}
PidFile {directory / 'sshd.pid'}
AuthorizedKeysFile {authorized}
StrictModes no
PasswordAuthentication no
KbdInteractiveAuthentication no
UsePAM no
AllowUsers {getpass.getuser()}
LogLevel VERBOSE
ForceCommand {shlex.quote(shutil.which('git'))} upload-pack {shlex.quote(str(bare))}
''')
        log_path = directory / 'sshd.log'
        environment = os.environ.copy()
        environment['SSH_AUTH_SOCK'] = str(directory / 'agent.sock')
        askpass = directory / 'askpass'
        askpass.write_text('#!/bin/sh\nprintf "%s\\n" probe-passphrase\n')
        askpass.chmod(0o700)
        environment.update(SSH_ASKPASS=str(askpass), SSH_ASKPASS_REQUIRE='force', DISPLAY='gwz-probe')
        rows = []
        try:
            with log_path.open('w') as server_log:
                server = subprocess.Popen(['/usr/sbin/sshd', '-D', '-e', '-f', str(config)],
                                          stdout=server_log, stderr=server_log)
                processes.append(server)
                agent = subprocess.Popen(['ssh-agent', '-D', '-a', environment['SSH_AUTH_SOCK']],
                                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                processes.append(agent)
                deadline = time.monotonic() + 5
                while True:
                    if server.poll() is not None:
                        raise RuntimeError('fixture sshd failed: ' + log_path.read_text())
                    try:
                        with socket.create_connection(('127.0.0.1', port), timeout=0.2): pass
                        if Path(environment['SSH_AUTH_SOCK']).exists(): break
                    except OSError: pass
                    if time.monotonic() >= deadline: raise RuntimeError('fixture startup timed out')
                    time.sleep(0.02)
                for label, order, mode, explicit, expected in [
                    ('agent-a-first', [identity_a, identity_b], 'agent', None, True),
                    ('agent-b-first', [identity_b, identity_a], 'agent', None, True),
                    ('explicit-a-with-b-first', [identity_b, identity_a], 'file', identity_a[0], True),
                    ('explicit-wrong-no-fallback', [identity_b], 'file', wrong[0], False),
                    ('explicit-missing-no-fallback', [identity_b], 'file', directory / 'missing', False),
                    ('encrypted-file-with-unlocked-agent', [identity_b, encrypted], 'file', encrypted[0], False),
                ]:
                    run('ssh-add', '-D', env=environment)
                    for item in order: run('ssh-add', str(item[0]), env=environment)
                    offset = log_path.stat().st_size
                    env = {**environment, 'GWZ_PROBE_URL': f'ssh://{getpass.getuser()}@127.0.0.1:{port}/remote.git',
                           'GWZ_PROBE_HOST_SHA256': host[3], 'GWZ_PROBE_MODE': mode,
                           'GWZ_PROBE_REPOSITORY': str(directory / f'client-{label}')}
                    if explicit is not None: env['GWZ_PROBE_KEY'] = str(explicit)
                    result = json.loads(run(str(binary), env=env))
                    if result['authenticated'] != expected: raise AssertionError((label, result, log_path.read_text()[offset:]))
                    accepted = [line.split('SHA256:', 1)[1].split()[0] for line in log_path.read_text()[offset:].splitlines()
                                if 'Accepted publickey' in line and 'SHA256:' in line]
                    row = {'case': label, **result, 'accepted_fingerprints': ['SHA256:' + value for value in accepted]}
                    rows.append(row)
                assert rows[0]['accepted_fingerprints'] == [identity_a[2]], rows[0]
                assert rows[1]['accepted_fingerprints'] == [identity_b[2]], rows[1]
                assert rows[2]['accepted_fingerprints'] == [identity_a[2]], rows[2]
        finally:
            for process in reversed(processes):
                process.terminate()
                try: process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill(); process.wait()
        args.output.write_text(json.dumps({'scope': 'native library capability, not product acceptance', 'cases': rows}, indent=2) + '\n')
        print(f'controlled SSH capability probe: {len(rows)} cases passed')


if __name__ == '__main__': main()
