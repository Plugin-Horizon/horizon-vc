#!/usr/bin/env python3
"""Bare voice chat test client: two or three offline-mode players join a server, open voice
connections and talk; prints a transcript with UUIDs replaced by names, so two runs can be
compared line by line.

Usage: vc_probe.py <game port> <screen session of the server console>
Needs the `cryptography` package. Players are teleported with console commands (screen stuff).
"""
import hashlib, os, re, socket, struct, subprocess, sys, time, uuid, zlib

from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def varint(n):
    n &= 0xFFFFFFFF
    out = b""
    while True:
        b, n = n & 0x7F, n >> 7
        out += bytes([b | 0x80]) if n else bytes([b])
        if not n:
            return out


def parse_varint(buf, i=0):
    n = shift = 0
    while True:
        b = buf[i]; i += 1
        n |= (b & 0x7F) << shift
        if not b & 0x80:
            return n, i
        shift += 7


def string(s):
    b = s.encode()
    return varint(len(b)) + b


def offline_uuid(name):
    h = bytearray(hashlib.md5(("OfflinePlayer:" + name).encode()).digest())
    h[6] = (h[6] & 0x0F) | 0x30
    h[8] = (h[8] & 0x3F) | 0x80
    return uuid.UUID(bytes=bytes(h))


VOICE_CHANNELS = ["voicechat:secret", "voicechat:states", "voicechat:state", "voicechat:remove_state",
                  "voicechat:add_group", "voicechat:remove_group", "voicechat:joined_group",
                  "voicechat:add_category", "voicechat:remove_category"]


class Player:
    """A protocol 777 (26.3) client: play state, keep-alives, teleports, custom payloads, voice UDP."""

    def __init__(self, port, name):
        self.name, self.uuid = name, offline_uuid(name)
        self.s = socket.create_connection(("127.0.0.1", port), timeout=30)
        self.t, self.buf = None, b""
        self.payloads, self.chat, self.sounds, self.voice_ids = [], [], [], []
        self.pos, self.last_move, self.udp, self.secret = None, 0.0, None, None
        self.send(0, varint(777) + string("127.0.0.1") + struct.pack(">H", port) + varint(2))
        self.send(0, string(name) + self.uuid.bytes)
        while True:
            pid, d = self.recv()
            if pid == 3:
                self.t = parse_varint(d)[0]
            elif pid == 2:
                break
            elif pid == 0:
                raise SystemExit(f"login refused: {d!r}")
        self.send(3)
        while True:
            pid, d = self.recv()
            if pid == 15:
                self.send(7, varint(0))
            elif pid in (4, 5):
                self.send(pid, d)
            elif pid == 3:
                self.send(3)
                break
            elif pid == 2:
                raise SystemExit(f"disconnected in configuration: {d!r}")
        self.s.settimeout(0.02)
        self.custom("minecraft:register", b"\0".join(c.encode() for c in VOICE_CHANNELS))
        self.send(44)

    def exact(self, n):
        while len(self.buf) < n:
            c = self.s.recv(65536)
            if not c:
                raise EOFError
            self.buf += c
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def vi(self):
        n = shift = 0
        while True:
            b = self.exact(1)[0]
            n |= (b & 0x7F) << shift
            if not b & 0x80:
                return n
            shift += 7

    def send(self, pid, body=b""):
        data = varint(pid) + body
        if self.t is not None and self.t >= 0:
            data = varint(len(data)) + zlib.compress(data) if len(data) >= self.t else varint(0) + data
        self.s.sendall(varint(len(data)) + data)

    def recv(self):
        data = self.exact(self.vi())
        if self.t is not None and self.t >= 0:
            size, i = parse_varint(data)
            data = zlib.decompress(data[i:]) if size else data[i:]
        pid, i = parse_varint(data)
        return pid, data[i:]

    def custom(self, channel, data):
        self.send(22, string(channel) + data)

    # Voice: client frames are 0xFF, player UUID, VarInt length, nonce(12) + AES-GCM.
    def voice(self, pid, body=b""):
        iv = os.urandom(12)
        enc = iv + AESGCM(self.secret).encrypt(iv, bytes([pid]) + body, None)
        self.udp.sendto(b"\xff" + self.uuid.bytes + varint(len(enc)) + enc, self.voice_addr)

    def connect_voice(self, request_version=20):
        self.custom("voicechat:request_secret", struct.pack(">i", request_version))
        end = time.time() + 3
        while time.time() < end and not any(ch == "voicechat:secret" for ch, _ in self.payloads):
            self.pump()
        sec = [d for ch, d in self.payloads if ch == "voicechat:secret"]
        if not sec:
            return None
        d = sec[-1]
        self.secret = d[:16]
        port = struct.unpack(">i", d[16:20])[0]
        self.voice_addr = ("127.0.0.1", port)
        self.udp = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.udp.settimeout(0.01)
        self.voice(5, self.uuid.bytes + self.secret)
        return d

    def mic(self, seq, whisper=False, audio=bytes(range(40))):
        self.voice(1, varint(len(audio)) + audio + struct.pack(">q", seq) + (b"\x01" if whisper else b"\x00"))

    def pump(self):
        while True:
            try:
                pid, d = self.recv()
            except (socket.timeout, BlockingIOError):
                break
            if pid == 45:
                self.send(28, d)
            elif pid == 73:
                tid, i = parse_varint(d)
                self.pos = struct.unpack(">ddd", d[i:i + 24])
                self.send(0, varint(tid) + struct.pack(">dddff", *self.pos, 0.0, 0.0)); self.send(44)
            elif pid == 24:
                n, i = parse_varint(d)
                self.payloads.append((d[i:i + n].decode(), d[i + n:]))
            elif pid == 124:
                self.chat.append(re.sub(rb"[^\x20-\x7e]+", b" ", d).decode().strip())
            elif pid == 32:
                raise SystemExit(f"{self.name} disconnected: {d!r}")
        if self.pos and time.time() - self.last_move > 0.25:
            self.send(31, struct.pack(">dddffB", *self.pos, 0.0, 0.0, 1)); self.send(13)
            self.last_move = time.time()
        while self.udp:
            try:
                raw = self.udp.recvfrom(65536)[0]
            except (socket.timeout, BlockingIOError):
                break
            n, i = parse_varint(raw, 1)
            assert raw[0] == 0xFF and len(raw) == i + n, "server frame: 0xFF, VarInt length, sealed"
            plain = AESGCM(self.secret).decrypt(raw[i:i + 12], raw[i + 12:i + n], None)
            self.voice_ids.append(plain[0])
            if plain[0] == 8:
                self.voice(8)
            elif plain[0] in (2, 3):
                self.sounds.append(plain)


class Run:
    def __init__(self, port, session):
        self.port, self.session, self.players, self.names = port, session, [], {}

    def console(self, cmd):
        subprocess.run(["screen", "-S", self.session, "-X", "stuff", cmd + "\r"], check=True)

    def join(self, name):
        p = Player(self.port, name)
        self.players.append(p)
        self.names[p.uuid.hex] = name
        return p

    def pump(self, secs):
        end = time.time() + secs
        while True:
            for p in self.players:
                p.pump()
            if time.time() >= end:
                return
            time.sleep(0.01)

    def norm(self, data):
        h = data.hex()
        for u, n in sorted(self.names.items(), key=lambda kv: kv[1]):
            h = h.replace(u, f"<{n}>")
        return h

    def take(self, p):
        for ch, d in p.payloads:
            # Group ids are random: name them in order of appearance.
            if ch == "voicechat:add_group" and d[:16].hex() not in self.names:
                self.names[d[:16].hex()] = f"G{sum(1 for v in self.names.values() if v.startswith('G')) + 1}"
        out = [f"{ch.split(':')[1]}={self.norm(d)}" for ch, d in p.payloads]
        p.payloads.clear()
        return out

    def heard(self, speaker, label, whisper=False, setup=()):
        for cmd in setup:
            self.console(cmd)
        self.pump(1.2)
        for p in self.players:
            p.sounds.clear()
        speaker.mic(int(time.time() * 10), whisper)
        self.pump(0.5)
        got = []
        for p in self.players:
            for s in p.sounds:
                kind = "player" if s[0] == 2 else "group"
                tail = s[-5:].hex() if s[0] == 2 else s[-1:].hex()
                got.append(f"{p.name}:{kind}:{self.norm(s[1:33])}:{tail}")
        print(f"{label:44} -> {sorted(got)}")


def main():
    port, session = int(sys.argv[1]), sys.argv[2]
    r = Run(port, session)
    a = r.join("ProbeA")
    r.console("tp ProbeA 0 -60 0")
    r.pump(1.5)
    print("A on join:", r.take(a))
    print("A asks with version 1, chat:", end=" ")
    a.custom("voicechat:request_secret", struct.pack(">i", 1)); r.pump(1)
    print([c for c in a.chat if "compatible" in c]); r.take(a)
    sec = a.connect_voice()
    if sec is None:
        print("A got no voicechat:secret in 3 s: no voice chat on this server")
        print("A payloads:", r.take(a))
        return
    print("A secret fields:", r.norm(sec[16:]))
    print("A before the secret:", [x for x in r.take(a) if not x.startswith("secret")])
    r.pump(0.3)
    print("A voice ids after authenticate:", a.voice_ids); a.voice_ids.clear()
    a.voice(9); r.pump(0.5)
    print("A voice ids after connection check:", a.voice_ids[:1], "state:", r.take(a))
    b = r.join("ProbeB")
    r.console("tp ProbeB 2 -60 0")
    r.pump(1.5)
    print("A when B joined:", r.take(a))
    b.connect_voice(); r.pump(0.3); b.voice(9); r.pump(1.5)
    print("B states/secret order:", [x.split("=")[0] for x in r.take(b)])
    print("A when B connected voice:", r.take(a))
    print("keep-alives in 2.5 s:", end=" ")
    a.voice_ids.clear(); r.pump(2.5); print(a.voice_ids.count(8) >= 2)
    r.heard(a, "B 2 blocks away", setup=["tp ProbeB 2 -60 0"])
    r.heard(a, "B 49.0 away", setup=["tp ProbeB 49.5 -60 0.5", "tp ProbeA 0.5 -60 0.5"])
    r.heard(a, "B 49.0 away, whisper", whisper=True)
    r.heard(a, "B 49.25 away", setup=["tp ProbeB 49.75 -60 0.5"])
    r.heard(a, "B in the nether", setup=["execute in minecraft:the_nether run tp ProbeB 0 70 0"])
    r.heard(a, "B back, A spectator", setup=["execute in minecraft:overworld run tp ProbeB 2.5 -60 0.5",
                                             "gamemode spectator ProbeA"])
    r.console("gamemode survival ProbeA")
    r.take(a); r.take(b)
    b.custom("voicechat:update_state", b"\x01"); r.pump(1)
    print("A when B disabled:", r.take(a))
    r.heard(a, "B disabled, A speaks")
    r.heard(b, "B disabled, B speaks")
    b.custom("voicechat:update_state", b"\x00"); r.pump(1); r.take(a); r.take(b)
    c = r.join("ProbeC")
    r.console("tp ProbeC 4 -60 0")
    r.pump(1.5); c.connect_voice(); r.pump(0.3); c.voice(9); r.pump(1.5)
    for p in (a, b, c):
        r.take(p)
    a.custom("voicechat:create_group", string("team") + b"\x01" + string("pw") + struct.pack(">h", 0)); r.pump(1)
    print("create group, A:", r.take(a))
    print("create group, B:", r.take(b))
    gid = bytes.fromhex([k for k, v in r.names.items() if v == "G1"][0])
    b.custom("voicechat:set_group", gid + b"\x00"); r.pump(1)
    print("B joins without password:", r.take(b))
    b.custom("voicechat:set_group", gid + b"\x01" + string("pw")); r.pump(1)
    print("B joins with password, B:", r.take(b), "C:", r.take(c))
    r.take(a)
    r.heard(a, "group: A speaks, B member, C outsider")
    r.heard(c, "group: C (outsider) speaks")
    r.heard(a, "group: B 200 blocks away", setup=["tp ProbeB 200 -60 0"])
    r.console("tp ProbeB 2 -60 0")
    b.custom("voicechat:leave_group", b""); r.pump(1)
    print("B leaves, B:", r.take(b), "A:", r.take(a))
    a.custom("voicechat:leave_group", b""); r.pump(1)
    print("A leaves too, C:", r.take(c))
    r.take(a); r.take(b)
    b.s.close(); r.players.remove(b); r.pump(1.5)
    print("B quit, A:", r.take(a))


if __name__ == "__main__":
    main()
