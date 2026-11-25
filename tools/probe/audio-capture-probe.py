#!/usr/bin/env python3
"""Exercise the Husk AAudio capture bridge end to end.

This probe checks the wire protocol, not whether the room is quiet. A PASS
means OPEN/READ/CLOSE completed with exactly the requested PCM bytes; the
reported peak and silent_chunks are evidence about the captured samples.

Usage (guest): python3 audio-capture-probe.py [socket] [seconds]
"""
import socket
import sys
import time

SOCKET = sys.argv[1] if len(sys.argv) > 1 else "/run/husk/audio-in.sock"
SECONDS = float(sys.argv[2]) if len(sys.argv) > 2 else 1.0
RATE = 48_000
CHANNELS = 2
FRAMES = 1024


def recv_line(sock):
    data = bytearray()
    while True:
        chunk = sock.recv(1)
        if not chunk:
            raise RuntimeError("peer closed while reading a line")
        if chunk == b"\n":
            return data.decode("utf-8", "replace")
        if chunk != b"\r":
            data.extend(chunk)
            if len(data) > 4096:
                raise RuntimeError("response line too long")


def request(sock, line):
    sock.sendall(line.encode() + b"\n")
    return recv_line(sock)


def fields(line):
    return dict(item.split("=", 1) for item in line.split() if "=" in item)


sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.settimeout(5.0)
try:
    sock.connect(SOCKET)
    pong = request(sock, "PING")
    print(pong)
    if not pong.startswith("PONG"):
        raise RuntimeError(f"bad PING response: {pong}")

    ready = request(sock, f"OPEN rate={RATE} channels={CHANNELS}")
    print(ready)
    if not ready.startswith("CAPTURE_READY"):
        raise RuntimeError(f"OPEN failed: {ready}")

    started = time.monotonic()
    chunks = total_frames = total_bytes = silent_chunks = 0
    peak = 0
    while time.monotonic() - started < SECONDS:
        reply = request(sock, f"READ frames={FRAMES}")
        print(reply)
        if not reply.startswith("PCM_OK"):
            raise RuntimeError(f"READ failed: {reply}")
        count = int(fields(reply)["bytes"])
        pcm = bytearray()
        while len(pcm) < count:
            chunk = sock.recv(count - len(pcm))
            if not chunk:
                raise RuntimeError("peer closed while reading PCM")
            pcm.extend(chunk)
        chunk_peak = 0
        for offset in range(0, len(pcm) - 1, 2):
            sample = int.from_bytes(pcm[offset:offset + 2], "little", signed=True)
            chunk_peak = max(chunk_peak, abs(sample))
        peak = max(peak, chunk_peak)
        if chunk_peak == 0:
            silent_chunks += 1
        chunks += 1
        total_frames += int(fields(reply)["frames"])
        total_bytes += count

    closed = request(sock, "CLOSE")
    print(closed)
    if not closed.startswith("CAPTURE_OK"):
        raise RuntimeError(f"CLOSE failed: {closed}")

    elapsed_ms = round((time.monotonic() - started) * 1000)
    print(
        "capture-probe|PASS|"
        f"chunks={chunks}|frames={total_frames}|bytes={total_bytes}|"
        f"silent_chunks={silent_chunks}|peak={peak}|elapsed_ms={elapsed_ms}"
    )
finally:
    sock.close()
