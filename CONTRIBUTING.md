# Contributing

Every change reaches `main` through a pull request, squash-merged, so `main`
keeps one commit per change, each building and passing its tests.

## The pull request

- **Title.** It becomes the commit's subject, so it is a [Conventional
  Commits][conventional] subject in the form `git log --oneline` shows:
  `type(scope): what changed`. GitHub adds the pull request's number. Name the
  change, never the process that produced it.
- **Description.** What and why, the checks run, review notes and `Closes #N`.
  None of that goes in the commit.
- **Draft until ready.** CI runs once a pull request is ready for review, and
  every run spends runner minutes. Its lanes are a thin gate: Linux, macOS and
  the checks that cost seconds. So run the checks in the README's [Develop
  section](README.md#develop) on your own machine first, and push when the
  branch is ready. A change the thin gate cannot judge, such as one to an
  exporter, gets the full run: `gh workflow run ci.yml --ref <branch>`.
- **Docs move with the code.** A change to behaviour updates the decision
  record and its index. A change a plugin author sees adds a line under
  `[Unreleased]` in [CHANGELOG.md](CHANGELOG.md).
- **Plans stay out.** Working plans and tickets are not committed.
  [ROADMAP.md](ROADMAP.md) holds the big picture, and changes in a pull
  request of its own.

## The merge

Fetch first. If `main` moved, rebase onto it; never merge `main` into the
branch. `main` is not tested again after a merge, so the rebased branch is what
must have passed.

```sh
gh pr merge <number> --squash --body-file <file>
```

The body says why, and the design-level facts, in about ten lines. It never
restates the diff, and it credits no tool.

## The toolchain

`rust-toolchain.toml` and the workspace's `rust-version` move together, so CI
always builds the minimum supported Rust version.

[conventional]: https://www.conventionalcommits.org/en/v1.0.0/
