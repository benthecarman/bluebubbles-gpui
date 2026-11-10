#!/usr/bin/env python3
"""Synthesizes the notification sounds in assets/sounds/.

They are made here from sine partials, so they are this project's own work
under its license. Needs ffmpeg for the Ogg Vorbis encoding.

Usage: scripts/make-sounds.py
"""

import math
import os
import struct
import subprocess
import tempfile
import wave

RATE = 48000
OUT = os.path.join(os.path.dirname(__file__), "..", "assets", "sounds")


def note(freq, start, length, partials, attack=0.004):
    """A struck tone: (ratio, amplitude, decay seconds) partials."""
    out = []
    for i in range(int(length * RATE)):
        t = i / RATE
        env = min(1.0, t / attack)
        out.append(
            (start + t,
             env * sum(a * math.exp(-t / d) * math.sin(2 * math.pi * freq * r * t)
                       for r, a, d in partials)))
    return out


def mix(notes, length):
    buf = [0.0] * int(length * RATE)
    for n in notes:
        for at, v in n:
            i = int(at * RATE)
            if i < len(buf):
                buf[i] += v
    # Peak at -4 dBFS, with a short fade so it ends in silence.
    peak = max(abs(v) for v in buf) or 1.0
    gain = 10 ** (-4 / 20) / peak
    fade = int(0.02 * RATE)
    for i in range(fade):
        buf[-1 - i] *= i / fade
    return [v * gain for v in buf]


MARIMBA = [(1.0, 1.0, 0.18), (3.93, 0.22, 0.05), (9.2, 0.04, 0.02)]
BELL = [(1.0, 1.0, 0.45), (2.0, 0.35, 0.25), (2.76, 0.18, 0.12), (5.4, 0.06, 0.05)]
GLASS = [(1.0, 1.0, 0.6), (2.32, 0.3, 0.3), (4.25, 0.12, 0.15), (6.8, 0.05, 0.08)]


def chime():
    """Two bell notes a fourth apart, rising."""
    return mix([note(1318.5, 0.0, 0.9, BELL), note(1760.0, 0.11, 0.9, BELL)], 1.0)


def marimba():
    """Three quick marimba notes, rising: a major triad."""
    return mix([note(1046.5, 0.0, 0.5, MARIMBA),
                note(1318.5, 0.085, 0.5, MARIMBA),
                note(1568.0, 0.17, 0.5, MARIMBA)], 0.7)


def glass():
    """One clear, high glass ping."""
    return mix([note(1975.5, 0.0, 1.0, GLASS)], 1.0)


def pop():
    """A soft bubble pop: a sine sweeping up as it fades."""
    out = []
    phase = 0.0
    for i in range(int(0.16 * RATE)):
        t = i / RATE
        freq = 520 + 680 * (1 - math.exp(-t / 0.025))
        phase += 2 * math.pi * freq / RATE
        env = min(1.0, t / 0.003) * math.exp(-t / 0.045)
        out.append((t, env * math.sin(phase)))
    return mix([out], 0.18)


SOUNDS = {"chime": chime, "marimba": marimba, "glass": glass, "pop": pop}


def main():
    os.makedirs(OUT, exist_ok=True)
    for name, make in SOUNDS.items():
        samples = make()
        with tempfile.NamedTemporaryFile(suffix=".wav") as tmp:
            with wave.open(tmp.name, "wb") as w:
                w.setnchannels(1)
                w.setsampwidth(2)
                w.setframerate(RATE)
                w.writeframes(b"".join(
                    struct.pack("<h", int(max(-1, min(1, v)) * 32767)) for v in samples))
            dest = os.path.join(OUT, f"{name}.oga")
            subprocess.run(
                ["ffmpeg", "-v", "error", "-y", "-i", tmp.name, "-c:a", "libvorbis",
                 "-q:a", "6", "-map_metadata", "-1", dest],
                check=True)
        print(dest)


if __name__ == "__main__":
    main()
