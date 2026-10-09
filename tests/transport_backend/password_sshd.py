#!/usr/bin/env python3
"""A loopback SSH server for gwz-core's URL-password tests (TR2.18).

Stock sshd checks only system passwords, so this test-only server accepts one
fixed test password instead. It is written on Python's standard library
alone, so a test needs no package and no network. It speaks only what libssh2
needs: diffie-hellman-group14-sha256, an rsa-sha2-256 host key,
chacha20-poly1305@openssh.com, password and RSA or DSA publickey
authentication, and session channels whose exec request runs a Git command.
It sends server-sig-algs (RFC 8308) only when its configuration names a value,
so a test chooses whether the client has one and what it lists (TR2.8). It
logs each authentication request, never a password, as one JSON object per
line, a publickey request with its algorithm.

Usage: password_sshd.py CONFIG. CONFIG is a JSON file:
  {"password": "...", "methods": ["password", "publickey"],
   "authorized": ["<base64 RSA or DSA public key blob>", ...], "log": "<path>",
   "server_sig_algs": "<comma-separated algorithms>"}, the last optional.
It listens on 127.0.0.1 at a port of its own, prints {"port": ...,
"host_key": "ssh-rsa <base64>"} on one line, and serves until it is killed.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import math
import os
import secrets
import socket
import struct
import subprocess
import sys
import threading
import time

M32 = 0xFFFFFFFF
# RFC 3526 group 14, generator 2.
GROUP14 = int(
    "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74020BBEA63B139B22514A0879"
    "8E3404DDEF9519B3CD3A431B302B0A6DF25F14374FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B"
    "0BFF5CB6F406B7EDEE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF0598DA4836"
    "1C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB9ED529077096966D670C354E4ABC9804"
    "F1746C08CA18217C32905E462E36CE3BE39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF6"
    "955817183995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF",
    16,
)
DIGEST_INFO = {
    b"ssh-rsa": (hashlib.sha1, bytes.fromhex("3021300906052b0e03021a05000414")),
    b"rsa-sha2-256": (hashlib.sha256, bytes.fromhex("3031300d060960864801650304020105000420")),
    b"rsa-sha2-512": (hashlib.sha512, bytes.fromhex("3051300d060960864801650304020305000440")),
}
ALGORITHMS = {*DIGEST_INFO, b"ssh-dss"}


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

    def byte(self) -> int:
        return self.take(1)[0]

    def u32(self) -> int:
        return struct.unpack(">I", self.take(4))[0]

    def string(self) -> bytes:
        return self.take(self.u32())

    def mpint(self) -> int:
        return int.from_bytes(self.string(), "big")


def probable_prime(candidate: int) -> bool:
    for small in (3, 5, 7, 11, 13, 17, 19, 23, 29, 31, 37, 41, 43, 47):
        if candidate % small == 0:
            return False
    odd, shifts = candidate - 1, 0
    while odd % 2 == 0:
        odd, shifts = odd // 2, shifts + 1
    for _ in range(40):
        x = pow(secrets.randbelow(candidate - 3) + 2, odd, candidate)
        if x in (1, candidate - 1):
            continue
        for _ in range(shifts - 1):
            x = pow(x, 2, candidate)
            if x == candidate - 1:
                break
        else:
            return False
    return True


def rsa_key(bits: int = 2048) -> tuple[int, int, int]:
    """A fresh RSA key: modulus, public and private exponents."""
    def prime() -> int:
        while True:
            candidate = secrets.randbits(bits // 2) | (3 << (bits // 2 - 2)) | 1
            if probable_prime(candidate):
                return candidate

    while True:
        p, q = prime(), prime()
        phi = (p - 1) * (q - 1)
        if p != q and math.gcd(65537, phi) == 1 and (p * q).bit_length() == bits:
            return p * q, 65537, pow(65537, -1, phi)


def encoded_digest(message: bytes, algorithm: bytes, length: int) -> bytes:
    """EMSA-PKCS1-v1_5 of `message` for an RSA signature `algorithm`."""
    digest, prefix = DIGEST_INFO[algorithm]
    t = prefix + digest(message).digest()
    return b"\x00\x01" + b"\xff" * (length - len(t) - 3) + b"\x00" + t


def quarter_round(x: list[int], a: int, b: int, c: int, d: int) -> None:
    for left, right, target, rotation in ((a, b, d, 16), (c, d, b, 12), (a, b, d, 8), (c, d, b, 7)):
        x[left] = (x[left] + x[right]) & M32
        mixed = x[target] ^ x[left]
        x[target] = ((mixed << rotation) | (mixed >> (32 - rotation))) & M32


def chacha20(key: bytes, nonce: bytes, counter: int, data: bytes) -> bytes:
    """The original ChaCha20, with a 64-bit counter and nonce, as OpenSSH's."""
    words = struct.unpack("<8I", key)
    out = bytearray()
    for offset in range(0, len(data), 64):
        block = counter + offset // 64
        state = [0x61707865, 0x3320646E, 0x79622D32, 0x6B206574, *words, block & M32,
                 block >> 32, *struct.unpack("<2I", nonce)]
        x = list(state)
        for _ in range(10):
            for indices in ((0, 4, 8, 12), (1, 5, 9, 13), (2, 6, 10, 14), (3, 7, 11, 15),
                            (0, 5, 10, 15), (1, 6, 11, 12), (2, 7, 8, 13), (3, 4, 9, 14)):
                quarter_round(x, *indices)
        stream = struct.pack("<16I", *((x[i] + state[i]) & M32 for i in range(16)))
        chunk = data[offset : offset + 64]
        mixed = int.from_bytes(chunk, "little") ^ int.from_bytes(stream[: len(chunk)], "little")
        out += mixed.to_bytes(len(chunk), "little")
    return bytes(out)


def poly1305(key: bytes, message: bytes) -> bytes:
    r = int.from_bytes(key[:16], "little") & 0x0FFFFFFC0FFFFFFC0FFFFFFC0FFFFFFF
    accumulator, prime = 0, (1 << 130) - 5
    for offset in range(0, len(message), 16):
        block = message[offset : offset + 16] + b"\x01"
        accumulator = (accumulator + int.from_bytes(block, "little")) * r % prime
    return ((accumulator + int.from_bytes(key[16:32], "little")) & ((1 << 128) - 1)).to_bytes(16, "little")


class Connection:
    def __init__(self, sock: socket.socket, server: "Server") -> None:
        self.sock, self.server = sock, server
        self.send_lock = threading.Lock()
        self.send_seq = self.recv_seq = 0
        self.send_keys = self.recv_keys = None
        self.session_id = b""
        self.channels: dict[int, Channel] = {}
        self.next_channel = 0

    def read_exact(self, count: int) -> bytes:
        data = b""
        while len(data) < count:
            chunk = self.sock.recv(count - len(data))
            if not chunk:
                raise EOFError
            data += chunk
        return data

    def read_packet(self) -> bytes:
        nonce = struct.pack(">Q", self.recv_seq)
        if self.recv_keys is None:
            body = self.read_exact(struct.unpack(">I", self.read_exact(4))[0])
        else:
            main, header = self.recv_keys
            sealed_length = self.read_exact(4)
            length = struct.unpack(">I", chacha20(header, nonce, 0, sealed_length))[0]
            if length > 1 << 18:
                raise ValueError("oversized packet")
            sealed = self.read_exact(length)
            tag = self.read_exact(16)
            if not hmac.compare_digest(poly1305(chacha20(main, nonce, 0, bytes(32)), sealed_length + sealed), tag):
                raise ValueError("bad tag")
            body = chacha20(main, nonce, 1, sealed)
        self.recv_seq = (self.recv_seq + 1) & M32
        return body[1 : len(body) - body[0]]

    def send(self, payload: bytes) -> None:
        with self.send_lock:
            nonce = struct.pack(">Q", self.send_seq)
            aligned = 1 + len(payload) + (0 if self.send_keys else 4)
            padding = 8 - aligned % 8
            padding += 8 if padding < 4 else 0
            body = bytes([padding]) + payload + os.urandom(padding)
            if self.send_keys is None:
                packet = u32(len(body)) + body
            else:
                main, header = self.send_keys
                length = chacha20(header, nonce, 0, u32(len(body)))
                sealed = chacha20(main, nonce, 1, body)
                packet = length + sealed + poly1305(chacha20(main, nonce, 0, bytes(32)), length + sealed)
            self.send_seq = (self.send_seq + 1) & M32
            self.sock.sendall(packet)

    def key_exchange(self) -> None:
        server_version = b"SSH-2.0-gwz_password_fixture"
        self.sock.sendall(server_version + b"\r\n")
        line = b""
        while not line.startswith(b"SSH-"):
            line = b""
            while not line.endswith(b"\n"):
                line += self.read_exact(1)
        client_version = line.rstrip(b"\r\n")
        names = [b"diffie-hellman-group14-sha256", b"rsa-sha2-256", b"chacha20-poly1305@openssh.com",
                 b"chacha20-poly1305@openssh.com", b"hmac-sha2-256", b"hmac-sha2-256", b"none", b"none",
                 b"", b""]
        server_init = bytes([20]) + os.urandom(16) + b"".join(string(name) for name in names) + b"\x00" + u32(0)
        self.send(server_init)
        client_init = self.read_packet()
        if client_init[0] != 20:
            raise ValueError("expected KEXINIT")
        kexdh = Reader(self.read_packet())
        if kexdh.byte() != 30:
            raise ValueError("expected KEXDH_INIT")
        e = kexdh.mpint()
        if not 1 < e < GROUP14 - 1:
            raise ValueError("bad e")
        y = secrets.randbits(512) | 1
        f, shared = pow(2, y, GROUP14), pow(e, y, GROUP14)
        host = self.server.host_blob
        exchange = hashlib.sha256(string(client_version) + string(server_version) + string(client_init)
                                  + string(server_init) + string(host) + mpint(e) + mpint(f)
                                  + mpint(shared)).digest()
        self.session_id = self.session_id or exchange
        signature = string(b"rsa-sha2-256") + string(self.server.sign(exchange))
        self.send(bytes([31]) + string(host) + mpint(f) + string(signature))
        self.send(bytes([21]))
        if self.read_packet()[:1] != bytes([21]):
            raise ValueError("expected NEWKEYS")

        def key(letter: bytes) -> tuple[bytes, bytes]:
            first = hashlib.sha256(mpint(shared) + exchange + letter + self.session_id).digest()
            material = first + hashlib.sha256(mpint(shared) + exchange + first).digest()
            return material[:32], material[32:64]

        self.send_keys, self.recv_keys = key(b"D"), key(b"C")
        if self.server.sig_algs is not None:
            self.send(bytes([7]) + u32(1) + string(b"server-sig-algs") + string(self.server.sig_algs))

    def serve(self) -> None:
        try:
            self.key_exchange()
            while True:
                self.dispatch(self.read_packet())
        except (EOFError, OSError, ValueError):
            pass
        finally:
            for channel in list(self.channels.values()):
                channel.terminate()
            self.sock.close()

    def dispatch(self, payload: bytes) -> None:
        message = Reader(payload)
        kind = message.byte()
        if kind == 1:
            raise EOFError
        if kind == 5:
            self.send(bytes([6]) + string(message.string()))
        elif kind == 50:
            self.authenticate(message)
        elif kind == 80:
            message.string()
            if message.byte():
                self.send(bytes([82]))
        elif kind == 90:
            message.string()
            peer, window, packet = message.u32(), message.u32(), message.u32()
            number, self.next_channel = self.next_channel, self.next_channel + 1
            self.channels[number] = Channel(self, number, peer, window, packet)
            self.send(bytes([91]) + u32(peer) + u32(number) + u32(1 << 21) + u32(32768))
        elif kind in (93, 94, 96, 97, 98):
            channel = self.channels.get(message.u32())
            if channel is not None:
                channel.handle(kind, message)

    def authenticate(self, message: Reader) -> None:
        user, service, method = message.string(), message.string(), message.string()
        offered = self.server.methods
        entry: dict = {"method": method.decode("latin-1")}
        accepted = False
        if method == b"password" and "password" in offered:
            change, password = message.byte(), message.string()
            accepted = not change and hmac.compare_digest(password, self.server.password)
            entry["accepted"] = accepted
        elif method == b"publickey" and "publickey" in offered:
            signed, algorithm, blob = message.byte(), message.string(), message.string()
            entry["signed"] = bool(signed)
            entry["algorithm"] = algorithm.decode("latin-1")
            if blob in self.server.authorized and algorithm in ALGORITHMS:
                if not signed:
                    self.server.log(entry)
                    self.send(bytes([60]) + string(algorithm) + string(blob))
                    return
                data = (string(self.session_id) + bytes([50]) + string(user) + string(service)
                        + string(b"publickey") + b"\x01" + string(algorithm) + string(blob))
                accepted = verify(blob, algorithm, data, Reader(message.string()))
            entry["accepted"] = accepted
        self.server.log(entry)
        if accepted:
            self.send(bytes([52]))
        else:
            self.send(bytes([51]) + string(",".join(offered).encode()) + b"\x00")


def verify(blob: bytes, algorithm: bytes, data: bytes, signature: Reader) -> bool:
    key = Reader(blob)
    kind = key.string()
    if signature.string() != algorithm:
        return False
    if kind == b"ssh-dss" and algorithm == b"ssh-dss":
        p, q, g, y = key.mpint(), key.mpint(), key.mpint(), key.mpint()
        value = signature.string()
        r, s = int.from_bytes(value[:20], "big"), int.from_bytes(value[20:], "big")
        if len(value) != 40 or not (0 < r < q and 0 < s < q):
            return False
        w, digest = pow(s, -1, q), int.from_bytes(hashlib.sha1(data).digest(), "big")
        return pow(g, digest * w % q, p) * pow(y, r * w % q, p) % p % q == r
    if kind != b"ssh-rsa" or algorithm == b"ssh-dss":
        return False
    e, n = key.mpint(), key.mpint()
    length = (n.bit_length() + 7) // 8
    value = int.from_bytes(signature.string(), "big")
    return value < n and pow(value, e, n).to_bytes(length, "big") == encoded_digest(data, algorithm, length)


class Channel:
    def __init__(self, connection: Connection, number: int, peer: int, window: int, packet: int) -> None:
        self.connection, self.number, self.peer = connection, number, peer
        self.window, self.packet = window, packet
        self.changed = threading.Condition()
        self.received = 0
        self.process: subprocess.Popen | None = None
        self.closed = self.sent_close = False

    def handle(self, kind: int, message: Reader) -> None:
        if kind == 93:
            with self.changed:
                self.window += message.u32()
                self.changed.notify_all()
        elif kind == 94 and self.process is not None:
            data = message.string()
            try:
                self.process.stdin.write(data)
                self.process.stdin.flush()
            except OSError:
                pass
            self.received += len(data)
            if self.received >= 1 << 20:
                self.connection.send(bytes([93]) + u32(self.peer) + u32(self.received))
                self.received = 0
        elif kind == 96 and self.process is not None:
            try:
                self.process.stdin.close()
            except OSError:
                pass
        elif kind == 97:
            self.terminate()
            self.close()
        elif kind == 98:
            request, reply = message.string(), message.byte()
            if request == b"exec" and self.process is None:
                self.process = subprocess.Popen(["sh", "-c", message.string()], stdin=subprocess.PIPE,
                                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
                if reply:
                    self.connection.send(bytes([99]) + u32(self.peer))
                threading.Thread(target=self.run, daemon=True).start()
            elif reply:
                self.connection.send(bytes([100]) + u32(self.peer))

    def forward(self, stream, extended: bool) -> None:
        while chunk := os.read(stream.fileno(), 16384):
            while chunk:
                with self.changed:
                    while self.window == 0 and not self.closed:
                        self.changed.wait()
                    if self.closed:
                        return
                    size = min(len(chunk), self.window, self.packet - 64)
                    self.window -= size
                part, chunk = chunk[:size], chunk[size:]
                head = bytes([95]) + u32(self.peer) + u32(1) if extended else bytes([94]) + u32(self.peer)
                self.connection.send(head + string(part))

    def run(self) -> None:
        try:
            errors = threading.Thread(target=self.forward, args=(self.process.stderr, True), daemon=True)
            errors.start()
            self.forward(self.process.stdout, False)
            errors.join()
            status = self.process.wait()
            if not self.closed:
                self.connection.send(bytes([98]) + u32(self.peer) + string(b"exit-status") + b"\x00" + u32(status))
                self.connection.send(bytes([96]) + u32(self.peer))
                self.close()
        except OSError:
            self.terminate()

    def close(self) -> None:
        with self.changed:
            if self.sent_close:
                return
            self.sent_close = True
        self.connection.send(bytes([97]) + u32(self.peer))

    def terminate(self) -> None:
        with self.changed:
            self.closed = True
            self.changed.notify_all()
        if self.process is not None and self.process.poll() is None:
            self.process.kill()


class Server:
    def __init__(self, config: dict) -> None:
        self.password = config["password"].encode("latin-1")
        self.methods = list(config["methods"])
        self.authorized = {base64.b64decode(blob) for blob in config.get("authorized", [])}
        self.log_path = config["log"]
        sig_algs = config.get("server_sig_algs")
        self.sig_algs = None if sig_algs is None else sig_algs.encode()
        self.log_lock = threading.Lock()
        self.n, self.e, self.d = rsa_key()
        self.host_blob = string(b"ssh-rsa") + mpint(self.e) + mpint(self.n)

    def sign(self, message: bytes) -> bytes:
        length = (self.n.bit_length() + 7) // 8
        value = int.from_bytes(encoded_digest(message, b"rsa-sha2-256", length), "big")
        return pow(value, self.d, self.n).to_bytes(length, "big")

    def log(self, entry: dict) -> None:
        with self.log_lock, open(self.log_path, "a", encoding="utf-8") as log:
            log.write(json.dumps(entry) + "\n")


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
    with open(sys.argv[1], encoding="utf-8") as config:
        server = Server(json.load(config))
    listener = socket.socket()
    listener.bind(("127.0.0.1", 0))
    listener.listen(64)
    host_key = "ssh-rsa " + base64.b64encode(server.host_blob).decode()
    print(json.dumps({"port": listener.getsockname()[1], "host_key": host_key}), flush=True)
    while True:
        sock, _ = listener.accept()
        threading.Thread(target=Connection(sock, server).serve, daemon=True).start()


if __name__ == "__main__":
    main()
