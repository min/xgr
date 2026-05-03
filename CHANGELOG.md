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

## 0.1.0

### Added

- Initial Rust implementation of XcodeGen-compatible spec loading and project generation.
- `xgr` CLI with `validate`, `dump`, and `generate` commands.
- YAML and JSON spec loading with include resolution, variable expansion, target templates, schemes, settings, plists, and breakpoints.
- Deterministic PBX project generation and generated artifact writing.
- Upstream XcodeGen fixture coverage and test-inventory tracking.
- GitHub Actions CI for clippy, tests, and dependency audit.
