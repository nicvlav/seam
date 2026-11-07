#!/usr/bin/env python3
"""Tests of the check scripts: each must refuse the bad input it exists for.

Run from the repository root: python scripts/test-checks.py
"""
import contextlib
import importlib.util
import io
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPTS = Path(__file__).parent


def load(name: str):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


tags = load("check-comment-tags")
index = load("check-decision-index")
deps = load("check-core-deps")
coverage = load("check-example-coverage")
loom = load("run-loom")
shape = load("check-shape-append-only")

RECORD = """\
| Q99 | something |
| Q100 | something later |
### Q1. A rule
"""


class CommentTags(unittest.TestCase):
    def ids(self):
        return tags.record_ids(RECORD)

    def test_ids_of_any_width_are_read(self):
        defined, _, _ = self.ids()
        self.assertIn("Q100", defined)
        self.assertIn("Q99", defined)

    def test_an_id_past_99_in_a_comment_is_found(self):
        hits = tags.problems_in("// see Q100", False, self.ids())
        self.assertEqual(hits, ["Q100"])

    def test_a_record_without_ids_fails(self):
        with self.assertRaises(tags.CheckError):
            tags.record_ids("| id | text |\n| --- | --- |\n")

    def test_a_quote_char_literal_does_not_hide_a_comment(self):
        found = tags.slash_comments("let q = '\"'; // Q100 here\n")
        self.assertEqual(len(found), 1)
        self.assertIn("Q100", found[0][1])

    def test_a_string_with_an_escaped_backslash_ends(self):
        found = tags.slash_comments('let p = "\\\\"; // after\n')
        self.assertEqual(len(found), 1)

    def js(self, source: str):
        return tags.comments("page/src/sample.ts", source)

    def test_a_single_quoted_string_with_a_double_quote_hides_no_comment(self):
        self.assertEqual(self.js("const q = 'a\"b'; // Q100 here\n"), [(1, "// Q100 here")])
        self.assertEqual(self.js("const q = 'it\\'s \"'; // Q100\n"), [(1, "// Q100")])

    def test_a_template_literal_hides_its_text_and_no_comment_after_it(self):
        self.assertEqual(self.js("const t = `a\"b`; // Q100\n"), [(1, "// Q100")])
        self.assertEqual(self.js("const u = `http://x`; // Q100\n"), [(1, "// Q100")])

    def test_a_template_literal_over_two_lines_ends_where_it_closes(self):
        source = "const t = `a\n// not a comment \"\n`; // Q100\n"
        self.assertEqual(self.js(source), [(3, "// Q100")])

    def test_a_template_expression_is_code_whose_braces_and_strings_count(self):
        self.assertEqual(self.js("const t = `${'\"'}`; // Q100\n"), [(1, "// Q100")])
        self.assertEqual(self.js("const t = `${ {a: '`'}.a } \"`; // Q100\n"), [(1, "// Q100")])
        # The object's brace closes the object, not the expression.
        self.assertEqual(self.js("const t = `${ {a: 1}['`'] }` + x; // Q100\n"), [(1, "// Q100")])
        self.assertEqual(self.js("const t = `${f(/* Q99 */ 1)}`;\n"), [(1, "/* Q99 */")])

    def test_a_cpp_raw_string_hides_quotes_and_slashes_across_lines(self):
        cpp = lambda source: tags.comments("adapters/juce/sample.cpp", source)
        self.assertEqual(cpp('auto s = R"x(a "b)x"; // Q100\n'), [(1, "// Q100")])
        self.assertEqual(cpp('auto s = u8R"(\n// not a comment\n)"; // Q100\n'), [(3, "// Q100")])
        self.assertEqual(cpp('auto n = 1\'000; // Q100\n'), [(1, "// Q100")], "a digit separator")
        self.assertEqual(cpp('auto q = \'\\\'\'; // Q100\n'), [(1, "// Q100")], "an escaped quote")

    def test_a_block_comment_over_lines_is_read_on_each(self):
        self.assertEqual(tags.slash_comments("a; /* one\ntwo */ b; // three\n"), [(1, "/* one"), (2, "two */ // three")])

    def rust(self, source: str):
        """Writes `source` to a file and returns the facts `cargo xtask scan` reads from it."""
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.rs"
            path.write_text(source, encoding="utf-8")
            return tags.xtask_facts("scan", [str(path)])[str(path)]

    def test_a_rust_comment_citing_an_id_is_refused(self):
        facts = self.rust("fn a() {}\n// see Q100\n/// and Q99\n")
        found = tags.rust_comments(facts)
        self.assertEqual([n for n, _ in found], [2, 3])
        self.assertEqual([tags.problems_in(c, False, self.ids()) for _, c in found], [["Q100"], ["Q99"]])

    def test_a_comment_after_a_string_over_two_lines_is_read(self):
        found = tags.rust_comments(self.rust('let s = "abc\ndef"; // Q100\n'))
        self.assertEqual(found, [(2, "// Q100")])

    def test_a_raw_string_and_a_char_literal_hide_no_comment(self):
        found = tags.rust_comments(self.rust('let a = br#"say "hi"#; let q = \'"\'; // D6\n'))
        self.assertEqual(found, [(1, "// D6")])

    def test_rust_string_literals_and_names_are_read(self):
        defined = self.ids()[0]
        source = 'fn each_origin_falls_back_as_q99_lists() {\n    assert!(x, "snapped (Q100)");\n    let k = "Q99";\n}\n'
        hits = [
            (line, tags.literal_problems(text, defined, is_name))
            for line, text, is_name in tags.rust_literals(self.rust(source))
        ]
        self.assertEqual(hits, [(1, ["q99"]), (2, ["Q100"]), (3, [])])

    def test_a_literal_in_a_comment_is_not_read(self):
        self.assertEqual(tags.rust_literals(self.rust('// "Q100"\nlet a = 1;\n')), [])

    def test_a_tab_and_a_backslash_in_a_literal_survive_the_round_trip(self):
        found = tags.rust_literals(self.rust(r'let a = "x\ty\\z";' + chr(10)))
        self.assertEqual([text for _, text, _ in found], ["x" + chr(9) + "y" + chr(92) + "z"])

    def test_rust_that_does_not_lex_fails(self):
        with self.assertRaises(tags.CheckError):
            self.rust('let s = "never closed;\n')

    def test_no_rust_file_to_scan_is_no_facts(self):
        self.assertEqual(tags.xtask_facts("scan", []), {})

    def test_a_scanned_file_that_is_not_utf8_fails_and_a_corpus_entry_is_not_scanned(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "notes.md"
            path.write_bytes(b"caf\xe9\n")
            with self.assertRaises(tags.CheckError):
                tags.read_scanned(str(path))
            path.write_text("fine\n", encoding="utf-8")
            self.assertEqual(tags.read_scanned(str(path)), "fine\n")
        self.assertTrue(tags.is_scanned("crates/x/src/lib.rs"))
        self.assertTrue(tags.is_scanned("adapters/juce/CMakeLists.txt"))
        self.assertFalse(tags.is_scanned("fuzz/corpus/ump_packets/mixed"))

    def test_a_check_that_cannot_run_fails_with_its_reason_not_an_exit_deep_inside(self):
        def cannot_run():
            raise tags.CheckError("cannot run")

        was, tags.check = tags.check, cannot_run
        try:
            with contextlib.redirect_stderr(io.StringIO()) as err:
                status = tags.main()
        finally:
            tags.check = was
        self.assertEqual(status, 1)
        self.assertIn("cannot run", err.getvalue())

    def test_the_decision_history_is_exempt_like_the_record_and_its_index(self):
        source = (SCRIPTS / "check-comment-tags.py").read_text(encoding="utf-8")
        self.assertIn('HISTORY = "docs/decision-history.md"', source)
        self.assertEqual(tags.HISTORY, "docs/decision-history.md")


    def test_the_generated_files_are_read_from_their_generator(self):
        generated = tags.generated_files()
        self.assertIn("page/src/generated/protocol.ts", generated)
        self.assertIn("page/test/generated/params.ts", generated)

    def test_a_generator_that_yields_no_generated_file_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "generate.rs"
            path.write_text("pub fn generated() -> Vec<Generated> {" + chr(10) + "    Vec::new()" + chr(10) + "}" + chr(10), encoding="utf-8")
            with self.assertRaises(tags.CheckError):
                tags.generated_files(str(path))

    def test_shell_comments_are_read(self):
        found = tags.comments("scripts/x.sh", "echo hi # Q100\n")
        self.assertEqual(len(found), 1)

    def test_an_apostrophe_in_a_yaml_plain_scalar_hides_no_comment(self):
        found = tags.comments("ci.yml", "- name: the scripts' tests # Q100\n")
        self.assertEqual(len(found), 1)
        self.assertIn("Q100", found[0][1])

    def test_a_quoted_yaml_value_still_hides_a_hash(self):
        self.assertEqual(tags.comments("ci.yml", "- name: 'a # b'\n"), [])
        self.assertEqual(tags.comments("ci.yml", 'key: "a # b" # real\n')[0][1], "# real")

class DecisionIndex(unittest.TestCase):
    def defined(self, source: str, name: str) -> bool:
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "sample.rs"
            path.write_text(source, encoding="utf-8")
            index.definitions.cache_clear()
            return index.found_in(str(path), source, name)

    def test_a_use_list_does_not_define_a_field_or_variant(self):
        source = "use seam::{\n    Commit, ParamId,\n};\nfn f() {}\n"
        self.assertFalse(self.defined(source, "ParamId"))
        self.assertTrue(self.defined(source, "f"))

    def test_a_path_needs_every_name_defined(self):
        source = "struct A;\nimpl A { fn go(&self) {} }\n"
        self.assertTrue(self.defined(source, "A::go"))
        self.assertFalse(self.defined(source, "A::stop"))
        self.assertFalse(self.defined(source, "B::go"))

    def test_a_keyword_and_name_in_a_comment_or_string_define_nothing(self):
        source = '// the fn tick runs here\nconst S: &str = "struct Gain { depth: u8 }";\nfn other() {}\n'
        for name in ("tick", "Gain", "depth"):
            self.assertFalse(self.defined(source, name), name)

    def test_a_non_rust_item_must_appear_as_written(self):
        self.assertTrue(index.found_in("ci.yml", "jobs:\n  core-alone:\n", "core-alone"))
        self.assertFalse(index.found_in("ci.yml", "jobs:\n  other:\n", "core-alone"))

    def test_an_index_with_no_heading_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            empty = Path(directory) / "decision-index.md"
            empty.write_text("# Index\n\nnothing listed\n", encoding="utf-8")
            was, index.INDEX = index.INDEX, empty
            try:
                with contextlib.redirect_stdout(io.StringIO()) as out:
                    status = index.main()
            finally:
                index.INDEX = was
        self.assertEqual(status, 1)
        self.assertIn("no heading found", out.getvalue())


class CoreDeps(unittest.TestCase):
    def package(self, name, path, depends=()):
        return {
            "name": name,
            "manifest_path": str(Path("/repo") / path / "Cargo.toml"),
            "dependencies": [{"name": d, "kind": None} for d in depends],
        }

    def test_a_new_plugin_under_examples_takes_the_plugin_layer(self):
        packages = [
            self.package("seam-host", "crates/host"),
            self.package("seam-example-reverb", "examples/reverb", ["seam-host"]),
        ]
        problems = deps.layering_problems(
            [p for p in packages] + [self.package(n, f"crates/{n}") for n in deps.LAYERS if n != "seam-host"],
            Path("/repo"),
        )
        self.assertEqual([p for p in problems if "reverb" in p], [])

    def test_a_crate_outside_examples_still_needs_a_layer(self):
        packages = [self.package("seam-new", "crates/new")]
        problems = deps.layering_problems(packages, Path("/repo"))
        self.assertIn("seam-new has no layer in LAYERS", problems)

    def test_a_table_entry_that_is_no_crate_fails(self):
        problems = deps.layering_problems([], Path("/repo"))
        self.assertIn("seam-host is in LAYERS but is not a workspace crate", problems)

    def test_common_ui_stacks_are_forbidden(self):
        for name in ["egui", "eframe", "iced", "iced_wgpu", "slint", "sdl2", "glfw", "x11", "x11rb",
                     "wayland-client"]:
            self.assertTrue(deps.forbidden(name), name)
        self.assertFalse(deps.forbidden("serde"))


class ExampleCoverage(unittest.TestCase):
    def test_a_crate_with_no_reachable_item_fails(self):
        with self.assertRaises(coverage.CheckError):
            coverage.missing_of("seam-x", [])

    def test_a_check_that_cannot_run_fails_with_its_reason(self):
        was, coverage.library_crates = coverage.library_crates, lambda: {}
        try:
            with contextlib.redirect_stderr(io.StringIO()) as err:
                status = coverage.main()
        finally:
            coverage.library_crates = was
        self.assertEqual(status, 1)
        self.assertIn("no library crate found", err.getvalue())

    def test_missing_names_are_listed(self):
        self.assertEqual(coverage.missing_of("seam-x", [("a", True), ("b", False)]), ["b"])

    def test_an_unknown_format_version_fails(self):
        with self.assertRaises(coverage.CheckError):
            coverage.check_format({"format_version": coverage.FORMAT_VERSION + 1}, "seam-x")
        with self.assertRaises(coverage.CheckError):
            coverage.check_format({}, "seam-x")
        coverage.check_format({"format_version": coverage.FORMAT_VERSION}, "seam-x")

    def test_methods_are_counted_per_owner_and_hidden_ones_skipped(self):
        def fn(name, docs="", visibility="public", attrs=()):
            return {"name": name, "docs": docs, "visibility": visibility, "attrs": list(attrs),
                    "inner": {"function": {}}}

        example = "```" + chr(10) + "let a = 1;" + chr(10) + "```"
        index = {
            "1": fn("shown", example),
            "2": fn("bare"),
            "3": fn("private", visibility="default"),
            "4": fn("hidden", attrs=[{"other": "#[doc(hidden)]"}]),
            "5": {"name": None, "attrs": [], "inner": {"impl": {"trait": None, "items": [1, 2, 3, 4]}}},
            "6": {"name": None, "attrs": [], "inner": {"impl": {"trait": {"path": "Clone"}, "items": [7]}}},
            "7": fn("clone"),
            "8": fn("provided", example),
        }
        owner = {"inner": {"struct": {"impls": [5, 6]}}}
        trait = {"inner": {"trait": {"items": [8]}}}
        self.assertEqual(coverage.method_ids(index, "struct", owner), ["2", "1"])
        self.assertEqual(coverage.method_ids(index, "trait", trait), ["8"])

    def test_the_method_report_lists_the_bare_ones_and_never_fails(self):
        lines = coverage.method_report("seam-x", [("A::a", True), ("A::b", False), ("B::c", False)])
        self.assertEqual(lines, ["seam-x: 1 of 3 public methods have an example", "  A::b", "  B::c"])

    def test_a_type_alias_is_counted_and_another_crates_item_is_not(self):
        example = "```" + chr(10) + "let a = 1;" + chr(10) + "```"

        def item(name, inner, docs=""):
            return {"name": name, "visibility": "public", "attrs": [], "docs": docs, "inner": inner}

        data = {"root": 0, "index": {
            "0": item("seam_x", {"module": {"items": [1, 2, 3]}}),
            "1": item("Section", {"type_alias": {}}, example),
            "2": item("WriteFn", {"type_alias": {}}),
            # A `pub use` of another crate's item, which this crate's JSON lacks.
            "3": item(None, {"use": {"id": 99, "name": "Value", "is_glob": False}}),
        }}
        self.assertEqual(coverage.Walk(data).run(), [("Section", True), ("WriteFn", False)])

    def test_only_doc_hidden_hides(self):
        self.assertTrue(coverage.is_hidden({"attrs": [{"other": "#[doc(hidden)]"}]}))
        self.assertFalse(coverage.is_hidden({"attrs": [{"other": '#[doc(alias = "hidden")]'}]}))
        self.assertFalse(coverage.is_hidden({"attrs": ["must_use"]}))


class RunLoom(unittest.TestCase):
    OUTPUT = (
        "     Running unittests src/lib.rs (target/release/deps/seam_a-0123abcd.exe)\n"
        "test queue::tests::loom_x ... ok\n"
        "     Running unittests src\\lib.rs (target\\release\\deps\\seam_b-0123abcd.exe)\n"
        "test queue::tests::loom_x ... FAILED\n"
        "test slot::loom_y ... ok\n"
    )

    def test_a_pass_belongs_to_the_crate_that_ran_it(self):
        self.assertEqual(
            loom.passed_models(self.OUTPUT),
            {("seam_a", ("queue", "tests"), "loom_x"), ("seam_b", ("slot",), "loom_y")},
        )

    def test_coloured_output_is_read_like_plain_output(self):
        coloured = (
            "\x1b[1m\x1b[92m     Running\x1b[0m unittests src/lib.rs (target/release/deps/seam_a-0123abcd)\n"
            "test queue::loom_x ... \x1b[32mok\x1b[0m\n"
        )
        self.assertEqual(loom.passed_models(coloured), {("seam_a", ("queue",), "loom_x")})

    def test_a_source_with_no_model_fails(self):
        was, loom.models_in_source = loom.models_in_source, lambda: (set(), [])
        try:
            with contextlib.redirect_stderr(io.StringIO()) as err:
                status = loom.main()
        finally:
            loom.models_in_source = was
        self.assertEqual(status, 1)
        self.assertIn("no loom model found", err.getvalue())

    def tree(self, files: dict[str, str]):
        """The models and stray calls the scan finds in a repository holding `files`."""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for crate in loom.LANE:
                (root / "crates" / crate).mkdir(parents=True)
                (root / "crates" / crate / "Cargo.toml").write_text(
                    f'[package]\nname = "seam-{crate}"\n', encoding="utf-8"
                )
            for name, text in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text, encoding="utf-8")
            return loom.models_in_source(root)

    MODEL = "fn loom_a() {\n    loom::model(|| {});\n}\n"

    def test_a_model_in_a_lane_crates_source_is_found(self):
        models, stray = self.tree({"crates/params/src/queue.rs": self.MODEL})
        self.assertEqual(models, {("seam_params", ("queue",), "loom_a")})
        self.assertEqual(stray, [])

    def test_a_model_the_lane_does_not_build_is_stray(self):
        for where in (
            "crates/params/tests/queue.rs",
            "crates/params/benches/queue.rs",
            "crates/ump/src/queue.rs",
            "examples/gain/src/lib.rs",
            "xtask/src/lib.rs",
        ):
            models, stray = self.tree({where: self.MODEL})
            self.assertEqual(models, set(), where)
            self.assertEqual(len(stray), 1, where)
            self.assertIn(where, stray[0])

    def test_a_call_outside_any_function_is_stray(self):
        _, stray = self.tree({"crates/host/src/lib.rs": "loom::model(|| {});\n"})
        self.assertEqual(len(stray), 1)

    def test_a_stray_model_fails_the_run_before_cargo_starts(self):
        was, loom.models_in_source = loom.models_in_source, lambda: ({("a", (), "loom_x")}, ["x.rs: stray"])
        started = []
        run, loom.subprocess.run = loom.subprocess.run, lambda *args, **kwargs: started.append(args)
        try:
            with contextlib.redirect_stderr(io.StringIO()) as err:
                status = loom.main()
        finally:
            loom.models_in_source = was
            loom.subprocess.run = run
        self.assertEqual(started, [])
        self.assertEqual(status, 1)
        self.assertIn("x.rs: stray", err.getvalue())

    def test_build_output_is_not_source(self):
        _, stray = self.tree({"target/debug/build/x/out.rs": self.MODEL})
        self.assertEqual(stray, [])

    def test_same_named_models_in_two_crates_are_two(self):
        ran = ("seam_a", ("queue", "tests"), "loom_x")
        self.assertTrue(loom.matches(("seam_a", ("queue",), "loom_x"), ran))
        self.assertFalse(loom.matches(("seam_b", ("queue",), "loom_x"), ran))
        self.assertFalse(loom.matches(("seam_a", ("slot",), "loom_x"), ran))


class ShapeAppendOnly(unittest.TestCase):
    BASE = "# note\nversion 2 shape aaaa\nversion 3 shape bbbb\n"

    def test_an_appended_line_passes(self):
        self.assertEqual(shape.problems(self.BASE, self.BASE + "version 4 shape cccc\n"), [])

    def test_an_overwritten_line_fails(self):
        head = "version 2 shape aaaa\nversion 3 shape ffff\n"
        self.assertEqual(len(shape.problems(self.BASE, head)), 1)

    def test_a_removed_line_fails(self):
        self.assertEqual(len(shape.problems(self.BASE, "version 2 shape aaaa\n")), 1)

    def test_a_comment_edit_passes(self):
        head = "# another note\nversion 2 shape aaaa\nversion 3 shape bbbb\n"
        self.assertEqual(shape.problems(self.BASE, head), [])

    def test_an_indented_comment_is_a_comment(self):
        head = "  # an indented note\nversion 2 shape aaaa\nversion 3 shape bbbb\n"
        self.assertEqual(shape.problems(self.BASE, head), [])
        self.assertEqual(shape.version_lines(head), ["version 2 shape aaaa", "version 3 shape bbbb"])

    def test_a_file_with_no_version_line_fails(self):
        self.assertTrue(shape.problems(self.BASE, "# nothing\n"))
        self.assertTrue(shape.problems("# nothing\n", self.BASE))
        self.assertTrue(shape.problems(None, ""))

    def test_a_new_file_passes(self):
        self.assertEqual(shape.problems(None, self.BASE), [])

    def test_a_base_this_clone_lacks_is_no_base(self):
        self.assertTrue(shape.known("HEAD"))
        self.assertFalse(shape.known("0" * 40))


def bash() -> str | None:
    """A bash that can run the script: Git's own on Windows, where `bash` on
    the path may be WSL's."""
    git = shutil.which("git")
    if git and os.name == "nt":
        for parent in Path(git).resolve().parents:
            if (parent / "bin" / "bash.exe").is_file():
                return str(parent / "bin" / "bash.exe")
        return None
    return shutil.which("bash")


class RunBenches(unittest.TestCase):
    @unittest.skipUnless(bash() and shutil.which("git"), "needs bash and git")
    def test_a_failing_run_leaves_the_checkout_where_it_was(self):
        script = (SCRIPTS / "run-benches.sh").read_text(encoding="utf-8").replace("\r\n", "\n")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo, tools = root / "repo", root / "tools"
            (repo / "crates/ump/benches").mkdir(parents=True)
            tools.mkdir()
            (tools / "cargo").write_text("#!/bin/sh\nexit 1\n", encoding="utf-8", newline="\n")
            (repo / "run-benches.sh").write_text(script, encoding="utf-8", newline="\n")
            (repo / "crates/ump/benches/iai.rs").write_text("// base\n", encoding="utf-8")

            def git(*args):
                done = subprocess.run(
                    ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                    cwd=repo, check=True, capture_output=True, text=True,
                )
                return done.stdout.strip()

            git("init", "-q", "-b", "work")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            (repo / "later.txt").write_text("later\n", encoding="utf-8")
            git("add", ".")
            git("commit", "-qm", "later")
            env = dict(os.environ, PATH=f"{tools}{os.pathsep}{os.environ['PATH']}")
            run = subprocess.run([bash(), "run-benches.sh", base], cwd=repo, env=env, capture_output=True)
            self.assertNotEqual(run.returncode, 0)
            self.assertEqual(git("symbolic-ref", "--short", "HEAD"), "work")


if __name__ == "__main__":
    unittest.main()
