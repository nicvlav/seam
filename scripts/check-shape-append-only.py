#!/usr/bin/env python3
"""Fails if crates/bridge/protocol-shape.txt lost or changed a version line.

The file holds one `version <n> shape <hash>` line per protocol version, and a
line is never edited after it ships. `crates/bridge/tests/protocol_version.rs`
reads only the last line, so it cannot see an old line overwritten; this
compares the file with the one on the base revision of a pull request. Every
version line of the base must be at the same place in the new file, word for
word, and only lines may follow. Comment and blank lines are not compared. A
file with no version line fails, in either revision that has the file.

Usage: python3 scripts/check-shape-append-only.py <base revision>
"""
import subprocess
import sys

PATH = "crates/bridge/protocol-shape.txt"


def version_lines(text: str) -> list[str]:
    """The lines that record a version, whitespace-normalised."""
    return [
        " ".join(line.split())
        for line in text.splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]


def problems(base: str | None, head: str) -> list[str]:
    """Why `head` is not `base` plus appended lines; `base` is None when the
    file did not exist there."""
    new = version_lines(head)
    if not new:
        return [f"{PATH} has no version line"]
    if base is None:
        return []
    old = version_lines(base)
    if not old:
        return [f"{PATH} has no version line on the base revision"]
    found = []
    for position, line in enumerate(old):
        if position >= len(new):
            found.append(f"`{line}` was removed")
        elif new[position] != line:
            found.append(f"`{line}` was changed to `{new[position]}`")
    return found


def known(revision: str) -> bool:
    """Whether `revision` names a commit this clone has. A shallow clone may
    lack the base, and reading nothing there would pass every change."""
    found = subprocess.run(
        ["git", "rev-parse", "--verify", "--quiet", f"{revision}^{{commit}}"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    return found.returncode == 0


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2
    if not known(sys.argv[1]):
        print(f"{sys.argv[1]} is no commit this clone has; fetch the base first", file=sys.stderr)
        return 1
    shown = subprocess.run(
        ["git", "show", f"{sys.argv[1]}:{PATH}"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
    )
    base = shown.stdout if shown.returncode == 0 else None
    with open(PATH, encoding="utf-8") as file:
        found = problems(base, file.read())
    for problem in found:
        print(f"{PATH}: {problem}; a line is kept as it is and the next version appends one", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
