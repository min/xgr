# Changelog

All notable changes to this project should be documented in this file.

This project is pre-1.0. Until a stable compatibility policy is published, minor versions may include CLI, API, or generated-output changes.

## Unreleased

### Added

- Public README guidance for status, compatibility, source installation, and security expectations.
- Contributor workflow covering required checks and upstream XcodeGen parity maintenance.
- GitHub issue templates for bugs, compatibility gaps, and feature requests.
- Pull request template with verification and compatibility notes.
- Cargo package metadata for public discovery.
- CI formatting enforcement with `cargo fmt --all --check`.
- Internal `__upstream-fixture-golden` cargo feature that gates the upstream
  fixture-parity test; CI enables it via `--all-features`.

### Changed

- Replaced unmaintained `serde_yaml 0.9` with `serde_norway 0.9`, `md5 0.7`
  with `md-5 0.10`, and bumped `thiserror` to `2.0`. `cargo audit` is clean.
- Reduced public API surface: the `pbxproj` and `spec` modules are now
  `#[doc(hidden)]`, with a curated set of types re-exported at the crate root.
  Internal helpers (`JsonMap`, `format_deployment_target`, etc.) are no longer
  reachable from outside the crate.
- `SpecLoader` now exposes `load_project` as an associated function instead of
  a `&mut self` instance method; the previously held `project_dictionary`
  cache is gone (load through `SpecFile::resolved_dictionary` instead).
- `tests/performance_tests.rs` renamed to `tests/performance_smoke_tests.rs`
  to reflect that it is functional smoke coverage, not a benchmark.
- Started splitting `src/pbxproj.rs` into a module directory; the
  XcodeGen-reference-id pass now lives in `src/pbxproj/references.rs` and
  the plist writer in `src/pbxproj/plist.rs`.
- `SpecError` lost six near-duplicate `UnknownBreakpoint*` variants in favor
  of a single `UnknownBreakpoint { kind: BreakpointField, value: String }`.
- Crate-level rustdoc added for `lib.rs`.

### Fixed

- `expand_string` in spec resolution is now order-deterministic. Previous
  implementation iterated a `HashMap`, which produced nondeterministic output
  if any variable expansion contained another variable's reference.
- Removed `unwrap()` calls in `add_native_target` for files that participate
  in build phases; replaced with a typed helper that filter-maps cleanly.
- `ProjectWriter::generate` and the internal `PbxGenerator` chain now return
  `Result<_, ProjectWriteError>`. Missing build-script paths surface as
  `ProjectWriteError::Read` instead of swallowing the error or panicking.

### Performance

- `XcodeReferenceGenerator` no longer clones each `PbxObject` during the
  reference pass; it borrows from the graph directly.
- `mapped_id` returns `Cow<'_, str>` instead of allocating per call.
- Replaced `"\t".repeat(indent)` allocations across plist and PBX
  serialization with a static tab table (`write_tabs`).
- `SpecFile::merge_unique` consumes `self`, eliminating one full-dictionary
  clone per spec node during resolution.
- `expand_variables_in_map` skips the `mem::take` rebuild when no map key
  contains a `${variable}` reference, walking values in place instead.

## 0.1.0

### Added

- Initial Rust implementation of XcodeGen-compatible spec loading and project generation.
- `xgr` CLI with `validate`, `dump`, and `generate` commands.
- YAML and JSON spec loading with include resolution, variable expansion, target templates, schemes, settings, plists, and breakpoints.
- Deterministic PBX project generation and generated artifact writing.
- Upstream XcodeGen fixture coverage and test-inventory tracking.
- GitHub Actions CI for clippy, tests, and dependency audit.
