# Oxidegen Remaining Upstream Test Plan

This is the working plan to port the rest of `yonaskolb/xcodegen` test coverage while keeping `cargo test` green after each batch.

## Phase 1: ProjectSpec Validation

- Finish `ProjectSpecTests.testValidation`.
- Cover minimum XcodeGen version checks.
- Cover missing/default config validation and per-config settings mistakes.
- Cover target, aggregate target, scheme, project reference, SDK dependency, source path, script path, test plan, and supported-destination validation.
- Keep validation errors structured so tests can assert exact cases.

## Phase 2: Remaining SpecLoading Edges

- Finish storeKit/test plan/build tool plugin edge cases.
- Add alternate scheme variable cases not yet represented.
- Add warning validation behavior for deprecated/new placeholder usage.
- Expand template cycle and nested-template ordering cases.

## Phase 3: XcodeGenKit Source Generator

- Port source group, synced folder, folder reference, variant group, localized resources, Core Data, framework source, include/exclude, file type, resource tag, and sort-order tests.
- Implement enough source graph generation to assert object shapes directly.

Progress:

- Completed source/resource/build header phase classification, explicit source build phase overrides, localized intent definition phase handling, XcodeGen default file-type classification, custom `options.fileTypes`, framework sources, Core Data model and mapping model source phases, source de-duping, source renaming for explicit files, default ignored files/extensions, include/exclude filters including bracket/range globs and no-match include sets, `Info.plist` resource exclusion, folder reference sources, optional missing file/folder/group behavior, destination filters, inferred destination filters, compiler flags, attributes, resource tags, project known asset tags, known region detection from `.lproj` and string catalogs, header visibility for framework targets, and public static library header copy phases.
- Expanded synced folder support: synchronized root groups, target attachment, deduping, default synced-folder directory type, explicit folder glob expansion, merged explicit folders across targets, per-target membership exceptions, and `Info.plist` exceptions.

## Phase 4: PBX Project Generator

- Port PBX group ordering, package sorting, project metadata, target dependency, build phase ordering, products group, package reference, local package, and embed/copy phase tests.
- Port generated plist artifact tests.
- Replace the ignored fixture golden test with active byte-for-byte `project.pbxproj` parity.

Progress:

- Completed target dependencies, aggregate target dependencies, aggregate build scripts, products group behavior, build phase ordering, package references, local package references, local package grouping/exclusion, source/dependency destination filters, weak target dependency build file settings, copy/embed phase basics, bundle dependency resource copy phases, embedded target dependency copy phases, embedded framework/SDK/package dependency copy phases, project `LastUpgradeCheck`, generated `Info.plist`/entitlements files, `Info.plist` resource exclusion behavior, run script phases, custom build rules, and the Swift static library Objective-C interface header copy phase.
- Completed ProjectGenerator build-setting defaults for bundle identifiers, development language, default configuration, single-platform project SDK root, deployment targets, supported destinations, static framework embedding, and Objective-C linker propagation.
- Completed target attributes, cyclic target dependency generation, local Swift package group/top-level placement, multiple package products, and Carthage static/dynamic search path and copy behavior.
- Added Carthage option coverage for project-level framework discovery, custom build paths, custom executable paths, platform-specific copy-frameworks inputs, and dynamic-framework copy script generation.
- Expanded source navigator graph generation for source groups, intermediate groups, custom groups, folder references with intermediate groups, duplicate display-name roots, relative outside-base folders, and dependency Frameworks groups.
- Added partial config settings and `settingPresets: none` coverage in generated PBX build settings.

## Phase 5: Scheme And Breakpoint Generators

- Port scheme generation XML tests, target scheme variants, environment variables, storeKit references, test plans, macro expansion, screenshot settings, management metadata, and debugger toggles.
- Port breakpoint file generation tests.

Progress:

- Started real shared scheme file writing, including build/test/run/profile/analyze/archive actions, buildable references, target scheme variants, environment variables, command-line arguments, launch language/region attributes, storeKit references, test plans, code coverage targets, macro expansion, local Swift package test references, profile ask-to-launch, and pre/post actions.
- Added scheme launch/test details for custom LLDB init files, custom working directories, ask-to-launch, GPU capture mode passthrough, and test-target location references.
- Added scheme management plist writing for hidden target schemes.
- Added external project build and code coverage references.
- Added watch app target scheme remote runnable output and host app build action insertion.
- Started shared breakpoint file writing for exception and file breakpoints.

## Phase 6: Resolvers, Fixtures, And Performance

- Port Carthage dependency resolver tests.
- Add fixture outputs for schemes, plists, breakpoints, SwiftPM metadata, and cache files.
- Port performance tests as ignored or benchmark-style tests once functional parity is stable.

Progress:

- Completed write-level fixture generation coverage for TestProject/AnotherProject, TestProject, CarthageProject, and SPM.
- Added fixture scheme and breakpoint artifact assertions for written projects.
- Accounted for all sample fixture XCTest methods in the generated project graph.

## Iteration Rule

For every batch:

1. Add or unignore tests that mirror upstream behavior.
2. Implement the narrowest missing behavior.
3. Run `cargo fmt && cargo test`.
4. Update `TEST_PARITY.md`.
