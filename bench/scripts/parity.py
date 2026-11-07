"""Checks that the JUCE EQ in bench/ stores and computes what the Rust EQ
does, bit for bit, before any audio is compared: one ulp of difference in a
parameter value or a coefficient already breaks the comparison's bar of
-90 dBFS peak at low frequencies.

    python bench/scripts/parity.py --tool bench/build/BenchParity_artefacts/Release/BenchParity.exe

Runs the Rust side (examples/eq/examples/parity/) in release, hands its
cases to bench-parity, and compares the answers. Exits 1 on any mismatch.
"""

import argparse
import math
import pathlib
import random
import struct
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]


def rust(mode, lines=None):
    run = subprocess.run(
        ["cargo", "run", "-q", "--release", "--locked", "-p", "seam-example-eq", "--example", "parity", "--", mode],
        cwd=ROOT, input=None if lines is None else "\n".join(lines) + "\n",
        capture_output=True, text=True, check=True)
    return run.stdout.splitlines()


def juce(tool, mode, lines):
    run = subprocess.run([str(tool), mode], input="\n".join(lines) + "\n",
                         capture_output=True, text=True, check=True)
    return [line.rstrip("\r") for line in run.stdout.splitlines()]


def as_float(hex_bits):
    return struct.unpack("<f", struct.pack("<I", int(hex_bits, 16)))[0]


def compare(name, expected, got, describe):
    mismatches = [(e, g) for e, g in zip(expected, got) if e != g]
    if len(expected) != len(got):
        print(f"{name}: {len(expected)} cases, {len(got)} answers")
        return False
    print(f"{name}: {len(expected)} cases, {len(mismatches)} mismatches")
    for e, g in mismatches[:5]:
        print(f"  Rust {describe(e)}\n  JUCE {describe(g)}")
    return not mismatches


def values(line):
    key, given, result = line.split()
    return f"{key}: {as_float(given)!r} -> {as_float(result)!r} ({result})"


def coefficients(line):
    shape, gain, frequency, q, rate, *rest = line.split()
    return (f"{shape} gain {as_float(gain)} f {as_float(frequency)} q {as_float(q)}: "
            + " ".join(rest))


def script(rate, seed):
    """Blocks of every size from 1 to 1,024 frames, with host writes of
    random 0..1 values (each an f32, as a JUCE parameter takes it) to random
    parameters between them, and an occasional reset."""
    chooser = random.Random(seed)
    keys = [line.split()[0] for line in rust("ranges")]
    bools = {line.split()[0] for line in rust("ranges") if line.split()[1] == "bool"}
    lines = [f"rate {rate}"]
    for _ in range(600):
        for _ in range(chooser.choice([0, 0, 1, 1, 2, 3])):
            key = chooser.choice(keys)
            value = float(chooser.random() < 0.7) if key in bools else chooser.random()
            bits = struct.unpack("<I", struct.pack("<f", value))[0]
            lines.append(f"set {key} {bits:08x}")
        if chooser.random() < 0.01:
            lines.append("reset")
        lines.append(f"block {chooser.choice([1, 16, 32, 37, 64, 128, 256, 512, 999, 1024])}")
    return lines


def difference_db(expected, got):
    """The RMS and peak of the difference, in dBFS."""
    total, peak, count = 0.0, 0.0, 0
    for e, g in zip(expected, got):
        for a, b in zip(e.split(), g.split()):
            delta = as_float(a) - as_float(b)
            total += delta * delta
            peak = max(peak, abs(delta))
            count += 1
    rms = math.sqrt(total / max(count, 1))
    db = lambda x: 20 * math.log10(x) if x > 0 else float("-inf")
    return db(rms), db(peak)


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--tool", required=True, type=pathlib.Path, help="the built bench-parity")
    tool = parser.parse_args().tool

    ok = compare("ranges", rust("ranges"), juce(tool, "ranges", []), lambda line: line)

    snapped = rust("snap")
    inputs = [" ".join(line.split()[:2]) for line in snapped]
    ok &= compare("snap", snapped, juce(tool, "snap", inputs), values)
    ok &= compare("restore", snapped, juce(tool, "restore", inputs), values)

    normalized = rust("normalized")
    inputs = [" ".join(line.split()[:2]) for line in normalized]
    ok &= compare("normalized", normalized, juce(tool, "normalized", inputs), values)

    computed = rust("coefficients")
    inputs = [" ".join(line.split()[:5]) for line in computed]
    ok &= compare("coefficients", computed, juce(tool, "coefficients", inputs), coefficients)

    for rate, seed in [(44100, 1), (48000, 2), (96000, 3)]:
        lines = script(rate, seed)
        expected, got = rust("audio", lines), juce(tool, "audio", lines)
        rms, peak = difference_db(expected, got)
        print(f"audio at {rate} Hz: difference {rms:.1f} dBFS RMS, {peak:.1f} dBFS peak")
        ok &= compare(f"audio at {rate} Hz", expected, got, lambda line: line)

    print("parity holds" if ok else "parity does NOT hold")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
