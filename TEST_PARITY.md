# XcodeGen Test Parity

This tracks upstream `yonaskolb/XcodeGen` test coverage ported into the Rust implementation.

## Current Status

- Upstream top-level Swift test methods in this checkout: 75.
- Rust porting status for those upstream methods: 75 accounted for.
- Current Rust suite: 205 tests under `cargo test --all-features`; default
  `cargo test` runs 181 tests and skips the feature-gated upstream fixture
  golden test binary.
- Fixture `project.pbxproj` parity: byte-for-byte equality is enforced for all checked upstream generated fixture goldens through an explicit test helper. The normal `ProjectWriter::generate` and CLI paths always run the Rust generator rather than reading fixture goldens.

The executable inventory guard lives in `tests/upstream_test_inventory.rs`. It walks `upstream-xcodegen/Tests`, excludes `Tests/Fixtures`, extracts top-level Swift `func test...` methods, and asserts the inventory matches the ported list. This prevents us from silently missing a newly added upstream test method or carrying a stale mapping.

## Ported Coverage

### Fixture Tests

- `FixtureTests.testProjectFixture`
- Rust coverage:
  - `tests/upstream_fixtures.rs`
  - fixture spec loading for the primary upstream fixture set
  - fixture project writing for TestProject, AnotherProject, CarthageProject, SPM, and scheme fixtures
  - byte-for-byte `project.pbxproj` equality for TestProject, AnotherProject, CarthageProject, SPM, and scheme fixtures
  - scheme and breakpoint artifact existence checks

### Performance Tests

- `PerformanceTests.testLoading`
- `PerformanceTests.testGeneration`
- `PerformanceTests.testWriting`
- `PerformanceTests.testFixtureDecoding`
- `PerformanceTests.testCacheFileGeneration`
- `PerformanceTests.testFixtureGeneration`
- `PerformanceTests.testFixtureWriting`
- Rust coverage:
  - `tests/performance_smoke_tests.rs`
  - ported as deterministic smoke tests for the same loading, generation, writing, fixture decoding, and cache-payload paths

Note: XCTest `measure { ... }` timing assertions are not reproduced as Rust benchmark measurements in the normal test suite. The functional operations are covered and run under `cargo test`.

### ProjectSpec Tests

- `Dictionary+Extension_Tests.testRemovingNil_ShouldReturnNewDictionaryWithoutOptionalValues`
- `InvalidConfigsFormatTests.testInvalidConfigsMappingFormat`
- `ProjectSpecTests.testTargetType`
- `ProjectSpecTests.testTargetFilename`
- `ProjectSpecTests.testDeploymentTarget`
- `ProjectSpecTests.testValidation`
- `ProjectSpecTests.testJSONEncodable`
- `SpecLoadingTests.testSpecLoaderDuplicateImports`
- `SpecLoadingTests.testSpecLoader`
- `SpecLoadingTests.testSpecLoaderLoadingJSON`
- `SpecLoadingTests.testSpecWarningValidation`
- `SpecLoadingTests.testProjectSpecParser`
- `SpecLoadingTests.testPackagesVersion`
- `SpecLoadingTests.testDecoding`
- Rust coverage:
  - `src/spec.rs` unit tests
  - `tests/upstream_fixtures.rs`
  - null/empty removal, target type metadata, target filenames, deployment target formatting, validation, JSON/YAML loading, includes, relative path rewriting, warning validation, package version validation, template/environment expansion, build scripts, build rules, plugins, aggregate targets, options, packages, target schemes, schemes, settings, plists, breakpoints, and fixture decoding

### XcodeGenCore Tests

- `ArrayExtensionsTests.*`
- `AtomicTests.testSimultaneousWriteOrder`
- `GlobTests.*`
- `PathExtensionsTests.testPathRelativeToPath`
- Rust coverage:
  - `tests/core_tests.rs`
  - sorted-array search/sorting semantics, atomic concurrent writes, relative paths, brace globs, direct access, Bash v3/v4 globstar behavior, Gradle globstar behavior, indexing/repeated iteration behavior, and blacklisted directories

### XcodeGenKit Tests

- `BreakpointGeneratorTests.testBreakpoints`
- `CarthageDependencyResolverTests.*`
- `PBXProjGeneratorTests.*`
- `ProjectGeneratorTests.*`
- `SchemeGeneratorTests.*`
- `SourceGeneratorTests.testSourceGenerator`
- Rust coverage:
  - `tests/pbx_generator_tests.rs`
  - `tests/scheme_writer_tests.rs`
  - `tests/upstream_fixtures.rs`
  - `tests/performance_smoke_tests.rs`

Covered behavior includes:

- breakpoint XML writing
- Carthage default/custom build paths and executable paths
- Carthage platform build paths, related framework discovery, deduping, sorting, top-level target dependency resolution, transitive/aggregate resolution, direct embedding, custom copy phases, and platform filtering
- PBX group ordering, product groups, `LastUpgradeCheck`, platform dependencies, target dependencies, aggregate targets, run scripts, build rules, generated plists/entitlements, local/remote Swift packages, weak dependencies, source/dependency destination filters, target attributes, build-setting presets, copy/embed phases, headers phases, file-type classification, known regions, known asset tags, source groups, synced folders, folder references, include/exclude filters, optional sources, localized intent definitions, duplicate source handling, and all active fixture pbxproj goldens
- ProjectGenerator options, config generation, aggregate targets, target generation, destination-filtered generation, platform-filtered dependencies, and custom dependency destinations
- Scheme generation, last-upgrade-version default/override behavior, hidden target scheme management, target scheme variants, environment variables, command-line arguments, macro expansion, test plans, code coverage, external project references, watch app runnable behavior, checker toggles, screenshot capture settings, and location simulation

## Maintenance Rule

When updating the upstream XcodeGen checkout:

1. Run `cargo test --test upstream_test_inventory`.
2. If the inventory changes, add or intentionally retire the corresponding Rust coverage.
3. Update `EXPECTED_UPSTREAM_TESTS` and keep `PORTED_UPSTREAM_TESTS` equal only when the new behavior is represented.
4. Run full `cargo test`.
