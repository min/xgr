<p align="center">
  <img src="assets/logo-readme.png" width="180" alt="xgr logo">
</p>

<h1 align="center">xgr</h1>

<p align="center">
  A pre-1.0 Rust implementation of XcodeGen-compatible <code>project.yml</code> loading and <code>.xcodeproj</code> generation.
</p>

<p align="center">
  <a href="https://github.com/min/xgr/actions/workflows/ci.yml"><img src="https://github.com/min/xgr/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue.svg" alt="MIT License"></a>
  <a href="https://www.rust-lang.org"><img src="https://img.shields.io/badge/rust-2021-orange.svg" alt="Rust 2021"></a>
</p>

`xgr` is built to load the same `project.yml` specs as [`yonaskolb/XcodeGen`](https://github.com/yonaskolb/XcodeGen) and generate matching `.xcodeproj` output, ported to Rust. The upstream XcodeGen Swift test inventory (75 methods) is ported and enforced; checked fixtures match upstream byte-for-byte. Public benchmark runs are currently 2-3x faster on the measured projects in [`BENCHMARKS.md`](BENCHMARKS.md).

## Quickstart

```yaml
# project.yml
name: HelloWorld
targets:
  HelloWorld:
    type: application
    platform: iOS
    sources: [Sources]
```

```sh
xgr generate --spec project.yml
```

Produces a `HelloWorld.xcodeproj` byte-for-byte equivalent to what upstream XcodeGen would generate from the same spec. See [Usage](#usage) for `validate` and `dump`.

## Status

`xgr` is pre-1.0. It is intended for compatibility testing, automation experiments, and projects that can compare generated output before adopting it. Generate to a temporary path and diff against upstream XcodeGen before replacing checked-in projects.

<details>
<summary><strong>Implemented features</strong></summary>

- YAML and JSON spec loading
- `include` resolution with `relativePaths`, `enable`, duplicate include protection, additive merging, and `:REPLACE`
- Environment / template variable expansion
- Target, scheme, and nested target-template merging
- Multi-platform target expansion
- Typed models for projects, targets, dependencies, sources, settings, schemes, plists, and breakpoints
- Deterministic PBX project generation
- Scheme, breakpoint, generated plist, and entitlement file writing
- XcodeGen `preGenCommand` and `postGenCommand` execution when writing a project
- Upstream XcodeGen fixture coverage and test-inventory tracking
- GitHub Actions CI for formatting, clippy, tests, and dependency audit

</details>

## Compatibility

The compatibility target is upstream XcodeGen behavior for `project.yml` specs. Current coverage includes the upstream Swift test inventory in this checkout, plus byte-for-byte `project.pbxproj` parity for the checked fixture goldens listed in [`TEST_PARITY.md`](TEST_PARITY.md).

Known limitations:

- `preGenCommand` and `postGenCommand` are executed by project-writing paths (`xgr generate` and `ProjectWriter::write`), but not by in-memory generation (`ProjectWriter::generate`).
- Compatibility is measured against the vendored `upstream-xcodegen` checkout. Updating it requires rerunning the inventory workflow documented in [`TEST_PARITY.md`](TEST_PARITY.md).
- Not yet a drop-in replacement for every real-world XcodeGen configuration. If output differs from upstream, please file a compatibility issue with the spec and a description of the expected output.

See [`BENCHMARKS.md`](BENCHMARKS.md) for the public real-world comparison harness and measured results.

## Install

### crates.io

```sh
cargo install xgr --locked
```

### Homebrew

```sh
brew tap min/xgr https://github.com/min/xgr
brew install xgr            # stable: latest tagged release
brew install --HEAD xgr     # bleeding edge: builds from main
```

### From source

```sh
git clone --recurse-submodules https://github.com/min/xgr.git
cd xgr
cargo install --path . --locked
```

If you cloned without submodules:

```sh
git submodule update --init --recursive
```

Prebuilt binary release artifacts are not yet attached to GitHub releases.

## Usage

```sh
# Validate a spec
xgr validate --spec project.yml

# Print the resolved JSON form
xgr dump --spec project.yml

# Generate an Xcode project
xgr generate --spec project.yml

# Generate to an explicit path
xgr generate --spec path/to/project.yml --output path/to/Project.xcodeproj
```

From an uninstalled checkout, build first and run the local binary directly:

```sh
cargo build --release --locked
target/release/xgr generate --spec path/to/project.yml --output path/to/Project.xcodeproj
```

## Development

```sh
cargo fmt
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo audit --deny warnings
```

`--all-features` enables the internal `__upstream-fixture-golden` cargo feature that the upstream XcodeGen fixture-parity test depends on. Default `cargo test` skips that suite.

Local benchmark artifacts should stay under `.context/bench`, which is ignored by git.

For public real-world XcodeGen comparisons:

```sh
scripts/bench_public_xcodegen.sh --only element-ios
```

The script keeps cloned repositories, generated projects, diffs, and timing JSON under `.context/bench/public-xcodegen`. It compares upstream XcodeGen output against `xgr` byte-for-byte for `project.pbxproj` and the full generated `.xcodeproj`, then runs timing benchmarks with `hyperfine` when it is installed.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full contribution workflow and [`CHANGELOG.md`](CHANGELOG.md) for release history.

## Security

Treat project specs as trusted project configuration, not sandboxed input. `xgr` reads files referenced by specs and writes generated project artifacts to requested output paths.

`preGenCommand` and `postGenCommand` are executed when writing a project, and generated Xcode projects may still contain build scripts that Xcode can execute later.

See [`SECURITY.md`](SECURITY.md) for vulnerability reporting guidance.
