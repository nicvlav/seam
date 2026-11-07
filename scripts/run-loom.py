#!/usr/bin/env python3
"""Runs the loom models and fails unless every model in the source ran.

A model is a test that calls `loom::model(` in its own body, and its name
starts with `loom_`, which the run filters on. The run builds the library
tests of the crates in `LANE`, so a model belongs in `src/` of one of them.
`cargo test` passes when its filter matches nothing, so a rename that drops
the prefix, a cfg that hides a model, a feature missing from the command, or
a model in a `tests/` file, an example or another crate would pass the lane
while it checked nothing. This finds the models by their call to
`loom::model(` in every Rust file of the repository, not by their name: it
fails if one is named without the prefix, if one sits where the run does not
look, if one is missing from the run, or if the run holds one the source does
not. A model is identified by its crate, its module path and its name, so two
models of one name in different crates or modules are two.

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

# The crates under `crates/` whose library tests the run builds.
LANE = ("params", "learn", "host")
COMMAND = [
    "cargo", "test", *(arg for name in LANE for arg in ("-p", f"seam-{name}")),
    "--release", "--locked", "--lib", "loom_",
]
# Directories that hold no source of the repository's own.
SKIPPED_DIRECTORIES = {"target", "node_modules", ".git"}


def crate_name(path: pathlib.Path, root: pathlib.Path = ROOT) -> str:
    """The library name of the crate that holds `path`, as test binaries spell it."""
    manifest = root / "crates" / path.relative_to(root / "crates").parts[0] / "Cargo.toml"
    return NAME.search(manifest.read_text(encoding="utf-8")).group(1).replace("-", "_")


def module_path(path: pathlib.Path, root: pathlib.Path = ROOT) -> tuple[str, ...]:
    """The module path of a source file, as its crate names it."""
    parts = list(path.relative_to(root / "crates").parts[2:])
    parts[-1] = parts[-1].removesuffix(".rs")
    return tuple(part for part in parts if part not in ("lib", "main", "mod"))


def rust_files(root: pathlib.Path) -> list[pathlib.Path]:
    """Every Rust file under `root`, build output and dependencies left out."""
    return sorted(
        path
        for path in root.rglob("*.rs")
        if not SKIPPED_DIRECTORIES.intersection(path.relative_to(root).parts)
    )


def in_lane(path: pathlib.Path, root: pathlib.Path) -> bool:
    """Whether `--lib` of the `LANE` crates builds `path`."""
    parts = path.relative_to(root).parts
    return len(parts) > 3 and parts[0] == "crates" and parts[1] in LANE and parts[2] == "src"


def models_in_source(
    root: pathlib.Path = ROOT,
) -> tuple[set[tuple[str, tuple[str, ...], str]], list[str]]:
    """The models the run covers, each as (crate, module path of its file,
    name): the last `fn` before each call to `loom::model(`. Also the calls
    the run cannot reach, one line each, since a model there never runs."""
    models = set()
    stray = []
    for path in rust_files(root):
        text = path.read_text(encoding="utf-8")
        for call in CALL.finditer(text):
            shown = path.relative_to(root).as_posix()
            if not in_lane(path, root):
                stray.append(f"{shown}: loom::model( where the run does not build: only src/ of {', '.join(LANE)}")
                continue
            functions = FUNCTION.findall(text, 0, call.start())
            if not functions:
                stray.append(f"{shown}: loom::model( outside any function")
                continue
            models.add((crate_name(path, root), module_path(path, root), functions[-1]))
    return models, stray


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
    expected, stray = models_in_source()
    for problem in stray:
        print(problem, file=sys.stderr)
    if stray:
        return 1
    if not expected:
        print("no loom model found in the lane's crates", file=sys.stderr)
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
