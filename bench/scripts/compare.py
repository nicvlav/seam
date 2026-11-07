"""Runs the comparison on the bare EQ: A, B and C; A with JUCE's own ranges
(A-native); the ladder's builds, A-native with the Rust EQ's filters ported
to C++ (A-port), and A-port and C with their C++ built by clang-cl; each
loaded by the console host (bench-hosted), played and measured the same way,
and written up.

    python bench/scripts/compare.py --tool bench/build/BenchHosted_artefacts/Release/BenchHosted.exe

Build A, A-native, A-port and C first, and A-port and C in a folder
configured with clang-cl (bench/README.md). B's module is built here with
cargo and put in a bundle under --out, which `cargo xtask bundle` will do
once it exists.

1. A, B, C and C-clang must first play the parity script bit for bit as the
   Rust EQ driven directly does (scripts/hosted.py); the timing waits on it.
   A-native and the A-port builds store values a little apart from Seam's,
   so they are timed beside them but kept out of that check.
2. Each measurement runs in a fresh process: a load from the build's
   description, then real-time-paced blocks on an audio thread while the main
   thread runs the message loop (hosted.cpp, measure), the audio thread
   spinning or sleeping between blocks. Builds take turns, in an order that
   rotates every round, at every pace, block size and scenario.
3. Every run's output is kept until the other builds have played the same
   run. A, B, C and C-clang must match each other bit for bit in every run,
   and so must A-native and the A-port builds.

Writes report.json (every number) and report.md (the tables) into --out.
"""

import argparse
import datetime
import json
import math
import pathlib
import platform
import re
import shutil
import statistics
import struct
import subprocess
import sys
import xml.etree.ElementTree as ElementTree
from array import array

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import hosted  # noqa: E402
import parity  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parents[2]
RATE = 48000
SIZES = [32, 64, 128, 256, 512, 1024]
WARM_UP_SECONDS = 0.25
# Every run is at least this long and this many blocks, so a 99th percentile
# is not a run's maximum at large blocks.
MIN_SECONDS = 2.0
MIN_BLOCKS = 1000
# The parameter the sweep moves: the mid band's frequency, the commonest kind
# of automation.
SWEPT = "mid.freq"
SCENARIOS = {
    "quiet": "No parameter moves",
    "sweep": f"The mid band's frequency (`{SWEPT}`) moves every block",
    "busy": "Every continuous parameter (18) moves every block",
}
# How the audio thread waits between blocks (hosted.cpp, playPaced).
PACES = {
    "spin": "the audio thread spins until each block is due, so its core stays awake and a block's time is "
            "the code's alone",
    "idle": "the audio thread sleeps until each block is due, so its core may drop into a sleep state and "
            "wake with cold caches, as in a light session; how deep it sleeps varies with the machine's state, which has "
            "moved these times by up to three times at small blocks",
}

BUILDS = {
    "A": {"what": "the EQ written in JUCE, no Seam, with Seam's ranges ported so it stores what the Rust EQ stores",
          "ids": "juce", "gated": True,
          "bundle": ROOT / "bench/build/BenchJuceEq_artefacts/Release/VST3/Bench JUCE EQ.vst3"},
    "A-native": {"what": "A with JUCE's own ranges, as a JUCE developer would declare them; timing only",
                 "ids": "juce", "gated": False,
                 "bundle": ROOT / "bench/build/BenchJuceEqNative_artefacts/Release/VST3/Bench JUCE EQ native.vst3"},
    "B": {"what": "the Rust EQ through Seam's VST3 exporter", "ids": "seam", "gated": True, "bundle": None},
    "C": {"what": "the Rust EQ through Seam's C interface and the JUCE adapter", "ids": "juce", "gated": True,
          "bundle": ROOT / "bench/build/BenchSeamJuceEq_artefacts/Release/VST3/Seam Example EQ JUCE.vst3"},
    "A-port": {"what": "A-native with the Rust EQ's filters ported line for line to C++ (`juce-eq/EqPort.h`) in "
                       "place of JUCE's `dsp::IIR`; timing only",
               "ids": "juce", "gated": False,
               "bundle": ROOT / "bench/build/BenchJuceEqPort_artefacts/Release/VST3/Bench JUCE EQ port.vst3"},
    "A-port-clang": {"what": "A-port with its C++ built by clang-cl (LLVM, as Rust's compiler) in place of MSVC; "
                             "timing only",
                     "ids": "juce", "gated": False,
                     "bundle": ROOT / "bench/build-clang/BenchJuceEqPort_artefacts/Release/VST3/"
                                      "Bench JUCE EQ port clang.vst3"},
    "C-clang": {"what": "C with its C++ (JUCE's wrapper and the adapter) built by clang-cl; the same Rust",
                "ids": "juce", "gated": True,
                "bundle": ROOT / "bench/build-clang/BenchSeamJuceEq_artefacts/Release/VST3/Seam Example EQ JUCE.vst3"},
}
GATED = [build for build, spec in BUILDS.items() if spec["gated"]]
# Pairs whose outputs must match bit for bit in every run: A, B, C and C-clang
# store Seam's values; A-native and the A-port builds store JUCE's, and the
# port's arithmetic is JUCE's filters'.
MATCHED = [("A", "B"), ("A", "C"), ("B", "C"), ("A", "C-clang"), ("A-native", "A-port"),
           ("A-native", "A-port-clang")]
# Reported, not matched: how far JUCE's ranges move the output.
APART = [("A", "A-native")]
APART_NAMES = {f"{first}-{second}" for first, second in APART}
# The ladder (README, "The ladder"): each step changes one thing, as (name,
# build, the build it is set against, what changes).
STEPS = [
    ("port", "A-port", "A-native", "the Rust EQ's filters ported to C++ against JUCE's `dsp::IIR`, both MSVC"),
    ("compiler", "A-port-clang", "A-port", "clang-cl against MSVC, on the same C++"),
    ("framework", "C-clang", "A-port-clang",
     "Seam's parameters, the adapter, the C interface and the filters in Rust against JUCE's "
     "`AudioProcessorValueTreeState` and the same filters in C++, all LLVM, inside the same clang-built wrapper"),
    ("wrapper compiler", "C", "C-clang", "C's wrapper and adapter built by MSVC against clang-cl"),
    ("exporter", "B", "C", "Seam's VST3 exporter against JUCE's wrapper and the adapter"),
]
LADDER = ["A-native", "A-port", "A-port-clang", "C-clang", "C", "B"]


def run(command, **options):
    return subprocess.run(command, capture_output=True, text=True, **options)


def module_of(bundle):
    """The binary inside a Windows .vst3 bundle."""
    found = sorted((bundle / "Contents" / "x86_64-win").glob("*.vst3"))
    if len(found) != 1:
        raise SystemExit(f"expected one module in {bundle}, found {len(found)}")
    return found[0]


def bundle_b(out):
    """Builds the EQ crate's cdylib and puts it in a bundle, as B."""
    built = run(["cargo", "build", "-p", "seam-example-eq", "--release", "--locked"], cwd=ROOT)
    if built.returncode != 0:
        raise SystemExit(built.stderr)
    bundle = out / "bundles" / "Seam Example EQ.vst3"
    folder = bundle / "Contents" / "x86_64-win"
    folder.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ROOT / "target/release/seam_example_eq.dll", folder / "Seam Example EQ.vst3")
    return bundle


def describe(tool, build, spec, out):
    """The build's description, naming its bundle. A scan names the bundle
    when it holds a moduleinfo.json and the module inside it when not (B's,
    until the bundle step writes one); JUCE's hosting searches a bundle on
    every create, so every build is described by its bundle alike, as a host
    that keeps bundle paths does."""
    described = run([str(tool), str(spec["bundle"]), "describe"])
    if described.returncode != 0:
        raise SystemExit(f"{build}: {described.stderr.strip()}")
    element = ElementTree.fromstring(described.stdout.strip())
    element.set("file", str(spec["bundle"]))
    path = out / f"{build}.xml"
    path.write_text(ElementTree.tostring(element, encoding="unicode"), encoding="utf-8")
    return path


def listed_ids(tool, bundle):
    info = run([str(tool), str(bundle), "info"])
    if info.returncode != 0:
        raise SystemExit(info.stderr.strip())
    return {line.split("|")[0] for line in info.stdout.splitlines()
            if "|" in line and line.split("|")[0].isdigit()}


# --- Automation ---------------------------------------------------------


def steps_table():
    """Each parameter's 4,097 even 0..1 steps, as (f32 bits, the value the
    EQ stores for it) from the parity example, which A, B and C store alike
    (parity.py's normalized check)."""
    table = {}
    for line in parity.rust("normalized"):
        key, normalized, value = line.split()
        steps = table.setdefault(key, [])
        if len(steps) < 4097:
            steps.append((normalized, value))
    return table


def walk(steps, default_bits, direction):
    """0..1 steps back and forth through `steps` from the one nearest the
    default, each storing a value other than the one before it, so no build
    is sent a write it can skip (README, console host rule 3)."""
    default = parity.as_float(default_bits)
    position = min(range(len(steps)), key=lambda i: abs(parity.as_float(steps[i][1]) - default))
    last = default_bits
    while True:
        if not 0 <= position + direction < len(steps):
            direction = -direction
        position += direction
        if steps[position][1] != last:
            last = steps[position][1]
            yield steps[position][0]


def automation(scenario, warm_up, blocks, table, ranges):
    """The host writes of one run, as (block, key, f32 bits): none in the
    warm-up, so every build has played with the transport running first."""
    if scenario == "quiet":
        return []
    keys = [SWEPT] if scenario == "sweep" else [key for key, (kind, _) in ranges.items() if kind == "continuous"]
    walkers = {key: walk(table[key], ranges[key][1], 1 if i % 2 == 0 else -1) for i, key in enumerate(keys)}
    return [(block, key, next(walkers[key])) for block in range(warm_up, warm_up + blocks) for key in keys]


def f32(value):
    return struct.unpack("<f", struct.pack("<f", value))[0]


def native_repeats(writes, specs):
    """How many writes would leave A-native's stored value where the last
    write to that parameter left it, so A-native could skip their work:
    JUCE's NormalisableRange (skew for a centre, then the interval's snap)
    worked in f32 as JUCE does, from the parity example's ranges."""
    last, same, total = {}, 0, 0
    for _, key, bits in writes:
        start, end, interval, centre = (parity.as_float(specs[key][i]) for i in (2, 3, 4, 5))
        skew = 1.0 if math.isnan(centre) else f32(math.log(0.5) / math.log((centre - start) / (end - start)))
        proportion = min(max(parity.as_float(bits), 0.0), 1.0)
        if skew != 1.0 and proportion > 0.0:
            proportion = f32(math.exp(f32(math.log(proportion)) / skew))
        value = f32(start + (end - start) * proportion)
        value = min(max(f32(start + interval * math.floor(f32((value - start) / interval) + 0.5)), start), end)
        if key in last:
            total += 1
            same += value == last[key]
        last[key] = value
    return same, total


# --- Measuring ----------------------------------------------------------


def measure(tool, spec, frames, warm_up, blocks, writes, audio_out, pace):
    lines = "".join(f"{block} {spec['id_of'](key)} {bits}\n" for block, key, bits in writes)
    done = subprocess.run([str(tool), str(spec["bundle"]), "measure", str(spec["description"]), str(RATE),
                           str(frames), str(warm_up), str(blocks), str(audio_out), pace],
                          input=lines, capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit(f"measure failed for {spec['bundle'].name} at {frames}: {done.stderr.strip()}")
    result = {}
    for pair in done.stdout.split():
        key, value = pair.split("=")
        result[key] = float(value) if "." in value else int(value)
    return result


def difference(first, second):
    """Mismatched samples, and the RMS and peak of the difference in dBFS."""
    one, other = first.read_bytes(), second.read_bytes()
    a, b = array("f"), array("f")
    a.frombytes(one)
    b.frombytes(other)
    if one == other:
        return {"samples": len(a), "mismatched": 0, "rms_db": float("-inf"), "peak_db": float("-inf")}
    if len(a) != len(b):
        return {"samples": max(len(a), len(b)), "mismatched": max(len(a), len(b)), "rms_db": 0.0, "peak_db": 0.0}
    mismatched, total, peak = 0, 0.0, 0.0
    for x, y in zip(a, b):
        if struct.pack("<f", x) != struct.pack("<f", y):
            mismatched += 1
            delta = x - y
            total += delta * delta
            peak = max(peak, abs(delta))
    db = lambda value: 20 * math.log10(value) if value > 0 else float("-inf")
    return {"samples": len(a), "mismatched": mismatched,
            "rms_db": db(math.sqrt(total / len(a))), "peak_db": db(peak)}


# --- Lines of code ------------------------------------------------------


def code_lines(path):
    """(lines, code lines): code is neither blank nor only a comment. A Rust
    file's #[cfg(test)] items are left out, and so are the branches of A's
    source that A-native and A-port switch on (`#if BENCH_...`, with its
    #else and #endif): A's code is what A builds."""
    rust = path.suffix == ".rs"
    toml = path.suffix == ".toml"
    lines, code, in_block, skip_depth, switch = 0, 0, False, None, None
    for line in path.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("#if BENCH_"):
            switch = "branch"
            continue
        if switch == "branch":
            switch = "else" if stripped == "#else" else None if stripped == "#endif" else switch
            continue
        if switch == "else" and stripped == "#endif":
            switch = None
            continue
        if rust and skip_depth is None and stripped == "#[cfg(test)]":
            skip_depth = 0
            continue
        if skip_depth is not None:
            skip_depth += line.count("{") - line.count("}")
            if skip_depth <= 0 and "}" in line:
                skip_depth = None
            continue
        lines += 1
        if toml:
            code += bool(stripped) and not stripped.startswith("#")
            continue
        if in_block:
            if "*/" in stripped:
                in_block = False
            continue
        if not stripped or stripped.startswith("//"):
            continue
        if stripped.startswith("/*"):
            in_block = "*/" not in stripped
            continue
        code += 1
    return lines, code


def cmake_lines(path, names):
    """Code lines of the CMake statements in `path` whose first argument is
    one of `names`: a target's own build description."""
    count, depth, inside = 0, 0, False
    for line in path.read_text(encoding="utf-8").splitlines():
        if not inside and re.match(r"\s*\w+\((%s)\b" % "|".join(map(re.escape, names)), line):
            inside, depth = True, 0
        if inside:
            stripped = line.strip()
            count += bool(stripped) and not stripped.startswith("#")
            depth += line.count("(") - line.count(")")
            if depth <= 0:
                inside = False
    return count


def plugin_code():
    port = ROOT / "bench/juce-eq/EqPort.h"
    juce_eq = sorted(path for path in (ROOT / "bench/juce-eq").glob("*.[hc]*") if path != port)
    rust_eq = sorted((ROOT / "examples/eq/src").glob("*.rs"))
    cmake = ROOT / "bench/CMakeLists.txt"
    cargo = code_lines(ROOT / "examples/eq/Cargo.toml")[1]
    files = lambda paths: {str(p.relative_to(ROOT)).replace("\\", "/"): code_lines(p) for p in paths}
    return {
        "A": {"files": files(juce_eq), "language": "C++",
              "ranges_port": code_lines(ROOT / "bench/juce-eq/SeamRange.h")[1],
              "build_lines": cmake_lines(cmake, ["BenchJuceEq"])},
        "A-port": {"filters_port": code_lines(port)[1]},
        "B": {"files": files(rust_eq), "language": "Rust", "build_lines": cargo},
        "C": {"files": files(rust_eq), "language": "Rust",
              "build_lines": cargo + cmake_lines(cmake, ["BenchSeamJuceEq"]),
              "generated": "examples/eq/seam-juce.cmake"},
    }


# --- The machine and the builds -----------------------------------------


def machine():
    def powershell(command):
        done = run(["powershell", "-NoProfile", "-Command", command])
        return done.stdout.strip()

    def compiler_version(folder):
        found = next((ROOT / folder / "CMakeFiles").glob("*/CMakeCXXCompiler.cmake"), None)
        version = re.search(r'CMAKE_CXX_COMPILER_VERSION "([^"]+)"', found.read_text()) if found else None
        return version.group(1) if version else "unknown"

    return {
        "cpu": powershell("(Get-CimInstance Win32_Processor).Name"),
        "cores": powershell("(Get-CimInstance Win32_Processor).NumberOfCores"),
        "threads": powershell("(Get-CimInstance Win32_Processor).NumberOfLogicalProcessors"),
        "memory_gb": round(int(powershell("(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory") or 0) / 2**30),
        "os": platform.platform(),
        "power_plan": run(["powercfg", "/getactivescheme"]).stdout.strip(),
        "rustc": run(["rustc", "-V"], cwd=ROOT).stdout.strip(),
        "msvc": compiler_version("bench/build"),
        "clang_cl": compiler_version("bench/build-clang"),
        "juce": "9.0.0 (f8f8864)",
        "seam": run(["git", "rev-parse", "--short", "HEAD"], cwd=ROOT).stdout.strip(),
        "dirty": bool(run(["git", "status", "--porcelain"], cwd=ROOT).stdout.strip()),
        "date": datetime.date.today().isoformat(),
    }


# --- The report ---------------------------------------------------------


def median(values):
    return statistics.median(values) if values else float("nan")


def summarise(runs):
    """Per build, size and scenario, the median over rounds of each run's
    figures, and the spread of the per-run medians. Per-second figures are
    over the timed span's wall time."""
    table = {}
    for (build, frames, scenario, pace), results in runs.items():
        per_second = lambda field: median([r[field] / (r["wall_ns"] / 1e9) for r in results]) / 1e6
        table[(build, frames, scenario, pace)] = {
            "blocks": results[0]["blocks"],
            "median_us": median([r["median_ns"] for r in results]) / 1000,
            "median_us_low": min(r["median_ns"] for r in results) / 1000,
            "median_us_high": max(r["median_ns"] for r in results) / 1000,
            "p99_us": median([r["p99_ns"] for r in results]) / 1000,
            "max_us": max(r["max_ns"] for r in results) / 1000,
            "first_us": median([r["first_ns"] for r in results]) / 1000,
            "median_ticks": median([r["median_ticks"] for r in results]),
            "process_m": per_second("process_cycles"),
            "main_m": per_second("main_cycles"),
            "audio_outside_m": median([(r["audio_cycles"] - r["total_ticks"]) / (r["wall_ns"] / 1e9)
                                       for r in results]) / 1e6,
            "other_m": median([(r["process_cycles"] - r["main_cycles"] - r["audio_cycles"]) / (r["wall_ns"] / 1e9)
                               for r in results]) / 1e6,
            "overruns": sum(r["overruns"] for r in results),
            "budget_percent": median([r["median_ns"] for r in results]) / (frames / RATE * 1e9) * 100,
        }
    return table


def write_report(out, info, checks, runs, matches, native, sizes, code, arguments):
    summary = summarise(runs)
    by_build = lambda build: [r for (b, _, _, _), results in runs.items() if b == build for r in results]
    loads = {build: {
        "create_ms": median([r["create_ns"] for r in by_build(build)]) / 1e6,
        "create_ms_low": min(r["create_ns"] for r in by_build(build)) / 1e6,
        "create_ms_high": max(r["create_ns"] for r in by_build(build)) / 1e6,
        "prepare_ms": median([r["prepare_ns"] for r in by_build(build)]) / 1e6,
        "added_private_kb": median([r["peak_private"] - r["base_private"] for r in by_build(build)]) / 1024,
        "added_private_kb_high": max(r["peak_private"] - r["base_private"] for r in by_build(build)) / 1024,
        "added_working_set_kb": median([r["peak_working_set"] - r["base_working_set"] for r in by_build(build)]) / 1024,
        "peak_working_set_mb": max(r["peak_working_set"] for r in by_build(build)) / 2**20,
    } for build in BUILDS}
    tsc_ghz = median([r["tsc_hz"] for results in runs.values() for r in results]) / 1e9

    data = {
        "machine": info,
        "settings": {"rate": RATE, "sizes": SIZES, "min_seconds": MIN_SECONDS, "min_blocks": MIN_BLOCKS,
                     "rounds": arguments.rounds, "warm_up_seconds": WARM_UP_SECONDS, "scenarios": SCENARIOS,
                     "paces": PACES, "tsc_ghz": tsc_ghz},
        "builds": {build: {"what": spec["what"], "gated": spec["gated"], "module": str(spec["module"]),
                           "bytes": sizes[build]} for build, spec in BUILDS.items()},
        "hosted_checks": {build: {"ok": ok, "lines": lines} for build, (ok, lines) in checks.items()},
        "output_match": {" ".join(map(str, key)): result for key, result in matches.items()},
        "native_repeats": native,
        "runs": {" ".join(map(str, key)): results for key, results in runs.items()},
        "summary": {" ".join(map(str, key)): figures for key, figures in summary.items()},
        "load": loads,
        "code": code,
    }
    (out / "report.json").write_text(json.dumps(data, indent=1), encoding="utf-8")

    md = []
    w = md.append
    w("# The bare EQ, headless\n")
    w(f"{info['date']}, Seam `{info['seam']}`{' with local changes' if info['dirty'] else ''}, "
      f"on {info['cpu']} ({info['cores']} cores, {info['threads']} threads, {info['memory_gb']} GB), "
      f"{info['os']}.\n")
    w("| Build | What it is | Module |")
    w("| --- | --- | --- |")
    for build, spec in BUILDS.items():
        w(f"| {build} | {spec['what']} | {sizes[build] / 1024:,.0f} KiB |")
    w("")

    w("## Output\n")
    w(f"Each of {', '.join(GATED)} plays the parity script through JUCE's VST3 hosting, at 44.1, 48 and 96 kHz, "
      "against the Rust EQ driven directly (`scripts/hosted.py`):\n")
    w("| Build | Result |")
    w("| --- | --- |")
    for build, (ok, lines) in checks.items():
        cases = [line for line in lines if "cases" in line]
        total = sum(int(re.search(r"(\d+) cases", line).group(1)) for line in cases)
        bad = sum(int(re.search(r"(\d+) mismatches", line).group(1)) for line in cases)
        w(f"| {build} | {'bit for bit' if ok else 'DIFFERS'}: {total:,} frames, {bad} mismatched; state reloads |")
    w("")
    gated = {key: result for key, result in matches.items() if key[-1] not in APART_NAMES}
    apart = [result for key, result in matches.items() if key[-1] in APART_NAMES]
    differing = sorted({key[-1] for key, result in gated.items() if result["mismatched"]})
    worst = max((r for r in gated.values() if r["mismatched"]), key=lambda r: r["peak_db"], default=None)
    w(f"Every timed run's output, compared build against build in every round ({len(gated)} comparisons of "
      f"{', '.join(f'{a} with {b}' for a, b in MATCHED)}; {sum(r['samples'] for r in gated.values()):,} "
      "samples): "
      + ("**A, B, C and C-clang match bit for bit, and so do A-native, A-port and A-port-clang.**"
         if not differing else
         f"**{', '.join(differing)} differ**, worst {worst['rms_db']:.1f} dBFS RMS, {worst['peak_db']:.1f} dBFS "
         "peak.") + "\n")
    if apart:
        loudest_rms = max(r["rms_db"] for r in apart)
        loudest_peak = max(r["peak_db"] for r in apart)
        w(f"A-native against A: at most {loudest_rms:.1f} dBFS RMS and {loudest_peak:.1f} dBFS peak "
          f"apart, from the values JUCE's ranges store. Of the {native[1]:,} writes in the first round, "
          f"{native[0]} would leave such a value unchanged (JUCE's range worked in f32 from the same writes), so "
          "A-native and the A-port builds skip no work A does.\n")

    for pace, how in PACES.items():
        for scenario, meaning in SCENARIOS.items():
            w(f"## Time per block: {scenario}, {pace}\n")
            w(f"{meaning}, and {how}. Median over {arguments.rounds} rounds of each run's median time in "
              f"`process`, in µs at {RATE // 1000} kHz; in brackets the lowest and highest round. Ratios are of "
              "the medians; against A-native they set Seam against plain JUCE, against A they compare builds that "
              "store the same values.\n")
            w("| Frames | A | A-native | B | C | B/A-native | C/A-native | B/A | C/A |")
            w("| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |")
            for frames in SIZES:
                s = {b: summary[(b, frames, scenario, pace)] for b in BUILDS}
                cell = lambda f: f"{f['median_us']:.2f} ({f['median_us_low']:.2f}–{f['median_us_high']:.2f})"
                ratio = lambda x, y: f"{s[x]['median_us'] / s[y]['median_us']:.2f}"
                w(f"| {frames} | {cell(s['A'])} | {cell(s['A-native'])} | {cell(s['B'])} | {cell(s['C'])} "
                  f"| {ratio('B', 'A-native')} | {ratio('C', 'A-native')} | {ratio('B', 'A')} | {ratio('C', 'A')} |")
            w("")

    for pace, how in PACES.items():
        for scenario, meaning in SCENARIOS.items():
            w(f"## The ladder: {scenario}, {pace}\n")
            w(f"{meaning}, and {how}. Each step from plain JUCE to Seam's own build changes one thing, so each "
              "ratio has one cause:\n")
            for name, build, against, what in STEPS:
                w(f"- **{name}** ({build}/{against}): {what}.")
            w("\nMedian µs over the rounds, in brackets the lowest and highest round, then each step's ratio.\n")
            w("| Frames | " + " | ".join(LADDER) + " | " + " | ".join(name for name, *_ in STEPS) + " |")
            w("| ---: | " + " | ".join("---:" for _ in LADDER) + " | " + " | ".join("---:" for _ in STEPS) + " |")
            for frames in SIZES:
                s = {b: summary[(b, frames, scenario, pace)] for b in LADDER}
                cell = lambda f: f"{f['median_us']:.2f} ({f['median_us_low']:.2f}–{f['median_us_high']:.2f})"
                w(f"| {frames} | " + " | ".join(cell(s[b]) for b in LADDER) + " | "
                  + " | ".join(f"{s[build]['median_us'] / s[against]['median_us']:.2f}"
                               for _, build, against, _ in STEPS) + " |")
            w("")

    w("## Tails and budget\n")
    w("The 99th percentile of each run's block times, the median over rounds, in µs, with the blocks per run; "
      "and the share of a block's real-time period the median takes, in the same order.\n")
    w("| Frames | Scenario | Pace | Blocks | " + " | ".join(f"{b} p99" for b in BUILDS) + " | budget |")
    w("| ---: | --- | --- | ---: | " + " | ".join("---:" for _ in BUILDS) + " | --- |")
    for pace in PACES:
        for scenario in SCENARIOS:
            for frames in SIZES:
                s = {b: summary[(b, frames, scenario, pace)] for b in BUILDS}
                w(f"| {frames} | {scenario} | {pace} | {s['A']['blocks']:,} | "
                  + " | ".join(f"{s[b]['p99_us']:.2f}" for b in BUILDS) + " | "
                  + ", ".join(f"{s[b]['budget_percent']:.2f}%" for b in BUILDS) + " |")
    w("")

    # A spinning audio thread runs all the time, so the process's figure is
    # only worth giving from idle runs.
    cpu_pace = "idle" if "idle" in PACES else next(iter(PACES))
    w("## CPU while playing\n")
    w(f"Block times come from the processor's time stamp counter, which runs at a fixed {tsc_ghz:.2f} GHz here; "
      "a block's cycles in those units are its time times that rate. Below, the whole host process's cycles per "
      "second of audio while timed, in millions of the same units (`QueryProcessCycleTime`), and in brackets the "
      "main thread's and the other threads' (JUCE's timer threads and any a plugin starts). The rest is the audio "
      "thread: `process` and the host's own pacing. "
      + ("From the idle runs, since a spinning audio thread runs all the time.\n" if cpu_pace == "idle" else
         "This run has spin runs only, whose audio thread runs all the time, so the process's own figure counts "
         "that spinning.\n"))
    w("| Frames | Scenario | " + " | ".join(BUILDS) + " |")
    w("| ---: | --- | " + " | ".join("---" for _ in BUILDS) + " |")
    for scenario in SCENARIOS:
        for frames in SIZES:
            s = {b: summary[(b, frames, scenario, cpu_pace)] for b in BUILDS}
            cell = lambda f: f"{f['process_m']:.1f} ({f['main_m']:.1f} / {f['other_m']:.1f})"
            w(f"| {frames} | {scenario} | " + " | ".join(cell(s[b]) for b in BUILDS) + " |")
    w("")

    w("## First block, overruns\n")
    w("The first block after activation in µs, the median over every run at that size, and in brackets the "
      "blocks that took longer than their own real-time period, over every run.\n")
    w("| Frames | " + " | ".join(BUILDS) + " |")
    w("| ---: | " + " | ".join("---:" for _ in BUILDS) + " |")
    for frames in SIZES:
        first = {b: median([summary[(b, frames, s, p)]["first_us"] for s in SCENARIOS for p in PACES])
                 for b in BUILDS}
        over = {b: sum(summary[(b, frames, s, p)]["overruns"] for s in SCENARIOS for p in PACES) for b in BUILDS}
        w(f"| {frames} | " + " | ".join(f"{first[b]:.1f} ({over[b]})" for b in BUILDS) + " |")
    w("")

    w("## Load and memory\n")
    w("Every run is a fresh process that loads the build from a description naming its bundle, with no scan, as "
      "a host opening a project does: cold for the process, though the module's file is in the system's file "
      "cache from earlier runs. Create is `createInstanceFromDescription` (the bundle's search, the module's "
      "load, its factory and the instance); prepare is `prepareToPlay`. Memory is the host process's peak during "
      "the run less what it held just before creating the plugin.\n")
    w("| Build | create ms (low–high) | prepare ms | private memory added, KiB (highest) | working set added, KiB | peak working set, MiB |")
    w("| --- | ---: | ---: | ---: | ---: | ---: |")
    for build, load in loads.items():
        w(f"| {build} | {load['create_ms']:.2f} ({load['create_ms_low']:.2f}–{load['create_ms_high']:.2f}) "
          f"| {load['prepare_ms']:.2f} | {load['added_private_kb']:,.0f} ({load['added_private_kb_high']:,.0f}) "
          f"| {load['added_working_set_kb']:,.0f} | {load['peak_working_set_mb']:.1f} |")
    w("")

    w("## Binary size and plugin code\n")
    w("The module is the one binary inside each bundle. Code lines are neither blank nor only a comment; "
      "the Rust EQ's tests are left out. Build lines, by the same rule, are the plugin's own build description: "
      "A's CMake target, the EQ's `Cargo.toml`, and for C also its `seam_add_juce_plugin` call. A-native is A's "
      "source built with one switch.\n")
    w("| Build | Module bytes | Source files | Lines | Code lines | Build lines |")
    w("| --- | ---: | ---: | ---: | ---: | ---: |")
    for build in ["A", "B", "C"]:
        files = code[build]["files"]
        w(f"| {build} | {sizes[build]:,} | {len(files)} ({code[build]['language']}) "
          f"| {sum(l for l, _ in files.values())} | {sum(c for _, c in files.values())} | {code[build]['build_lines']} |")
    w(f"| A-native | {sizes['A-native']:,} | as A | | | |")
    w(f"| A-port | {sizes['A-port']:,} | as A-native, and `EqPort.h` | | | |")
    w(f"| A-port-clang | {sizes['A-port-clang']:,} | as A-port | | | |")
    w(f"| C-clang | {sizes['C-clang']:,} | as C | | | |")
    w("")
    w(f"A's code includes `SeamRange.h`, {code['A']['ranges_port']} code lines that port Seam's ranges so A "
      "stores what the Rust EQ stores; A-native leaves them out. A-port adds `EqPort.h`, "
      f"{code['A-port']['filters_port']} code lines that port the Rust EQ's filters, and leaves out JUCE's "
      f"`dsp::IIR`. C's settings for JUCE (`{code['C']['generated']}`) are generated, not written.\n")

    w("## How it was measured\n")
    w(f"- Machine: {info['cpu']}; {info['power_plan']}.")
    w(f"- Toolchains: {info['rustc']}; MSVC {info['msvc']}; clang-cl {info['clang_cl']} with lld-link; "
      f"JUCE {info['juce']}.")
    w("- Builds: Release. Rust with fat LTO and one codegen unit; the C++ with JUCE's recommended config and LTO "
      "flags, `/fp:precise` and the dynamic C runtime: under MSVC `/Ox /GL` and `-LTCG`, under clang-cl `/Ox`, "
      "which it takes as `-O3`, Rust's level, and `-flto`. x86-64 with no CPU-specific flags on any side, so no "
      "compiler can fuse a multiply-add. Both C++ compilers keep buffer security checks (`/GS`) on, their default; "
      "Rust has no stack protector.")
    w(f"- The console host is `bench-hosted measure` (`bench/hosted/hosted.cpp`) on JUCE's VST3 hosting. One "
      f"fresh process per run; {arguments.rounds} rounds, the build order rotating each round; per run "
      f"{WARM_UP_SECONDS} s of untimed warm-up, then at least {MIN_SECONDS:g} s and {MIN_BLOCKS:,} blocks timed, "
      f"stereo noise in, at {RATE} Hz.")
    w("- The audio thread is MMCSS \"Pro Audio\" at time-critical priority, not pinned to a core, and starts each "
      "block when its frames would be due, spinning or sleeping on a high-resolution timer until then; the main "
      "thread runs the message loop. Only `processBlock` is timed, between fenced reads of the time stamp "
      "counter; it includes JUCE's hosting of the call, the same for every build.")
    w("- JUCE's hosting hands every build one buffer as both input and output. Seam's exporter processes it in "
      "place; JUCE's wrapper, in every other build, copies the input into its own buffer and the result back.")
    w("- The transport reports playing at 120 bpm, and every host write arrives in `process`, a value that "
      "moves what the parameter stores (bench/README.md, console host rules).")
    w("")
    (out / "report.md").write_text("\n".join(md), encoding="utf-8")


def main():
    global MIN_SECONDS, MIN_BLOCKS
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--tool", required=True, type=pathlib.Path, help="the built bench-hosted")
    parser.add_argument("--out", type=pathlib.Path, default=ROOT / "bench/build/compare")
    parser.add_argument("--rounds", type=int, default=3)
    parser.add_argument("--sizes", type=lambda text: [int(x) for x in text.split(",")], default=SIZES)
    parser.add_argument("--min-seconds", type=float, default=MIN_SECONDS, help="timed audio per run, at least")
    parser.add_argument("--min-blocks", type=int, default=MIN_BLOCKS, help="timed blocks per run, at least")
    parser.add_argument("--paces", type=lambda text: text.split(","), default=list(PACES),
                        help=f"which paces to run, of {', '.join(PACES)}")
    for build in BUILDS:
        parser.add_argument(f"--{build.lower()}", type=pathlib.Path, metavar="BUNDLE",
                            help=f"a .vst3 to load as {build} instead of the default"
                            + (" (B's is otherwise built with cargo)" if build == "B" else ""))
    arguments = parser.parse_args()
    MIN_SECONDS, MIN_BLOCKS = arguments.min_seconds, arguments.min_blocks
    out = arguments.out.resolve()
    tool = arguments.tool.resolve()
    SIZES[:] = arguments.sizes
    if not arguments.paces or any(pace not in PACES for pace in arguments.paces):
        raise SystemExit(f"--paces takes {', '.join(PACES)}")
    for pace in list(PACES):
        if pace not in arguments.paces:
            del PACES[pace]
    # Clears only a folder a previous comparison wrote.
    if out.exists() and any(out.iterdir()):
        if not (out / "report.json").exists() and not (out / "audio").is_dir():
            raise SystemExit(f"{out} is not empty and holds no earlier comparison; name another --out")
        shutil.rmtree(out)
    (out / "audio").mkdir(parents=True)

    for build in BUILDS:
        given = getattr(arguments, build.lower().replace("-", "_"))
        if given:
            BUILDS[build]["bundle"] = given
    if BUILDS["B"]["bundle"] is None:
        BUILDS["B"]["bundle"] = bundle_b(out)
    for build, spec in BUILDS.items():
        spec["bundle"] = spec["bundle"].resolve()
        spec["module"] = module_of(spec["bundle"])
        spec["id_of"] = hosted.seam_id if spec["ids"] == "seam" else hosted.juce_id
        spec["description"] = describe(tool, build, spec, out)

    checks = {}
    for build in GATED:
        checks[build] = hosted.check(tool, BUILDS[build]["bundle"], BUILDS[build]["ids"])
        print(f"{build}: {checks[build][1][-1]}", flush=True)
    if not all(ok for ok, _ in checks.values()):
        raise SystemExit("a build does not match the Rust EQ; nothing is timed")
    keys = [line.split()[0] for line in parity.rust("ranges")]
    for build, spec in BUILDS.items():
        listed = listed_ids(tool, spec["bundle"])
        missing = [key for key in keys if str(spec["id_of"](key)) not in listed]
        if missing:
            raise SystemExit(f"{build} does not list the ids of {', '.join(missing)}")

    table = steps_table()
    specs = {line.split()[0]: line.split() for line in parity.rust("ranges")}
    ranges = {key: (fields[1], fields[6]) for key, fields in specs.items()}
    runs, matches, native = {}, {}, [0, 0]
    order = list(BUILDS)
    total = arguments.rounds * len(PACES) * len(SIZES) * len(SCENARIOS) * len(BUILDS)
    done = 0
    for round_number in range(arguments.rounds):
        # Rotated by an even share each round, so each build's places spread
        # over the order.
        shift = round_number * len(order) // arguments.rounds % len(order)
        turn = order[shift:] + order[:shift]
        for pace in PACES:
            for frames in SIZES:
                warm_up = max(1, math.ceil(WARM_UP_SECONDS * RATE / frames))
                blocks = max(MIN_BLOCKS, round(MIN_SECONDS * RATE / frames))
                for scenario in SCENARIOS:
                    writes = automation(scenario, warm_up, blocks, table, ranges)
                    if round_number == 0 and pace == next(iter(PACES)):
                        same, count = native_repeats(writes, specs)
                        native[0] += same
                        native[1] += count
                    audio = {build: out / "audio" / f"{build}-{frames}-{scenario}-{pace}-{round_number + 1}.f32"
                             for build in BUILDS}
                    for build in turn:
                        result = measure(tool, BUILDS[build], frames, warm_up, blocks, writes, audio[build], pace)
                        runs.setdefault((build, frames, scenario, pace), []).append(result)
                        done += 1
                        print(f"[{done}/{total}] round {round_number + 1} {pace} {build} {frames} {scenario}: "
                              f"{result['median_ns'] / 1000:.2f} us", flush=True)
                    for first, second in MATCHED + APART:
                        matches[(round_number + 1, pace, frames, scenario, f"{first}-{second}")] = difference(
                            audio[first], audio[second])
                    if round_number > 0:
                        for path in audio.values():
                            path.unlink()

    sizes = {build: spec["module"].stat().st_size for build, spec in BUILDS.items()}
    write_report(out, machine(), checks, runs, matches, native, sizes, plugin_code(), arguments)
    bad = [key for key, result in matches.items() if result["mismatched"] and key[-1] not in APART_NAMES]
    print(f"report: {out / 'report.md'}")
    if bad:
        print(f"outputs differ in {len(bad)} comparisons: {bad[:5]}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
