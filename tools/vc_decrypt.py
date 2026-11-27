#!/usr/bin/env python3
"""Decrypt the UDP datagrams of a capture made by realclient/vcbench/vc_capture.py with the secret that the same capture
holds (the last `voicechat:secret` payload, first 16 bytes), and print the plain packets: id and hex.
Usage: vc_decrypt.py <capture.jsonl> [--ids 2,3,4] [--since <unix time>]
Needs the `cryptography` package. Datagram: 0xFF, [player uuid when client to server], varint length, nonce(12) + ciphertext + tag."""
import json, sys
from cryptography.hazmat.primitives.ciphers.aead import AESGCM


def varint(b, i):
    n = s = 0
    while True:
        c = b[i]; i += 1
        n |= (c & 0x7F) << s
        if not c & 0x80:
            return n, i
        s += 7


args = sys.argv[1:]
ids = {int(x) for x in args[args.index("--ids") + 1].split(",")} if "--ids" in args else None
since = float(args[args.index("--since") + 1]) if "--since" in args else 0
rows = [json.loads(l) for l in open(args[0])]
secret = None
for r in rows:
    if r["kind"] == "tcp" and r["chan"] == "voicechat:secret" and r["dir"] == "s2c":
        secret = bytes.fromhex(r["hex"])[:16]
    if r["kind"] != "udp" or secret is None or r["t"] < since:
        continue
    raw = bytes.fromhex(r["hex"])
    if raw[0] != 0xFF:
        continue
    i = 1 + (16 if r["dir"] == "c2s" else 0)
    n, i = varint(raw, i)
    try:
        plain = AESGCM(secret).decrypt(raw[i:i + 12], raw[i + 12:i + n], None)
    except Exception:
        continue
    if ids is None or plain[0] in ids:
        print(r["dir"], r["t"], plain[0], plain.hex())
