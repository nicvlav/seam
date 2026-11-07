"""Checks a VST3 build of the example EQ as JUCE's VST3 hosting loads it:
the parity script's audio, bit for bit against the Rust EQ driven directly,
and a saved state that reloads the same.

    python bench/scripts/hosted.py --tool bench/build/BenchHosted_artefacts/Release/BenchHosted.exe \\
        --bundle "target/vst3/Seam Example EQ.vst3" --ids seam

--ids says how the build names parameters to hosts: `seam` for Seam's own
exporter (B: Seam's hash of the key), `juce` for a build JUCE's wrapper makes
(A and C: JUCE's hash of the key, which JUCE offers no way to change). Every
key's id must be one the build lists, so a wrong scheme fails at once. Exits
1 on any mismatch.
"""

import argparse
import pathlib
import subprocess
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import parity  # noqa: E402


def seam_id(key):
    """seam_params::host_id: 32-bit FNV-1a over the key's UTF-8, top bit cleared."""
    value = 0x811C9DC5
    for byte in key.encode():
        value = ((value ^ byte) * 0x01000193) & 0xFFFFFFFF
    return value & 0x7FFFFFFF


def juce_id(key):
    """seam_host::cabi::juce_param_id: JUCE's String::hashCode, top bit cleared."""
    value = 0
    for character in key:
        value = (value * 31 + ord(character)) & 0xFFFFFFFF
    return value & 0x7FFFFFFF


def hosted(tool, bundle, *arguments, lines=None):
    return subprocess.run([str(tool), str(bundle), *arguments],
                          input=None if lines is None else "\n".join(lines) + "\n",
                          capture_output=True, text=True)


def check(tool, bundle, ids):
    """Checks one build as `main` does. Returns whether it matches, and the
    lines that say so."""
    id_of = seam_id if ids == "seam" else juce_id
    said = []
    info = hosted(tool, bundle, "info")
    if info.returncode != 0:
        return False, [info.stderr.strip()]
    listed = {line.split("|")[0] for line in info.stdout.splitlines()
              if "|" in line and line.split("|")[0].isdigit()}
    keys = [line.split()[0] for line in parity.rust("ranges")]
    missing = [key for key in keys if str(id_of(key)) not in listed]
    if missing:
        return False, [f"ids not listed by the build: {', '.join(missing)}"]
    said.append(f"{len(keys)} parameters listed by their {ids} ids, of {len(listed)}")

    state = hosted(tool, bundle, "state", str(id_of("low.gain")))
    said.append(state.stdout.strip().splitlines()[-1] if state.stdout.strip() else state.stderr.strip())
    ok = state.returncode == 0

    for rate, seed in [(44100, 1), (48000, 2), (96000, 3)]:
        lines = parity.script(rate, seed)
        named = [f"set {id_of(line.split()[1])} {line.split()[2]}" if line.startswith("set ") else line
                 for line in lines]
        expected = parity.rust("audio", lines)
        run = hosted(tool, bundle, "audio", lines=named)
        got = [line.rstrip("\r") for line in run.stdout.splitlines()]
        rms, peak = parity.difference_db(expected, got)
        said.append(f"hosted audio at {rate} Hz: difference {rms:.1f} dBFS RMS, {peak:.1f} dBFS peak")
        mismatches = sum(e != g for e, g in zip(expected, got)) + abs(len(expected) - len(got))
        said.append(f"hosted audio at {rate} Hz: {len(expected)} cases, {mismatches} mismatches")
        ok &= run.returncode == 0 and mismatches == 0

    said.append("the hosted build matches" if ok else "the hosted build does NOT match")
    return ok, said


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--tool", required=True, type=pathlib.Path, help="the built bench-hosted")
    parser.add_argument("--bundle", required=True, type=pathlib.Path, help="the .vst3 to load")
    parser.add_argument("--ids", required=True, choices=["seam", "juce"])
    arguments = parser.parse_args()
    ok, said = check(arguments.tool, arguments.bundle, arguments.ids)
    print("\n".join(said))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
