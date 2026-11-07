#!/usr/bin/env python3
"""The core crates and the bridge build with no window, WebView or host
dependency.

For each crate below, lists what it links on every target platform
(`cargo tree --target all`, dev-dependencies left out), once with its default
features and once with all of them, and fails if any of it is a window,
WebView, UI toolkit or plugin-format crate. No feature may pull one in either
(ARCHITECTURE.md): a crate that needs one belongs in seam-host or in a
shell crate such as seam-webview.

It also checks the layering among Seam's own crates against TIERS: every
dependency, dev-dependencies included, points to a lower tier, so two crates
of one tier never depend on each other (undo and session in particular). A
crate missing from the table fails until it is given a tier, and a table entry
that is no workspace crate fails until it is removed. A crate under examples/
is a plugin and takes the plugin tier without an entry. The crates checked for
window, WebView and host dependencies are the ones of tiers 1 to 3.

The window, WebView and host dependencies are a denylist of names and
prefixes, not an allowlist, because an allowlist of every crate the core links,
transitively, would need an edit for each dependency update, and the failure
it guards against is a known class: a UI stack or a plugin format. A new
stack is a new line in FORBIDDEN.

Usage: python3 scripts/check-core-deps.py
"""

import json
import subprocess
import sys
from pathlib import Path

# A crate may depend only on crates of a lower tier. Learn is above session
# because it reads the session's saved block; the shells, which depend on
# everything, are on top; xtask drives the example.
TIERS = {
    "seam-testkit": 0,
    "seam-params": 1,
    "seam-ump": 1,
    "seam-undo": 2,
    "seam-session": 2,
    "seam-learn": 3,
    "seam-bridge": 3,
    "seam-host": 4,
    "xtask": 6,
}

# Every crate under examples/ is a plugin: it depends on the host and below,
# and nothing depends on it but xtask.
PLUGIN_TIER = 5

# The core and the projection that must build with no window, WebView or host:
# every crate of tiers 1 to 3, which is everything but the test tools (tier 0)
# and the crates that may link a host (tier 4 and above).
CORE = [name for name, tier in TIERS.items() if 1 <= tier <= 3]

# Exact crate names, and prefixes that cover a family (wry's per-platform
# backends, the gtk stack, the objc2 AppKit and WebKit bindings).
FORBIDDEN = {
    "seam-host",
    "wry",
    "tao",
    "winit",
    "baseview",
    "raw-window-handle",
    "cocoa",
    "objc2-app-kit",
    "objc2-web-kit",
    "objc2-ui-kit",
    "webview2-com",
    "webkit2gtk",
    "javascriptcore-rs",
    "soup3",
    "vst3",
    "vst3-sys",
    "vst",
    "clap-sys",
    "nih_plug",
    "softbuffer",
    "muda",
    "eframe",
    "druid",
    "fltk",
    "glfw",
    "xcb",
}
FORBIDDEN_PREFIXES = (
    "tauri", "gtk", "gdk", "webview2-com", "webkit2gtk", "clack-",
    "egui", "iced", "slint", "sdl2", "glfw-", "x11", "wayland-", "floem",
)


def dependencies(crate: str, all_features: bool) -> set[str]:
    command = [
        "cargo", "tree", "--locked", "--package", crate, "--target", "all",
        "--edges", "normal,build", "--prefix", "none", "--format", "{p}",
    ]
    if all_features:
        command.append("--all-features")
    out = subprocess.run(command, check=True, capture_output=True, text=True).stdout
    names = set()
    for line in out.splitlines():
        line = line.strip()
        if line:
            names.add(line.split()[0])
    names.discard(crate)
    return names


def forbidden(name: str) -> bool:
    return name in FORBIDDEN or name.startswith(FORBIDDEN_PREFIXES)


def tier_of(package: dict, root: Path) -> int | None:
    """The tier of a workspace package: its table entry, or the plugin tier
    for a crate under examples/."""
    if package["name"] in TIERS:
        return TIERS[package["name"]]
    manifest = Path(package["manifest_path"])
    if manifest.is_relative_to(root / "examples"):
        return PLUGIN_TIER
    return None


def layering_problems(packages: list[dict], root: Path) -> list[str]:
    """Every way a workspace crate depends on one that is not below it."""
    names = {package["name"] for package in packages}
    tiers = {package["name"]: tier_of(package, root) for package in packages}
    problems = [f"{name} is in TIERS but is not a workspace crate" for name in TIERS if name not in names]
    for package in packages:
        name = package["name"]
        if tiers[name] is None:
            problems.append(f"{name} has no tier in TIERS")
            continue
        for dependency in package["dependencies"]:
            target = dependency["name"]
            if target not in names:
                continue
            if tiers[target] is None:
                problems.append(f"{name} depends on {target}, which has no tier in TIERS")
            elif tiers[target] >= tiers[name]:
                kind = dependency["kind"] or "normal"
                problems.append(
                    f"{name} (tier {tiers[name]}) has a {kind} dependency on "
                    f"{target} (tier {tiers[target]}): dependencies point down a tier"
                )
    return problems


def layering() -> list[str]:
    out = subprocess.run(
        ["cargo", "metadata", "--locked", "--no-deps", "--format-version", "1"],
        check=True, capture_output=True, text=True,
    ).stdout
    metadata = json.loads(out)
    return layering_problems(metadata["packages"], Path(metadata["workspace_root"]))


def main() -> int:
    failed = False
    problems = layering()
    for problem in problems:
        failed = True
        print(problem, file=sys.stderr)
    if not problems:
        print("every dependency points down a tier")
    for crate in CORE:
        for all_features, label in ((False, "default features"), (True, "all features")):
            bad = sorted(name for name in dependencies(crate, all_features) if forbidden(name))
            if bad:
                failed = True
                print(f"{crate} ({label}) depends on {', '.join(bad)}", file=sys.stderr)
            else:
                print(f"{crate} ({label}): no window, WebView or host dependency")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
