#!/usr/bin/env python3
"""Runs the loom models and fails unless every model in the source ran.

A model is a test under `crates/*/src` that calls `loom::model(` in its own
body, and its name starts with `loom_`, which the run filters on. `cargo
test` passes when its filter matches nothing, so a rename that drops the
prefix, a cfg that hides a model, or a feature missing from the command
would pass the lane while it checked nothing. This finds the models by
their call to `loom::model(`, not by their name: it fails if one is named
without the prefix, if one is missing from the run, or if the run holds one
the source does not. A model is identified by its crate, its module path and
its name, so two models of one name in different crates or modules are two.

Usage: python3 scripts/run-loom.py
"""

import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
CALL = re.compile(r"\bloom::model\s*\(")
FUNCTION = re.compile(r"\bfn\s+(\w+)\s*[<(]")
RESULT = re.compile(r"^test (?:(\S+)::)?(loom_\w+) \.\.\. (\w+)")
RUNNING = re.compile(r"Running unittests .*[\\/]([A-Za-z0-9_]+)-[0-9a-f]+(?:\.exe)?\)")
ANSI = re.compile(r"\x1b\[[0-9;]*m")
NAME = re.compile(r'^name\s*=\s*"([^"]+)"', re.MULTILINE)

COMMAND = [
    "cargo", "test", "-p", "seam-params", "-p", "seam-learn", "-p", "seam-host",
    "--release", "--locked", "--lib", "loom_",
]


def crate_name(path: pathlib.Path) -> str:
    """The library name of the crate that holds `path`, as test binaries spell it."""
    manifest = ROOT / "crates" / path.relative_to(ROOT / "crates").parts[0] / "Cargo.toml"
    return NAME.search(manifest.read_text(encoding="utf-8")).group(1).replace("-", "_")


def module_path(path: pathlib.Path) -> tuple[str, ...]:
    """The module path of a source file, as its crate names it."""
    parts = list(path.relative_to(ROOT / "crates").parts[2:])
    parts[-1] = parts[-1].removesuffix(".rs")
    return tuple(part for part in parts if part not in ("lib", "main", "mod"))


def models_in_source() -> set[tuple[str, tuple[str, ...], str]]:
    """Each model as (crate, module path of its file, name): the last `fn`
    before each call to `loom::model(`."""
    models = set()
    for path in sorted((ROOT / "crates").glob("*/src/**/*.rs")):
        text = path.read_text(encoding="utf-8")
        for call in CALL.finditer(text):
            functions = FUNCTION.findall(text, 0, call.start())
            if not functions:
                print(f"{path.relative_to(ROOT)}: loom::model( outside any function", file=sys.stderr)
                models.add((crate_name(path), module_path(path), "<outside any function>"))
                continue
            models.add((crate_name(path), module_path(path), functions[-1]))
    return models


def passed_models(output: str) -> set[tuple[str, tuple[str, ...], str]]:
    """Each model that passed, as (crate, module path inside it, name), read
    from the run's output: a test line belongs to the binary named above it."""
    passed = set()
    crate = ""
    # CI sets CARGO_TERM_COLOR=always, which puts escape codes inside
    # "Running". The child keeps that colour so the log reads as it does
    # elsewhere; only the matching strips it.
    for line in ANSI.sub("", output).splitlines():
        running = RUNNING.search(line)
        if running:
            crate = running.group(1)
            continue
        result = RESULT.match(line)
        if result and result.group(3) == "ok":
            path = tuple(result.group(1).split("::")) if result.group(1) else ()
            passed.add((crate, path, result.group(2)))
    return passed


def matches(source: tuple[str, tuple[str, ...], str], ran: tuple[str, tuple[str, ...], str]) -> bool:
    """A model ran as a source model when the crate and name agree and the
    module path it ran under begins with its file's (inline modules follow)."""
    return source[0] == ran[0] and source[2] == ran[2] and ran[1][: len(source[1])] == source[1]


def main() -> int:
    expected = models_in_source()
    if not expected:
        print("no loom model found in crates/*/src", file=sys.stderr)
        return 1
    unprefixed = sorted(name for _, _, name in expected if not name.startswith("loom_"))
    if unprefixed:
        print(
            f"loom models the run's filter skips, named without loom_: {', '.join(unprefixed)}",
            file=sys.stderr,
        )
        return 1
    env = dict(os.environ)
    env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " --cfg loom").strip()
    run = subprocess.run(
        COMMAND, cwd=ROOT, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True
    )
    sys.stdout.write(run.stdout)
    if run.returncode != 0:
        return run.returncode
    passed = passed_models(run.stdout)
    missing = sorted(
        "::".join((crate, *path, name))
        for crate, path, name in expected
        if not any(matches((crate, path, name), ran) for ran in passed)
    )
    extra = sorted(
        "::".join((crate, *path, name))
        for crate, path, name in passed
        if not any(matches(source, (crate, path, name)) for source in expected)
    )
    if missing:
        print(f"loom models in the source that did not run and pass: {', '.join(missing)}", file=sys.stderr)
    if extra:
        print(f"loom models that ran but the source scan missed: {', '.join(extra)}", file=sys.stderr)
    if missing or extra:
        return 1
    print(f"{len(expected)} loom models ran and passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
