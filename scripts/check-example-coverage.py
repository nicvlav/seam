#!/usr/bin/env python3
"""Fail when a public type, trait, function or macro a user can reach has no example.

Builds nightly rustdoc's JSON for each library crate and walks it from the crate
root the way a user's `use` does: through public modules and through `pub use`
re-exports, under the name the crate exports. It counts the items a user names
(structs, enums, unions, traits, type aliases, free functions and macros) and
fails below 100% for any crate. Items marked `#[doc(hidden)]` are skipped, and
rustdoc's JSON leaves them out. A re-export of another crate's item is skipped
too: its JSON is that crate's, which counts the item where it is defined.

Methods are reported, never gated: for each crate it prints how many public
methods have an example and how many do not, and lists the ones without. A
method is a public function in an inherent `impl` of a reachable struct, enum
or union, or a function of a reachable trait (provided or required). Methods of
trait impls are the trait's own and count there. The Rust API Guidelines ask for
an example on every method whose summary does not tell the whole story, which a
script cannot judge, so the list is what a docs pass reads. A doc has an example when it holds a
fenced code block that is Rust and runs or compiles: a block marked `ignore`,
`text` or `compile_fail` does not count.

The crates are the workspace's library crates under crates/, found with `cargo
metadata`, each built with all its features. A
crate with no reachable item fails, as does a rustdoc JSON format this script
does not know: both would otherwise read as full coverage.

Run it from the repository root; it needs a nightly toolchain and writes into the
target directory `CARGO_TARGET_DIR` names (default `target`). The toolchain is
`NIGHTLY` (default `nightly`). Pass crate names to check only those.
"""

import json
import os
import re
import subprocess
import sys
from pathlib import Path

# The rustdoc JSON format this script reads; a different one is refused until
# the script is checked against it.
FORMAT_VERSION = 61


def library_crates():
    """Each library crate under crates/, with the features that expose every public item."""
    out = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        check=True, capture_output=True, text=True,
    ).stdout
    metadata = json.loads(out)
    root = Path(metadata["workspace_root"]) / "crates"
    crates = {}
    for package in metadata["packages"]:
        has_lib = any("lib" in target["kind"] for target in package["targets"])
        if has_lib and Path(package["manifest_path"]).is_relative_to(root):
            features = sorted(set(package["features"]) - {"default"})
            crates[package["name"]] = features
    return crates

# Fence attributes that leave a block compiled and run, or at least compiled.
RUNS = {"", "rust", "no_run", "should_panic", "edition2021", "edition2024"}


def rustdoc_json(crate, features):
    nightly = os.environ.get("NIGHTLY", "nightly")
    cmd = ["cargo", f"+{nightly}", "rustdoc", "-p", crate, "--lib", "--locked", "-q"]
    if features:
        cmd += ["--features", ",".join(features)]
    cmd += ["--", "-Z", "unstable-options", "--output-format", "json"]
    subprocess.run(cmd, check=True, stdout=subprocess.DEVNULL)
    target = Path(os.environ.get("CARGO_TARGET_DIR", "target"))
    report = target / "doc" / (crate.replace("-", "_") + ".json")
    data = json.loads(report.read_text(encoding="utf-8"))
    check_format(data, crate)
    return data


def check_format(data, crate):
    version = data.get("format_version")
    if version != FORMAT_VERSION:
        sys.exit(f"{crate}: rustdoc JSON format {version}, this script reads {FORMAT_VERSION}")


def has_example(item):
    """Whether the doc has a fenced block that rustdoc compiles as Rust."""
    inside = False
    for line in (item.get("docs") or "").splitlines():
        line = line.strip()
        if not line.startswith("```"):
            continue
        if inside:
            inside = False
            continue
        inside = True
        attributes = {a.strip() for a in line[3:].replace(",", " ").split()}
        if attributes <= RUNS:
            return True
    return False


def kind_of(item):
    inner = item["inner"]
    return next(iter(inner)) if isinstance(inner, dict) else inner


def is_public(item):
    return item.get("visibility") == "public"


def is_hidden(item):
    return any(is_doc_hidden(a) for a in item.get("attrs") or [])


def is_doc_hidden(attribute):
    """Whether an attribute is exactly `#[doc(hidden)]`.

    rustdoc's JSON writes an attribute it has no structure for as
    {"other": "<source>"}. A `#[doc(alias = "hidden")]` is not one.
    """
    text = attribute.get("other") if isinstance(attribute, dict) else attribute
    return isinstance(text, str) and re.fullmatch(r"#!?\[doc\(\s*hidden\s*\)\]", text.strip()) is not None


class Walk:
    """The public items reachable from a crate root, and whether each has an example."""

    def __init__(self, data):
        self.index = data["index"]
        self.root = str(data["root"])
        self.seen = set()
        self.items = []  # (qualified name, has an example)
        self.owners = []  # (qualified name, kind, item) of each type and trait

    def run(self):
        self.visit(self.root, "")
        return self.items

    def record(self, name, item):
        self.items.append((name, has_example(item)))
        if kind_of(item) in ("struct", "enum", "union", "trait"):
            self.owners.append((name, kind_of(item), item))

    def methods(self):
        """Each reachable public method as (qualified name, has an example)."""
        found = []
        for name, kind, item in self.owners:
            for method in method_ids(self.index, kind, item):
                found.append((f"{name}::{self.index[method]['name']}", has_example(self.index[method])))
        return found

    def visit(self, item_id, prefix):
        item_id = str(item_id)
        item = self.index.get(item_id)
        if item is None or item_id in self.seen:
            return
        self.seen.add(item_id)
        kind = kind_of(item)
        if is_hidden(item):
            return
        if kind == "module":
            if item_id != self.root and not is_public(item):
                return
            inner = prefix if item_id == self.root else f"{prefix}{item['name']}::"
            for child in item["inner"]["module"]["items"]:
                self.visit(child, inner)
        elif kind == "use":
            self.visit_use(item["inner"]["use"], item, prefix)
        elif is_public(item):
            self.handle(item, kind, f"{prefix}{item['name']}")

    def visit_use(self, use, item, prefix):
        if not is_public(item) or use.get("id") is None:
            return
        target = self.index.get(str(use["id"]))
        if target is None:
            # Another crate's item: that crate's walk counts it.
            return
        if use.get("is_glob"):
            if kind_of(target) == "module":
                for child in target["inner"]["module"]["items"]:
                    self.visit(child, prefix)
            return
        if str(use["id"]) in self.seen or is_hidden(target):
            return
        self.seen.add(str(use["id"]))
        self.handle(target, kind_of(target), f"{prefix}{use['name']}")

    def handle(self, item, kind, name):
        if kind in ("struct", "enum", "union", "type_alias"):
            self.record(name, item)
        elif kind in ("trait", "function", "macro", "proc_macro"):
            self.record(name, item)
        elif kind == "module":
            for child in item["inner"]["module"]["items"]:
                self.visit(child, name + "::")


def method_ids(index, kind, item):
    """The ids of an owner's public, unhidden methods.

    A struct, enum or union owns the functions of its inherent impls (an impl
    with no trait, hidden or not blanket-generated); a trait owns its functions.
    """
    inner = item["inner"][kind]
    if kind == "trait":
        candidates = [str(i) for i in inner.get("items", [])]
    else:
        candidates = []
        for impl_id in inner.get("impls", []):
            impl = index.get(str(impl_id))
            if impl is None or is_hidden(impl):
                continue
            body = impl["inner"]["impl"]
            if body.get("trait") is None and not body.get("is_synthetic"):
                candidates += [str(i) for i in body["items"]]
    found = []
    for candidate in candidates:
        method = index.get(candidate)
        if method is None or kind_of(method) != "function" or is_hidden(method):
            continue
        if kind == "trait" or is_public(method):
            found.append(candidate)
    return sorted(set(found), key=lambda i: index[i]["name"])


def method_report(crate, methods):
    """The lines that report a crate's method coverage; never a failure."""
    have = sum(1 for _, covered in methods if covered)
    lines = [f"{crate}: {have} of {len(methods)} public methods have an example"]
    lines += [f"  {name}" for name, covered in sorted(methods) if not covered]
    return lines


def missing_of(crate, items):
    """The names in `items` with no example; exits when there are no items."""
    if not items:
        sys.exit(f"{crate}: no reachable public item; the walk is out of date or the crate exports nothing")
    return sorted(name for name, covered in items if not covered)


def main():
    crates = library_crates()
    if not crates:
        sys.exit("no library crate found under crates/")
    wanted = sys.argv[1:] or list(crates)
    failed = False
    method_lines = []
    for crate in wanted:
        walk = Walk(rustdoc_json(crate, crates[crate]))
        items = walk.run()
        missing = missing_of(crate, items)
        have = len(items) - len(missing)
        method_lines.append((crate, walk.methods()))
        percent = 100 * have / len(items)
        print(f"{crate}: {have} of {len(items)} reachable public types, type aliases, traits, functions and macros have an example ({percent:.1f}%)")
        for name in missing:
            failed = True
            print(f"  {name}")
    print()
    print("Methods (report only):")
    for crate, methods in method_lines:
        print(chr(10).join(method_report(crate, methods)))
    if failed:
        print("Add a runnable example to each item listed, or mark plumbing #[doc(hidden)].")
        sys.exit(1)


if __name__ == "__main__":
    main()
