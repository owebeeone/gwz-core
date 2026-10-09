#!/usr/bin/env python3
"""A loopback SSH agent for gwz-core's agent key-type tests (TR2.8).

It lists the keys a test names, in order, and signs as each key's entry says:

- "upstream": a private ssh-agent signs. With "rsa_sha1" set, a request for
  rsa-sha2-256 or rsa-sha2-512 is sent on with no flags, so the answer is an
  ssh-rsa (SHA-1) signature, as an agent without RFC 8332 support answers.
- "sk": the fixture's software authenticator signs for a security key
  (sk-ssh-ed25519@openssh.com, sk-ecdsa-sha2-nistp256@openssh.com, or a
  certificate of either). It builds what a FIDO authenticator signs, SHA-256 of
  the key's application, the flags with user presence set, a counter and
  SHA-256 of the data (OpenSSH's PROTOCOL.u2f), and the private agent signs
  that with the plain key that backs the security key, so the signature is the
  one a hardware key holding that private key would make.
- "dsa": this agent generates a DSA key and signs with it itself, since
  OpenSSH 10's agent no longer holds DSA keys.
- "absent": every request fails, as for a security key whose device is absent.

It is written on Python's standard library alone, logs each request as one
JSON object per line, never key material or a signature, and serves until it
is killed or the process that started it exits.

Usage: key_agent.py CONFIG. CONFIG is a JSON file:
  {"socket": "<path>", "log": "<path>", "upstream": "<private agent socket>",
   "rsa_sha1": false,
   "keys": [{"sign": "upstream", "blob": "<base64>"},
            {"sign": "sk", "blob": "<base64>", "backing": "<base64>",
             "application": "ssh:"},
            {"sign": "dsa"}, {"sign": "absent", "blob": "<base64>"}]}
It listens on the socket and then prints {"keys": [...]} on one line: each
key's base64 public blob, in list order, a DSA key's as generated.
"""
from __future__ import annotations

import base64
import hashlib
import json
import os
import secrets
import socket
import struct
import sys
import threading
import time

# DSA domain parameters, 1024-bit P and 160-bit Q as ssh-dss requires, made
# once with `openssl genpkey -genparam` and checked prime. They are public.
P = int(
    "9cd3b7ba4d5059eeb05a8ef65a7c83181a1e323d3c1986f17752c783d7754cd691a3ce19b64f53d7eb6a2259"
    "61341b4ba9921189539403072446f4cafc36cd4b5dcc260426cb278bf4d81def0deb20e4c648068aaafe362d"
    "5d2a890e47d4cf3023191f75570a3df6437c67063f74c6391d9b6af735c1cad1cc55e4865eaf3ee9",
    16,
)
Q = int("cc47e2885c7005f2cae1de266d6dc4efad520447", 16)
G = int(
    "955e08a4686ef18adce5f3444c001c4a82a46c252e4996a4a165b4148bffacc53aebc47a490c92fa487199dc"
    "cc23c0196880bf2696525b03618d4bb6136434bb7754fe7a542f2dea049e727c2ca510b7eeb24a396befe6d1"
    "778b489be01874641c454bbd03cbdea67c50dc01e953ff6331e438cc8d2e6b4e33d18b4420daf440",
    16,
)
FAILURE, SIGN_RESPONSE = bytes([5]), 14
SK_NAMES = {
    b"ssh-ed25519": b"sk-ssh-ed25519@openssh.com",
    b"ecdsa-sha2-nistp256": b"sk-ecdsa-sha2-nistp256@openssh.com",
}
USER_PRESENT = 0x01


def u32(value: int) -> bytes:
    return struct.pack(">I", value)


def string(value: bytes) -> bytes:
    return u32(len(value)) + value


def mpint(value: int) -> bytes:
    return string(value.to_bytes((value.bit_length() + 8) // 8, "big") if value else b"")


class Reader:
    def __init__(self, data: bytes) -> None:
        self.data, self.at = data, 0

    def take(self, count: int) -> bytes:
        if self.at + count > len(self.data):
            raise ValueError("short message")
        self.at += count
        return self.data[self.at - count : self.at]

    def u32(self) -> int:
        return struct.unpack(">I", self.take(4))[0]

    def string(self) -> bytes:
        return self.take(self.u32())


def receive(sock: socket.socket, count: int) -> bytes | None:
    data = b""
    while len(data) < count:
        chunk = sock.recv(count - len(data))
        if not chunk:
            return None
        data += chunk
    return data


def read_frame(sock: socket.socket) -> bytes | None:
    header = receive(sock, 4)
    if header is None:
        return None
    length = struct.unpack(">I", header)[0]
    if not 0 < length <= 1 << 20:
        raise ValueError("bad frame length")
    return receive(sock, length)


def dsa_sign(x: int, data: bytes) -> bytes:
    digest = int.from_bytes(hashlib.sha1(data).digest(), "big")
    while True:
        k = secrets.randbelow(Q - 1) + 1
        r = pow(G, k, P) % Q
        s = pow(k, -1, Q) * (digest + x * r) % Q
        if r and s:
            return r.to_bytes(20, "big") + s.to_bytes(20, "big")


class Agent:
    def __init__(self, config: dict) -> None:
        self.upstream = config.get("upstream")
        self.rsa_sha1 = bool(config.get("rsa_sha1"))
        self.log_path = config["log"]
        self.lock = threading.Lock()
        self.counter = 0
        self.keys = []
        for entry in config["keys"]:
            key = dict(entry)
            if key["sign"] == "dsa":
                key["x"] = secrets.randbelow(Q - 1) + 1
                y = pow(G, key["x"], P)
                key["blob"] = string(b"ssh-dss") + mpint(P) + mpint(Q) + mpint(G) + mpint(y)
            else:
                key["blob"] = base64.b64decode(key["blob"])
            self.keys.append(key)

    def log(self, entry: dict) -> None:
        with self.lock, open(self.log_path, "a", encoding="utf-8") as log:
            log.write(json.dumps(entry) + "\n")

    def forward(self, body: bytes) -> bytes:
        with socket.socket(socket.AF_UNIX) as upstream:
            upstream.connect(self.upstream)
            upstream.sendall(string(body))
            reply = read_frame(upstream)
        if reply is None:
            raise ValueError("the private agent closed")
        return reply

    def authenticate(self, key: dict, data: bytes) -> bytes:
        with self.lock:
            self.counter += 1
            counter = self.counter
        application = key.get("application", "ssh:").encode()
        signed = (hashlib.sha256(application).digest() + bytes([USER_PRESENT]) + u32(counter)
                  + hashlib.sha256(data).digest())
        backing = base64.b64decode(key["backing"])
        reply = Reader(self.forward(bytes([13]) + string(backing) + string(signed) + u32(0)))
        if reply.take(1)[0] != SIGN_RESPONSE:
            raise ValueError("the private agent refused")
        signature = Reader(reply.string())
        algorithm, raw = signature.string(), signature.string()
        return string(SK_NAMES[algorithm]) + string(raw) + bytes([USER_PRESENT]) + u32(counter)

    def handle(self, body: bytes) -> bytes:
        if body[:1] == bytes([11]):
            self.log({"op": "list"})
            listed = b"".join(string(key["blob"]) + string(b"") for key in self.keys)
            return bytes([12]) + u32(len(self.keys)) + listed
        if body[:1] != bytes([13]):
            return FAILURE
        request = Reader(body[1:])
        blob, data, flags = request.string(), request.string(), request.u32()
        index = next((i for i, key in enumerate(self.keys) if key["blob"] == blob), None)
        if index is None:
            return FAILURE
        key = self.keys[index]
        self.log({"op": "sign", "key": index, "flags": flags})
        if key["sign"] == "absent":
            return FAILURE
        if key["sign"] == "malformed_rsa":
            key["sign"] = "upstream"
            signature = string(b"ssh-rsa") + string(bytes([0xff]) * key["length"])
            return bytes([SIGN_RESPONSE]) + string(signature)
        if key["sign"] == "dsa":
            signature = string(b"ssh-dss") + string(dsa_sign(key["x"], data))
            return bytes([SIGN_RESPONSE]) + string(signature)
        if key["sign"] == "sk":
            return bytes([SIGN_RESPONSE]) + string(self.authenticate(key, data))
        if self.rsa_sha1 and flags in (2, 4):
            flags = 0
        return self.forward(bytes([13]) + string(blob) + string(data) + u32(flags))


def serve(agent: Agent, connection: socket.socket) -> None:
    with connection:
        while True:
            try:
                body = read_frame(connection)
                if body is None:
                    return
                reply = agent.handle(body)
            except (OSError, ValueError, KeyError):
                reply = FAILURE
            try:
                connection.sendall(string(reply))
            except OSError:
                return


def exit_with_parent() -> None:
    """Exit once the process that started this one is gone.

    A test that is killed cannot run its guard's drop, so without this the
    server serves for ever, orphaned. The parent's pid changes when it dies on
    Unix (the process is adopted); where it does not, this never fires.
    """
    parent = os.getppid()

    def watch() -> None:
        while os.getppid() == parent:
            time.sleep(0.5)
        os._exit(0)

    threading.Thread(target=watch, daemon=True).start()


def main() -> None:
    exit_with_parent()
    with open(sys.argv[1], encoding="utf-8") as config_file:
        config = json.load(config_file)
    agent = Agent(config)
    listener = socket.socket(socket.AF_UNIX)
    listener.bind(config["socket"])
    listener.listen(16)
    blobs = [base64.b64encode(key["blob"]).decode() for key in agent.keys]
    print(json.dumps({"keys": blobs}), flush=True)
    while True:
        connection, _ = listener.accept()
        threading.Thread(target=serve, args=(agent, connection), daemon=True).start()


if __name__ == "__main__":
    main()
