#!/usr/bin/env python3
"""Fails if a comment or doc cites a decision id, the decision record or a stage.

A comment states its reason in words. The decision record
(docs/transfer-from-seam-juce.md) keeps the history, and its index
(docs/decision-index.md) points from each decision to the code, so the code
never points back.

What counts as an id is read from the record, never kept here:

- the ids it defines: each row of its decision tables, each rule heading
  (a range heading such as "M1 to M3" defines each id in it), and each
  numbered property in its properties section, cited as "property N";
- the ids it cites without defining, in parentheses. Those are found only
  in the same parenthesised form, because outside it they are ordinary
  words: in an audio codebase the letter C and a number is a note name.

A defined id is found however it is written: plain, with a space or a dash
before its number, in a block comment, or as "decision" and its number.
"stage" and a number is found too.

It also refuses the ids of audit findings (a two-letter prefix and a number
such as "RD-4", and a review's "R-M1"): a finding is closed by a change or a
written decision, and the code says why in words. The record, its index and
the dated reports are where those ids may stay. The script names no product
or company: a denylist of names would write them into the repository.

Scope: every file git tracks, except the record, its index and its history
(docs/decision-history.md, which cites ids by design), the dated bench reports
under bench/reports/ (records of runs, not to be reworded), the files `cargo
xtask gen` writes (listed in xtask/src/generate.rs, whose source is scanned
instead), this script, which spells out the patterns, and files with no
comments (JSON, lockfiles, the licence). Code files are scanned in their comments
and docs only: `//` and `/* */` comments in Rust, C, C++ and TypeScript,
`#` comments in Python, TOML, YAML and CMake, and Python's docstrings.
Markdown is scanned whole, except that a doc may link the record.
Shell scripts are scanned as `#` comments. In YAML a quote opens a string only
where a value starts. Rust is not tokenised here: `cargo xtask scan` lexes it
with proc-macro2 and returns each comment, string literal and `fn` name
(xtask/src/scan.rs, with its tests), so a string over several lines, a raw
string with any prefix and a char literal are read as the language reads
them. Rust string literals and `fn` names
are scanned too, for the ids the record defines in the form a comment would
cite them ("D6", "(P7)") or, in a name, as a token ("as_p3_lists"): an
assertion message or a test name is read like a comment. Numbers are read at
any width, so an id past 99 is found, and a record that yields no ids fails
instead of passing everything.
"""
import os
import re
import subprocess
import sys
from pathlib import Path

RECORD = "docs/transfer-from-seam-juce.md"
INDEX = "docs/decision-index.md"
HISTORY = "docs/decision-history.md"
GENERATOR = "xtask/src/generate.rs"
SKIPPED_PREFIXES = ("bench/reports/",)
SKIPPED_SUFFIXES = (".json", ".lock", ".proptest-regressions")
SKIPPED_NAMES = ("LICENSE", "THIRD-PARTY-NOTICES")

SLASH_COMMENTS = (".h", ".hpp", ".c", ".cpp", ".ts", ".tsx", ".js", ".mjs", ".cjs", ".css")
# Of those, the ones with single-quoted strings and template literals.
JS_LIKE = (".ts", ".tsx", ".js", ".mjs", ".cjs")
# A C++ raw string's opening, `R"delim(` with an optional encoding prefix.
RAW_STRING = re.compile(r'(?:u8|u|U|L)?R"([^()\\\s"]{0,16})\(')
HASH_COMMENTS = (".py", ".sh", ".toml", ".yml", ".yaml", ".cmake", ".txt", ".gitignore", ".gitattributes")
MARKDOWN = (".md",)

FINDING = re.compile(r"\b(?:(?:RD|RT|ST|HB|MD|TL|BN)-\d+|[RC]-[MSN]\d+)\b")
PROCESS = re.compile(r"\b(?:stage|wave) \d+\b", re.IGNORECASE)
RECORD_POINTER = re.compile(r"transfer-from-seam-juce|transfer document", re.IGNORECASE)
DECISION_WORD = re.compile(r"\bdecisions? \d+\b", re.IGNORECASE)
PROPERTY_WORD = re.compile(r"\bpropert(?:y|ies) (\d+)\b", re.IGNORECASE)
ID_LIKE = re.compile(r"\b([A-Z]{1,2})[ -]?(\d+)\b")
STRICT_ID = re.compile(r"\b([A-Z]\d+)\b")
ID_TOKEN = re.compile(r"^([a-z])(\d+)s?$")
# Types that read as an id when lower-cased.
NOT_IDS = {"u8", "u16", "u32", "u64", "i8", "i16", "i32", "i64", "f32", "f64"}
# What may precede a YAML quote that opens a string (or nothing, at a line's start).
YAML_VALUE_START = ("", ":", "-", "[", "{", ",", "?")
PARENTHESISED = re.compile(r"\(([^()]*)\)")


def record_ids(text: str) -> tuple[set[str], set[str], set[str]]:
    """Returns the ids the record defines, the ones it only cites, and its
    property numbers."""
    defined = set(re.findall(r"^\| ([A-Z]\d+) \|", text, re.MULTILINE))
    defined |= set(re.findall(r"^### ([A-Z]\d+)\. ", text, re.MULTILINE))
    for letter, first, last in re.findall(
        r"^### ([A-Z])(\d+) to [A-Z](\d+)\. ", text, re.MULTILINE
    ):
        defined |= {f"{letter}{n}" for n in range(int(first), int(last) + 1)}
    properties: set[str] = set()
    section = re.search(r"^## \d+\. Properties.*?(?=^## )", text, re.MULTILINE | re.DOTALL)
    if section:
        properties = set(re.findall(r"^(\d+)\. ", section.group(0), re.MULTILINE))
    cited = set()
    for inside in PARENTHESISED.findall(text):
        cited |= set(re.findall(r"\b([A-Z]\d+)\b", inside))
    if not defined:
        # A record whose tables changed format would otherwise let everything pass.
        sys.exit(f"{RECORD}: no decision ids found; the script's reading of the record is out of date")
    letters = {i[0] for i in defined}
    # An id the record cites but never defines, in a series of its own.
    cited_only = {i for i in cited - defined if i[0] not in letters}
    return defined, cited_only, properties


def generated_files(generator: str = GENERATOR) -> set[str]:
    """The paths of the files `cargo xtask gen` writes, read from the `path:`
    of each `Generated` entry in its generator. A generator that yields none
    fails, since the skip would then match nothing and scan generated text."""
    source = Path(generator).read_text(encoding="utf-8")
    paths = set(re.findall(r'path: "([^"]+)"', source))
    if not paths:
        sys.exit(f"{generator}: no generated file found; the script's reading of the generator is out of date")
    return paths


def comments(path: str, text: str) -> list[tuple[int, str]]:
    """Returns each line's comment or doc text, by line number."""
    if path.endswith(MARKDOWN):
        return [(n, line) for n, line in enumerate(text.splitlines(), 1)]
    if path.endswith(SLASH_COMMENTS):
        return slash_comments(text, "js" if path.endswith(JS_LIKE) else "c")
    if path.endswith(HASH_COMMENTS) or Path(path).name == "CMakeLists.txt":
        return hash_comments(
            text, python=path.endswith(".py"), yaml=path.endswith((".yml", ".yaml"))
        )
    return []


def slash_comments(text: str, language: str = "c") -> list[tuple[int, str]]:
    """Each line's `//` and `/* */` comment text, by line number.

    A string hides what looks like a comment inside it. Double-quoted strings
    are read in every language. With `language` "js" (TypeScript and
    JavaScript), so are single-quoted strings and template literals, across
    lines and through each `${}` expression. With "c" (C and C++), so are raw
    strings (`R"delim(...)delim"`, across lines), and a char literal such as
    `'"'` opens no string. A JavaScript regular expression literal is not
    recognised: none in the tracked files holds a quote or a `//`.
    """
    parts: dict[int, list[str]] = {}
    line, i, end = 1, 0, len(text)
    # What is open, innermost last: a template literal's text, or a `${}`
    # expression inside one with the braces it has opened.
    nesting: list[list] = []

    def skip(to: int) -> None:
        nonlocal line, i
        line += text.count("\n", i, to)
        i = to

    while i < end:
        ch = text[i]
        if nesting and nesting[-1][0] == "template":
            if ch == "\\":
                skip(min(i + 2, end))
            elif ch == "`":
                nesting.pop()
                i += 1
            elif text.startswith("${", i):
                nesting.append(["expression", 0])
                i += 2
            else:
                skip(i + 1)
            continue
        if text.startswith("//", i):
            stop = text.find("\n", i)
            stop = end if stop < 0 else stop
            parts.setdefault(line, []).append(text[i:stop])
            i = stop
        elif text.startswith("/*", i):
            stop = text.find("*/", i + 2)
            stop = end if stop < 0 else stop + 2
            for offset, piece in enumerate(text[i:stop].split("\n")):
                parts.setdefault(line + offset, []).append(piece)
            skip(stop)
        elif ch == '"' or (language == "js" and ch == "'"):
            skip(string_end(text, i))
        elif language == "js" and ch == "`":
            nesting.append(["template"])
            i += 1
        elif language == "c" and ch == "'":
            skip(char_literal_end(text, i))
        elif language == "c" and (raw := raw_string_at(text, i)) is not None:
            skip(raw)
        elif nesting and ch == "{":
            nesting[-1][1] += 1
            i += 1
        elif nesting and ch == "}":
            if nesting[-1][1] == 0:
                nesting.pop()
            else:
                nesting[-1][1] -= 1
            i += 1
        else:
            skip(i + 1)
    return [(number, " ".join(pieces)) for number, pieces in sorted(parts.items())]


def string_end(text: str, start: int) -> int:
    """Where the string opened by the quote at `start` ends: after its closing
    quote, or at the end of its line when it has none. A backslash escapes the
    next character, a line break included."""
    quote, i = text[start], start + 1
    while i < len(text):
        if text[i] == "\\":
            i += 2
        elif text[i] == quote:
            return i + 1
        elif text[i] == "\n":
            return i
        else:
            i += 1
    return len(text)


def char_literal_end(text: str, start: int) -> int:
    """Where the C char literal opened at `start` ends (`'a'`, `'"'`,
    `'\\''`), or `start + 1` for a quote that opens none, a C++ digit
    separator say."""
    if text[start + 1 : start + 2] == "\\":
        stop = text.find("'", start + 3, start + 12)
        return stop + 1 if stop >= 0 else start + 1
    if text[start + 2 : start + 3] == "'" and text[start + 1 : start + 2] != "\n":
        return start + 3
    return start + 1


def raw_string_at(text: str, start: int) -> int | None:
    """Where the C++ raw string that opens at `start` ends, or None when none
    opens there. The prefix must start a token: `XR"(` is no raw string."""
    if start > 0 and (text[start - 1].isalnum() or text[start - 1] == "_"):
        return None
    opening = RAW_STRING.match(text, start)
    if opening is None:
        return None
    closing = ")" + opening.group(1) + '"'
    stop = text.find(closing, opening.end())
    return len(text) if stop < 0 else stop + len(closing)


def xtask_command(task: str) -> list[str]:
    """The command that runs an xtask task: the binary `SEAM_XTASK` names, or
    cargo building it without the generators' dependencies."""
    binary = os.environ.get("SEAM_XTASK")
    if binary:
        return [binary, task]
    return ["cargo", "run", "--locked", "-q", "-p", "xtask", "--no-default-features", "--", task]


def unescape(text: str) -> str:
    """Undoes the escapes of the xtask output: backslash, newline, return, tab."""
    table = {"n": "\n", "r": "\r", "t": "\t"}
    return re.sub(r"\\(.)", lambda m: table.get(m.group(1), m.group(1)), text)


def xtask_facts(task: str, paths: list[str]) -> dict[str, list[list[str]]]:
    """Runs `cargo xtask <task>` over Rust files and returns its fact lines
    (split at tabs, text unescaped) by file. Rust is read by the xtask crate,
    which lexes it with the language's own lexer; this script never tokenises
    Rust. A file it cannot read fails the check."""
    if not paths:
        return {}
    done = subprocess.run(
        xtask_command(task), input="\n".join(paths), capture_output=True, text=True, encoding="utf-8"
    )
    if done.returncode != 0:
        sys.exit(f"cargo xtask {task} failed:\n{done.stderr}")
    facts: dict[str, list[list[str]]] = {}
    current = None
    for line in done.stdout.split("\n"):
        if not line:
            continue
        fields = line.split("\t")
        if fields[0] == "file":
            current = facts.setdefault(fields[1], [])
        else:
            current.append([fields[0]] + [unescape(f) for f in fields[1:]])
    missing = [path for path in paths if path not in facts]
    if missing:
        sys.exit(f"cargo xtask {task} read nothing of {missing[0]}")
    return facts


def rust_comments(facts: list[list[str]]) -> list[tuple[int, str]]:
    """Each line's comment text in a Rust file's facts, by line number: its
    `//` and `/* */` comments, doc comments, and any line holding a `#[doc`
    attribute."""
    by_line: dict[int, list[str]] = {}
    for kind, line, *rest in facts:
        if kind == "comment":
            by_line.setdefault(int(line), []).append(rest[0])
    return [(number, " ".join(parts)) for number, parts in sorted(by_line.items())]


def rust_literals(facts: list[list[str]]) -> list[tuple[int, str, bool]]:
    """Returns each string literal's value and each `fn` name (marked True), by line number."""
    found = [
        (int(line), rest[0], kind == "name")
        for kind, line, *rest in facts
        if kind in ("literal", "name")
    ]
    return sorted(found, key=lambda entry: entry[0])


def literal_problems(literal: str, defined: set[str], is_name: bool) -> list[str]:
    """The record's ids in a string literal or a `fn` name."""
    if is_name:
        for token in literal.split("_"):
            match = ID_TOKEN.match(token)
            if match and token not in NOT_IDS and f"{match.group(1).upper()}{match.group(2)}" in defined:
                return [token]
        return []
    # A literal that is nothing but an id is test data, not a citation.
    return [i for i in STRICT_ID.findall(literal) if i in defined and literal.strip() != i]


def hash_comments(text: str, python: bool, yaml: bool = False) -> list[tuple[int, str]]:
    """Each line's `#` comment. In YAML a quote opens a string only where a
    value or an item starts, so an apostrophe inside a plain scalar ("the
    scripts' tests") hides nothing."""
    found = []
    in_docstring = False
    for number, line in enumerate(text.splitlines(), 1):
        if python and line.count('"""') % 2 == 1:
            in_docstring = not in_docstring
            found.append((number, line))
            continue
        if in_docstring or (python and '"""' in line):
            found.append((number, line))
            continue
        in_string = None
        for i, ch in enumerate(line):
            if yaml and in_string is None and ch in "'\"" and line[:i].rstrip()[-1:] not in YAML_VALUE_START:
                continue
            if ch in "'\"" and (i == 0 or line[i - 1] != "\\"):
                in_string = None if in_string == ch else (in_string or ch)
            elif ch == "#" and in_string is None:
                found.append((number, line[i:]))
                break
    return found


def problems_in(comment: str, markdown: bool, ids) -> list[str]:
    defined, cited_only, properties = ids
    hits = []
    for letters, digits in ID_LIKE.findall(comment):
        if f"{letters}{digits}" in defined:
            hits.append(f"{letters}{digits}")
    for inside in PARENTHESISED.findall(comment):
        hits += [i for i in re.findall(r"\b([A-Z]\d+)\b", inside) if i in cited_only]
    hits += [f"property {n}" for n in PROPERTY_WORD.findall(comment) if n in properties]
    hits += DECISION_WORD.findall(comment) + PROCESS.findall(comment) + FINDING.findall(comment)
    if not markdown:
        hits += RECORD_POINTER.findall(comment)
    return hits


def main() -> int:
    ids = record_ids(Path(RECORD).read_text(encoding="utf-8"))
    skipped = {RECORD, INDEX, HISTORY, "scripts/check-comment-tags.py"} | generated_files()
    listed = subprocess.run(["git", "ls-files", "-z"], capture_output=True, text=True)
    if listed.returncode != 0:
        sys.exit("the check reads the files git tracks, so it runs in a git checkout:\n" + listed.stderr)
    tracked = listed.stdout.split("\0")
    problems = []
    texts: dict[str, str] = {}
    for path in filter(None, tracked):
        if (
            path in skipped
            or path.startswith(SKIPPED_PREFIXES)
            or path.endswith(SKIPPED_SUFFIXES)
            or Path(path).name in SKIPPED_NAMES
            or not Path(path).is_file()
        ):
            continue
        try:
            texts[path] = Path(path).read_text(encoding="utf-8")
        except UnicodeDecodeError:
            continue
    rust = xtask_facts("scan", [path for path in texts if path.endswith(".rs")])
    if not rust:
        sys.exit("no tracked Rust file to scan; the script's file selection is out of date")
    for path, text in texts.items():
        markdown = path.endswith(MARKDOWN)
        if path.endswith(".rs"):
            found = rust_comments(rust[path])
        else:
            found = comments(path, text)
        for number, comment in found:
            hits = problems_in(comment, markdown, ids)
            if hits:
                problems.append(f"{path}:{number}: {', '.join(hits)}: {comment.strip()}")
        if path.endswith(".rs"):
            for number, literal, is_name in rust_literals(rust[path]):
                hits = literal_problems(literal, ids[0], is_name)
                if hits:
                    problems.append(f"{path}:{number}: {', '.join(hits)}: {literal.strip()[:100]}")
    for problem in problems:
        print(problem)
    if problems:
        print(f"\n{len(problems)} line(s) cite a decision id, an audit finding, the decision record or a stage.")
        print("State the reason in words; docs/decision-index.md points from each decision to the code.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
