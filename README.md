<p align="center">
  <img src="assets/logo-readme.png" width="180" alt="XGR logo">
</p>

# XGR

`xgr` is a Rust implementation of XcodeGen-compatible `project.yml` loading and `.xcodeproj` generation.

The goal is practical parity with [`yonaskolb/XcodeGen`](https://github.com/yonaskolb/XcodeGen): load the same project specs, generate deterministic Xcode project files, and keep behavior covered by upstream fixture tests.

## Status

`XGR` is pre-1.0 software. It is intended for compatibility testing, automation experiments, and projects that can compare generated output before adopting it. It should not be treated as a drop-in replacement for upstream XcodeGen without checking the generated `.xcodeproj` in your repository.

Implemented:

- YAML and JSON spec loading.
- `include` resolution with `relativePaths`, `enable`, duplicate include protection, additive merging, and `:REPLACE`.
- Environment/template variable expansion.
- Target, scheme, and nested target-template merging.
- Multi-platform target expansion.
- Typed models for projects, targets, dependencies, sources, settings, schemes, plists, and breakpoints.
- Deterministic PBX project generation.
- Scheme, breakpoint, generated plist, and entitlement file writing.
- XcodeGen `postGenCommand` execution when writing a project.
- Upstream XcodeGen fixture coverage and test-inventory tracking.
- GitHub Actions CI for formatting, clippy, tests, and dependency audit.

See `TEST_PARITY.md` for the current upstream test parity map.

## Compatibility

The compatibility target is upstream XcodeGen behavior for `project.yml` specs. Current coverage includes the upstream Swift test inventory in this checkout, plus byte-for-byte `project.pbxproj` parity for the checked fixture goldens listed in `TEST_PARITY.md`.

Known limitations:

- XcodeGen `preGenCommand` hooks are not executed.
- `postGenCommand` is executed by project-writing paths (`xgr generate` and
  `ProjectWriter::write`), but not by in-memory generation
  (`ProjectWriter::generate`).
- Compatibility is measured against the vendored `upstream-xcodegen` checkout. Updating that checkout requires rerunning the inventory workflow documented in `TEST_PARITY.md`.
- The project is not yet released as a supported replacement for every real-world XcodeGen configuration. If output differs from upstream XcodeGen, please file a compatibility issue with the spec and a description of the expected output.

## Install With Homebrew

```sh
brew tap min/xgr https://github.com/min/xgr
brew install xgr
```

This installs the `xgr` CLI.

## Install From Source

```sh
git clone --recurse-submodules https://github.com/min/xgr.git
cd xgr
cargo build --release --locked
```

If the repo was cloned without submodules:

```sh
git submodule update --init --recursive
```

Install the CLI locally from a checkout:

```sh
cargo install --path . --locked
```

Published crates.io packages and binary release artifacts are not available yet. Until then, source builds are the supported installation path.

## Usage

Validate a spec:

```sh
xgr validate --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
```

Print the resolved JSON form:

```sh
xgr dump --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
```

Generate an Xcode project:

```sh
xgr generate --spec upstream-xcodegen/Tests/Fixtures/SPM/project.yml
```

Generate to an explicit path:

```sh
xgr generate \
  --spec path/to/project.yml \
  --output path/to/Project.xcodeproj
```

From an uninstalled checkout, build first and run the local binary directly:

```sh
cargo build --release --locked
target/release/xgr generate --spec path/to/project.yml --output path/to/Project.xcodeproj
```

When evaluating a real project, generate into a temporary path first and compare the generated project against upstream XcodeGen before replacing checked-in files.

## Development

```sh
cargo fmt
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo audit --deny warnings
```

`--all-features` enables the internal `__upstream-fixture-golden` cargo feature
that the upstream XcodeGen fixture-parity test depends on. Default
`cargo test` skips that suite.

Local benchmark artifacts should stay under `.context/bench`, which is ignored by git.

For public real-world XcodeGen comparisons, use:

```sh
scripts/bench_public_xcodegen.sh --only element-ios
```

The script keeps cloned repositories, generated projects, diffs, and timing JSON under
`.context/bench/public-xcodegen`. It compares upstream XcodeGen output against `xgr`
byte-for-byte for `project.pbxproj` and the full generated `.xcodeproj`, then runs
timing benchmarks with `hyperfine` when it is installed.

See `CONTRIBUTING.md` for the full contribution workflow, upstream fixture maintenance process, and issue-reporting expectations.

See `CHANGELOG.md` for release history and unreleased public-prep changes.

## Security Model

Treat project specs as trusted project configuration, not sandboxed input. `xgr` reads files referenced by specs and writes generated project artifacts to requested output paths.

`xgr` does not currently execute XcodeGen `preGenCommand` hooks. `postGenCommand`
is executed when writing a project, and generated Xcode projects may still
contain build scripts that Xcode can execute later.

See `SECURITY.md` for vulnerability reporting guidance.
