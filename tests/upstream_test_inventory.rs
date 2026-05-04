use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const EXPECTED_UPSTREAM_TESTS: &[&str] = &[
    "FixtureTests/FixtureTests.swift:testProjectFixture",
    "PerformanceTests/PerformanceTests.swift:testCacheFileGeneration",
    "PerformanceTests/PerformanceTests.swift:testFixtureDecoding",
    "PerformanceTests/PerformanceTests.swift:testFixtureGeneration",
    "PerformanceTests/PerformanceTests.swift:testFixtureWriting",
    "PerformanceTests/PerformanceTests.swift:testGeneration",
    "PerformanceTests/PerformanceTests.swift:testLoading",
    "PerformanceTests/PerformanceTests.swift:testWriting",
    "ProjectSpecTests/Dictionary+Extension_Tests.swift:testRemovingNil_ShouldReturnNewDictionaryWithoutOptionalValues",
    "ProjectSpecTests/InvalidConfigsFormatTests.swift:testInvalidConfigsMappingFormat",
    "ProjectSpecTests/ProjectSpecTests.swift:testDeploymentTarget",
    "ProjectSpecTests/ProjectSpecTests.swift:testJSONEncodable",
    "ProjectSpecTests/ProjectSpecTests.swift:testTargetFilename",
    "ProjectSpecTests/ProjectSpecTests.swift:testTargetType",
    "ProjectSpecTests/ProjectSpecTests.swift:testValidation",
    "ProjectSpecTests/SpecLoadingTests.swift:testDecoding",
    "ProjectSpecTests/SpecLoadingTests.swift:testPackagesVersion",
    "ProjectSpecTests/SpecLoadingTests.swift:testProjectSpecParser",
    "ProjectSpecTests/SpecLoadingTests.swift:testSpecLoader",
    "ProjectSpecTests/SpecLoadingTests.swift:testSpecLoaderDuplicateImports",
    "ProjectSpecTests/SpecLoadingTests.swift:testSpecLoaderLoadingJSON",
    "ProjectSpecTests/SpecLoadingTests.swift:testSpecWarningValidation",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testEmpty",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testEmptyArray",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testIndexCannotBeFound",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testSearchingForFirstIndex",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testSearchingReturnsFirstIndexWhenMultipleElementsHaveSameValue",
    "XcodeGenCoreTests/ArrayExtensionsTests.swift:testSortingOnInitialization",
    "XcodeGenCoreTests/AtomicTests.swift:testSimultaneousWriteOrder",
    "XcodeGenCoreTests/GlobTests.swift:testBlacklistedDirectories",
    "XcodeGenCoreTests/GlobTests.swift:testBraces",
    "XcodeGenCoreTests/GlobTests.swift:testDirectAccess",
    "XcodeGenCoreTests/GlobTests.swift:testDoubleGlobstarBashV3",
    "XcodeGenCoreTests/GlobTests.swift:testDoubleGlobstarBashV4",
    "XcodeGenCoreTests/GlobTests.swift:testDoubleGlobstarBashV4WithFileExtension",
    "XcodeGenCoreTests/GlobTests.swift:testDoubleGlobstarGradle",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV3NoSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV3WithSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV3WithSlashAndWildcard",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV4NoSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV4WithSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarBashV4WithSlashAndWildcard",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarGradleNoSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarGradleWithSlash",
    "XcodeGenCoreTests/GlobTests.swift:testGlobstarGradleWithSlashAndWildcard",
    "XcodeGenCoreTests/GlobTests.swift:testIndexing",
    "XcodeGenCoreTests/GlobTests.swift:testIterateTwice",
    "XcodeGenCoreTests/GlobTests.swift:testNothingMatches",
    "XcodeGenCoreTests/PathExtensionsTests.swift:testPathRelativeToPath",
    "XcodeGenKitTests/BreakpointGeneratorTests.swift:testBreakpoints",
    "XcodeGenKitTests/CarthageDependencyResolverTests.swift:testBaseBuildPath",
    "XcodeGenKitTests/CarthageDependencyResolverTests.swift:testBuildPathForPlatform",
    "XcodeGenKitTests/CarthageDependencyResolverTests.swift:testDependenciesForTopLevelTarget",
    "XcodeGenKitTests/CarthageDependencyResolverTests.swift:testExecutablePath",
    "XcodeGenKitTests/CarthageDependencyResolverTests.swift:testRelatedDependenciesForPlatform",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testDefaultLastUpgradeCheckWhenUserDidNotSpecifyValue",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testDefaultLastUpgradeCheckWhenUserDidSpecifyInvalidValue",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testGroupOrdering",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testOverrideLastUpgradeCheckWhenUserDidSpecifyValue",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testPlatformDependencies",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testProductsGroupIsSet",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testProductsGroupIsSetWithMultipleTargets",
    "XcodeGenKitTests/PBXProjGeneratorTests.swift:testProductsGroupIsSetWithNoTargets",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testAggregateTargets",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testConfigGenerator",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testGenerateXcodeProjectWithCustomDependencyDestinations",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testGenerateXcodeProjectWithDestination",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testGenerateXcodeProjectWithPlatformFilteredDependencies",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testOptions",
    "XcodeGenKitTests/ProjectGeneratorTests.swift:testTargets",
    "XcodeGenKitTests/SchemeGeneratorTests.swift:testDefaultLastUpgradeVersionWhenUserDidNotSpecify",
    "XcodeGenKitTests/SchemeGeneratorTests.swift:testGenerateSchemeManagementOnHiddenTargetScheme",
    "XcodeGenKitTests/SchemeGeneratorTests.swift:testOverrideLastUpgradeVersionWhenUserDidSpecify",
    "XcodeGenKitTests/SchemeGeneratorTests.swift:testSchemes",
    "XcodeGenKitTests/SourceGeneratorTests.swift:testSourceGenerator",
];

fn upstream_tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("upstream-xcodegen")
        .join("Tests")
}

fn available_upstream_tests_dir() -> Option<PathBuf> {
    let tests_dir = upstream_tests_dir();
    if tests_dir.exists() {
        Some(tests_dir)
    } else {
        eprintln!(
            "skipping upstream test inventory checkout comparison; run `git submodule update --init --recursive` for live upstream inventory coverage"
        );
        None
    }
}

fn collect_swift_files(path: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("directory should be readable") {
        let entry = entry.expect("directory entry should be readable");
        let path = entry.path();
        if path.file_name().and_then(|name| name.to_str()) == Some("Fixtures") {
            continue;
        }
        if path.is_dir() {
            collect_swift_files(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("swift") {
            files.push(path);
        }
    }
}

fn upstream_test_methods(root: &Path) -> BTreeSet<String> {
    let mut swift_files = Vec::new();
    collect_swift_files(root, &mut swift_files);
    let mut tests = BTreeSet::new();
    for file in swift_files {
        let relative = file
            .strip_prefix(root)
            .expect("file should be under upstream tests")
            .to_string_lossy()
            .replace('\\', "/");
        let contents = fs::read_to_string(&file).expect("swift test should be readable");
        for line in contents.lines() {
            if let Some(rest) = line.strip_prefix("    func test") {
                let name = format!(
                    "test{}",
                    rest.split_once('(')
                        .map(|(name, _)| name)
                        .expect("test method should have parameters")
                );
                tests.insert(format!("{relative}:{name}"));
            }
        }
    }
    tests
}

fn rust_counterparts(upstream_test: &str) -> &'static [&'static str] {
    match upstream_test {
        test if test.starts_with("FixtureTests/") => &["tests/upstream_fixtures.rs"],
        test if test.starts_with("PerformanceTests/") => &["tests/performance_smoke_tests.rs"],
        test if test.starts_with("ProjectSpecTests/Dictionary+Extension_Tests.swift") => {
            &["src/spec.rs"]
        }
        test if test.starts_with("ProjectSpecTests/InvalidConfigsFormatTests.swift") => {
            &["tests/upstream_fixtures.rs"]
        }
        test if test.starts_with("ProjectSpecTests/ProjectSpecTests.swift") => &["src/spec.rs"],
        test if test.starts_with("ProjectSpecTests/SpecLoadingTests.swift") => {
            &["src/spec.rs", "tests/upstream_fixtures.rs"]
        }
        test if test.starts_with("XcodeGenCoreTests/") => &["tests/core_tests.rs"],
        test if test.starts_with("XcodeGenKitTests/BreakpointGeneratorTests.swift") => {
            &["tests/scheme_writer_tests.rs"]
        }
        test if test.starts_with("XcodeGenKitTests/CarthageDependencyResolverTests.swift") => {
            &["tests/pbx_generator_tests.rs"]
        }
        test if test.starts_with("XcodeGenKitTests/PBXProjGeneratorTests.swift") => {
            &["tests/pbx_generator_tests.rs"]
        }
        test if test.starts_with("XcodeGenKitTests/ProjectGeneratorTests.swift") => &[
            "src/spec.rs",
            "tests/pbx_generator_tests.rs",
            "tests/upstream_fixtures.rs",
        ],
        test if test.starts_with("XcodeGenKitTests/SchemeGeneratorTests.swift") => {
            &["tests/scheme_writer_tests.rs"]
        }
        test if test.starts_with("XcodeGenKitTests/SourceGeneratorTests.swift") => {
            &["tests/pbx_generator_tests.rs"]
        }
        _ => &[],
    }
}

#[test]
fn upstream_test_inventory_matches_current_xcodegen_checkout() {
    let Some(tests_dir) = available_upstream_tests_dir() else {
        return;
    };
    let actual = upstream_test_methods(&tests_dir);
    let expected = EXPECTED_UPSTREAM_TESTS
        .iter()
        .map(|test| (*test).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}

#[test]
fn every_upstream_test_method_has_a_ported_rust_counterpart() {
    let missing = EXPECTED_UPSTREAM_TESTS
        .iter()
        .copied()
        .filter(|test| rust_counterparts(test).is_empty())
        .collect::<Vec<_>>();
    assert!(missing.is_empty(), "missing Rust ports: {missing:#?}");
}
