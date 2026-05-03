# XcodeGenRust Upstream Test Plan

The upstream XcodeGen test-porting pass is complete for the current checkout.

## Completion Criteria

- All top-level upstream Swift test methods outside `Tests/Fixtures` are inventoried.
- Every inventoried upstream test method has Rust coverage.
- The inventory is enforced by `tests/upstream_test_inventory.rs`.
- The upstream fixture `project.pbxproj` goldens used by the port are checked byte-for-byte through an explicit test helper, while normal generation remains on the Rust generator path.
- `cargo test` is the required verification command.

## Residual Notes

- XCTest performance tests are represented as Rust smoke tests in `tests/performance_tests.rs`, not as benchmark timing assertions.
- The upstream test inventory intentionally ignores helper functions and nested local functions, including the nested `func test(generateEmptyDirectories:)` helper inside `SourceGeneratorTests.swift`.
- If upstream XcodeGen is updated, first run the inventory guard. A failure means the porting map must be updated before claiming parity again.

## Maintenance Workflow

1. Update the `upstream-xcodegen` checkout.
2. Run `cargo test --test upstream_test_inventory`.
3. Port any newly discovered upstream test behavior.
4. Update `TEST_PARITY.md` and `tests/upstream_test_inventory.rs`.
5. Run full `cargo test`.
