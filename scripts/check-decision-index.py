#!/usr/bin/env python3
"""Fails if docs/decision-index.md no longer matches the record or the code.

The index lists, under a heading per id, the files and items that implement
or pin each decision and rule of the decision record. It is kept true by
hand; this check fails when:

- a listed file does not exist;
- a listed item is not defined in its file: in a Rust file each name of a
  path such as `Type::method` must have a definition there, as `cargo xtask
  defs` parses it with syn (an item of any kind, a `macro_rules!` name, a
  name in an `impl` header, a named field, an enum variant), so a word in a
  comment, a call, a `use` list or a struct literal does not keep a renamed
  item listed; any
  other item (a name in another language, a test
  name with spaces, a TOML table, a CI job) must appear in its file as
  written; "module" names the file itself;
- an id the record defines or cites has no heading here, or a heading names
  an id the record does not have;
- a heading lists no site and does not say "no code site".

The record's ids are read the way scripts/check-comment-tags.py reads them,
at any width; a record that yields none fails, and so does an index that has
no heading at all. A Rust file that does not parse fails the check.
"""
import functools
import importlib.util
import re
import sys
from pathlib import Path

INDEX = Path("docs/decision-index.md")
HEADING = re.compile(r"^### (property \d+|[A-Z]\d+)\. ")
SITE = re.compile(r"^- (test )?`([^`]+)`: (.+)$")
NO_SITE = "- no code site"
PATH_ITEM = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(?:::[A-Za-z_~][A-Za-z0-9_]*)*")


def load_tags():
    """check-comment-tags.py as a module: it reads the record and tokenises Rust."""
    spec = importlib.util.spec_from_file_location(
        "check_comment_tags", Path(__file__).with_name("check-comment-tags.py")
    )
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


tags = load_tags()


def record_ids() -> set[str]:
    defined, cited_only, properties = tags.record_ids(Path(tags.RECORD).read_text(encoding="utf-8"))
    return defined | cited_only | {f"property {n}" for n in properties}


def items_of(listed: str) -> list[str | None]:
    """The items a site line names; None for the file's own doc."""
    found: list[str | None] = re.findall(r"`([^`]+)`", listed)
    if re.search(r"(?:^|, )module(?:,|$)", listed):
        found.append(None)
    return found


@functools.cache
def definitions(path: str) -> frozenset[str]:
    """The names a Rust file defines, as `cargo xtask defs` reads them: items of
    every kind, macros, the fields of structs and unions, enum variants and
    their fields, and every name in an `impl` header. A word in a comment or a
    string defines nothing."""
    facts = tags.xtask_facts("defs", [path])[path]
    return frozenset(name for kind, name in facts if kind == "def")


def found_in(path: str, body: str, item: str) -> bool:
    if PATH_ITEM.fullmatch(item):
        if path.endswith(".rs"):
            return all(name in definitions(path) for name in item.split("::"))
        return all(re.search(rf"\b{re.escape(name)}\b", body) for name in item.split("::"))
    return item in body


def main() -> int:
    problems = []
    headings: dict[str, int] = {}
    sites: dict[str, int] = {}
    current = None
    texts: dict[str, str] = {}
    for number, line in enumerate(INDEX.read_text(encoding="utf-8").splitlines(), 1):
        heading = HEADING.match(line)
        if heading:
            current = heading.group(1)
            if current in headings:
                problems.append(f"{INDEX}:{number}: {current} is listed twice")
            headings[current] = number
            sites[current] = 0
            continue
        if current is None:
            continue
        if line == NO_SITE:
            sites[current] += 1
            continue
        site = SITE.match(line)
        if not site:
            continue
        sites[current] += 1
        path = site.group(2)
        if not Path(path).is_file():
            problems.append(f"{INDEX}:{number}: {current}: {path} does not exist")
            continue
        if path not in texts:
            texts[path] = Path(path).read_text(encoding="utf-8")
        for item in items_of(site.group(3)):
            if item is not None and not found_in(path, texts[path], item):
                problems.append(f"{INDEX}:{number}: {current}: `{item}` is not defined in {path}")
    if not headings:
        problems.append(f"{INDEX}: no heading found; the script's reading of the index is out of date")
    ids = record_ids()
    for missing in sorted(ids - headings.keys()):
        problems.append(f"{INDEX}: {missing} is in the record but not here")
    for extra in sorted(headings.keys() - ids):
        problems.append(f"{INDEX}:{headings[extra]}: {extra} is not in the record")
    for empty in sorted(name for name, count in sites.items() if count == 0):
        problems.append(f"{INDEX}:{headings[empty]}: {empty} lists no site; say \"no code site\"")
    for problem in problems:
        print(problem)
    if problems:
        print(f"\n{len(problems)} problem(s): move, add or remove the index's lines to match.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
