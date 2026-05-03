# Contributing

Thanks for helping improve `XcodeGenRust`. The project goal is practical compatibility with upstream XcodeGen, so changes should preserve deterministic output and include focused tests for any behavior difference.

## Setup

Clone with submodules:

```sh
git clone --recurse-submodules https://github.com/min/XcodeGenRust.git
cd XcodeGenRust
```

If you already cloned without submodules:

```sh
git submodule update --init --recursive
```

## Required Checks

Run these before opening a pull request:

```sh
cargo fmt
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

If `cargo-audit` is installed, also run:

```sh
cargo audit --deny warnings
```

## Compatibility Changes

For behavior that is intended to match upstream XcodeGen:

1. Add or update Rust coverage for the relevant spec behavior.
2. Prefer fixture-based tests when the behavior affects generated `.xcodeproj` output.
3. Keep generated output deterministic.
4. Update `TEST_PARITY.md` when the upstream parity story changes.

The upstream inventory guard is:

```sh
cargo test --test upstream_test_inventory
```

When updating the `upstream-xcodegen` checkout:

1. Run `cargo test --test upstream_test_inventory`.
2. Port any newly discovered upstream behavior or explicitly document why it is out of scope.
3. Update `EXPECTED_UPSTREAM_TESTS` and `PORTED_UPSTREAM_TESTS` in `tests/upstream_test_inventory.rs`.
4. Update `TEST_PARITY.md`.
5. Run the full required checks.

## Reporting Compatibility Bugs

Compatibility reports are most useful when they include:

- A minimal `project.yml` or a small reproduction repository.
- The upstream XcodeGen version or commit used for comparison.
- The `xgr` command that produced the unexpected output.
- The specific generated file or section that differs.
- Whether the difference affects Xcode behavior or only textual output.

Do not include private signing material, credentials, proprietary source files, or unreduced project files unless they are safe to publish.

## Pull Request Scope

Keep pull requests focused. Documentation, refactors, fixture updates, and behavior changes are easier to review when they are separate unless they are required for the same fix.

Public API and CLI changes should update `README.md` and `CHANGELOG.md`.

## Release Process

The project is currently pre-1.0. Before publishing a tag or crate release:

1. Move relevant `CHANGELOG.md` entries from `Unreleased` into the target version.
2. Confirm the README status and known limitations still match the release.
3. Run the required checks on a machine with Rust tooling installed.
4. Confirm generated fixture parity for any compatibility-sensitive changes.
5. Tag only after the public installation path is accurate for that release.
