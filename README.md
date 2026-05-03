<p align="center">
  <img src="assets/logo-readme.png" width="144" alt="XcodeGenRust logo">
</p>

# XcodeGenRust

`XcodeGenRust` is a Rust implementation of XcodeGen-compatible `project.yml` loading and `.xcodeproj` generation. The CLI is `xgr`.

The goal is practical parity with [`yonaskolb/XcodeGen`](https://github.com/yonaskolb/XcodeGen): load the same project specs, generate deterministic Xcode project files, and keep behavior covered by upstream fixture tests.

## Status

Implemented:

- YAML and JSON spec loading.
- `include` resolution with `relativePaths`, `enable`, duplicate include protection, additive merging, and `:REPLACE`.
- Environment/template variable expansion.
- Target, scheme, and nested target-template merging.
- Multi-platform target expansion.
- Typed models for projects, targets, dependencies, sources, settings, schemes, plists, and breakpoints.
- Deterministic PBX project generation.
- Scheme, breakpoint, generated plist, and entitlement file writing.
- Upstream XcodeGen fixture coverage and test-inventory tracking.
- GitHub Actions CI for tests, clippy, and dependency audit.

See `TEST_PARITY.md` for the current upstream test parity map.

## Install From Source

```sh
git clone --recurse-submodules https://github.com/min/XcodeGenRust.git
cd XcodeGenRust
cargo build --release --locked
```

If the repo was cloned without submodules:

```sh
git submodule update --init --recursive
```

## Usage

Validate a spec:

```sh
cargo run --bin xgr -- validate --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
```

Print the resolved JSON form:

```sh
cargo run --bin xgr -- dump --spec upstream-xcodegen/Tests/Fixtures/TestProject/project.yml
```

Generate an Xcode project:

```sh
cargo run --bin xgr -- generate --spec upstream-xcodegen/Tests/Fixtures/SPM/project.yml
```

Generate to an explicit path:

```sh
cargo run --bin xgr -- generate \
  --spec path/to/project.yml \
  --output path/to/Project.xcodeproj
```

## Development

```sh
cargo fmt
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo audit --deny warnings
```

Local benchmark artifacts should stay under `.context/bench`, which is ignored by git.

## Security Model

Treat project specs as trusted project configuration, not sandboxed input. `xgr` reads files referenced by specs and writes generated project artifacts to requested output paths.

`xgr` does not currently execute XcodeGen `preGenCommand` or `postGenCommand` hooks. Generated Xcode projects may still contain build scripts that Xcode can execute later.

See `SECURITY.md` for vulnerability reporting guidance.
