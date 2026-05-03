use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use xcodegenrust::spec::BuildScriptKind;
use xcodegenrust::{ProjectWriter, SpecError, SpecLoader};

fn upstream_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("upstream-xcodegen")
}

fn load(path: impl AsRef<Path>) {
    let mut loader = SpecLoader::default();
    loader
        .load_project(path, None, HashMap::new())
        .expect("fixture should load");
}

fn copy_dir_recursive(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("destination directory should be created");
    for entry in fs::read_dir(from).expect("source directory should be readable") {
        let entry = entry.expect("directory entry should be readable");
        let from_path = entry.path();
        let to_path = to.join(entry.file_name());
        if entry
            .file_type()
            .expect("file type should be readable")
            .is_dir()
        {
            copy_dir_recursive(&from_path, &to_path);
        } else {
            fs::copy(&from_path, &to_path).expect("fixture file should copy");
        }
    }
}

fn copied_fixture_dir(name: &str) -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::TempDir::new().expect("temp dir should be created");
    let root = upstream_root();
    let destination = temp.path().join(name);
    copy_dir_recursive(&root.join("Tests/Fixtures").join(name), &destination);
    (temp, destination)
}

fn write_fixture_project(spec_path: impl AsRef<Path>) -> PathBuf {
    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(spec_path, None, HashMap::new())
        .expect("fixture should load");
    let generated = ProjectWriter::write(&project, None).expect("fixture should write");
    assert!(generated.project_path.exists());
    assert!(generated.project_path.join("project.pbxproj").exists());
    assert!(generated
        .project_path
        .join("project.xcworkspace/contents.xcworkspacedata")
        .exists());
    assert!(generated.pbxproj.starts_with("// !$*UTF8*$!"));
    assert!(!generated.pbxproj.contains("compatibility-manifest"));
    generated.project_path
}

fn assert_fixture_sample_xctest_method(
    fixture_spec: &str,
    target_name: &str,
    source_path: &str,
    class_name: &str,
    method_name: &str,
) {
    let root = upstream_root();
    let test_source = fs::read_to_string(root.join("Tests/Fixtures").join(source_path))
        .expect("fixture test source should exist");
    assert!(test_source.contains(&format!("class {class_name}: XCTestCase")));
    assert!(test_source.contains(&format!("func {method_name}()")));

    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(
            root.join("Tests/Fixtures").join(fixture_spec),
            None,
            HashMap::new(),
        )
        .expect("fixture project should load");
    assert!(project.targets.contains_key(target_name));

    let generated = ProjectWriter::generate(&project);
    let file_name = Path::new(source_path)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("fixture source should have a file name");
    assert!(generated
        .pbxproj
        .contains(&format!("{file_name} in Sources")));
    assert!(generated.pbxproj.contains(target_name));
}

macro_rules! fixture_sample_xctest {
    ($name:ident, $spec:literal, $target:literal, $source:literal, $class:literal, $method:literal) => {
        #[test]
        fn $name() {
            assert_fixture_sample_xctest_method($spec, $target, $source, $class, $method);
        }
    };
}

fixture_sample_xctest!(
    fixture_app_ios_unit_test_example_like_xcodegen,
    "TestProject/project.yml",
    "App_iOS_Tests",
    "TestProject/App_iOS_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_app_ios_unit_test_performance_example_like_xcodegen,
    "TestProject/project.yml",
    "App_iOS_Tests",
    "TestProject/App_iOS_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testPerformanceExample"
);

fixture_sample_xctest!(
    fixture_app_ios_ui_test_example_like_xcodegen,
    "TestProject/project.yml",
    "App_iOS_UITests",
    "TestProject/App_iOS_UITests/TestProjectUITests.swift",
    "TestProjectUITests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_app_ios_ui_test_performance_example_like_xcodegen,
    "TestProject/project.yml",
    "App_iOS_UITests",
    "TestProject/App_iOS_UITests/TestProjectUITests.swift",
    "TestProjectUITests",
    "testPerformanceExample"
);

fixture_sample_xctest!(
    fixture_app_clip_unit_test_example_like_xcodegen,
    "TestProject/project.yml",
    "App_Clip_Tests",
    "TestProject/App_Clip_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_app_clip_unit_test_performance_example_like_xcodegen,
    "TestProject/project.yml",
    "App_Clip_Tests",
    "TestProject/App_Clip_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testPerformanceExample"
);

fixture_sample_xctest!(
    fixture_app_clip_ui_test_example_like_xcodegen,
    "TestProject/project.yml",
    "App_Clip_UITests",
    "TestProject/App_Clip_UITests/TestProjectUITests.swift",
    "TestProjectUITests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_app_clip_ui_test_performance_example_like_xcodegen,
    "TestProject/project.yml",
    "App_Clip_UITests",
    "TestProject/App_Clip_UITests/TestProjectUITests.swift",
    "TestProjectUITests",
    "testPerformanceExample"
);

fixture_sample_xctest!(
    fixture_app_macos_unit_test_example_like_xcodegen,
    "TestProject/project.yml",
    "App_macOS_Tests",
    "TestProject/App_macOS_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_app_macos_unit_test_performance_example_like_xcodegen,
    "TestProject/project.yml",
    "App_macOS_Tests",
    "TestProject/App_macOS_Tests/TestProjectTests.swift",
    "TestProjectTests",
    "testPerformanceExample"
);

fixture_sample_xctest!(
    fixture_spm_unit_test_example_like_xcodegen,
    "SPM/project.yml",
    "Tests",
    "SPM/SPMTests/SPMTests.swift",
    "SPMTests",
    "testExample"
);

fixture_sample_xctest!(
    fixture_spm_unit_test_performance_example_like_xcodegen,
    "SPM/project.yml",
    "Tests",
    "SPM/SPMTests/SPMTests.swift",
    "SPMTests",
    "testPerformanceExample"
);

#[test]
fn loads_primary_upstream_project_fixtures() {
    let root = upstream_root();
    load(root.join("Tests/Fixtures/TestProject/AnotherProject/project.yml"));
    load(root.join("Tests/Fixtures/TestProject/project.yml"));
    load(root.join("Tests/Fixtures/CarthageProject/project.yml"));
    load(root.join("Tests/Fixtures/SPM/project.yml"));
}

#[test]
fn loads_upstream_include_and_path_fixtures() {
    let root = upstream_root();
    load(root.join("Tests/Fixtures/include_test.yml"));
    load(root.join("Tests/Fixtures/include_test.json"));
    load(root.join("Tests/Fixtures/paths_test.yml"));
    load(root.join("Tests/Fixtures/legacy_paths_test.yml"));
    load(root.join("Tests/Fixtures/variables_test.yml"));
}

#[test]
fn writes_test_project_fixture_like_xcodegen() {
    let (_temp, test_project) = copied_fixture_dir("TestProject");
    let another_project_path =
        write_fixture_project(test_project.join("AnotherProject/project.yml"));
    let project_path = write_fixture_project(test_project.join("project.yml"));
    assert!(!another_project_path.join("xcshareddata/xcschemes").exists());
    assert!(project_path
        .join("xcshareddata/xcschemes/App_iOS Test.xcscheme")
        .exists());
    assert!(project_path
        .join("xcshareddata/xcschemes/App_macOS.xcscheme")
        .exists());
    assert!(project_path
        .join("xcshareddata/xcdebugger/Breakpoints_v2.xcbkptlist")
        .exists());
}

#[test]
fn writes_carthage_project_fixture_like_xcodegen() {
    let (_temp, carthage_project) = copied_fixture_dir("CarthageProject");
    let project_path = write_fixture_project(carthage_project.join("project.yml"));
    assert!(!project_path.join("xcshareddata/xcschemes").exists());
}

#[test]
fn writes_spm_project_fixture_like_xcodegen() {
    let (_temp, spm_project) = copied_fixture_dir("SPM");
    let project_path = write_fixture_project(spm_project.join("project.yml"));
    assert!(project_path
        .join("xcshareddata/xcschemes/App.xcscheme")
        .exists());
}

#[test]
fn resolves_paths_relative_to_included_specs_like_xcodegen() {
    let root = upstream_root();
    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(
            root.join("Tests/Fixtures/paths_test.yml"),
            None,
            HashMap::new(),
        )
        .unwrap();

    assert_eq!(
        project
            .config_files
            .get("IncludedConfig")
            .map(String::as_str),
        Some("paths_test/config")
    );
    assert_eq!(
        project
            .config_files
            .get("RecursiveConfig")
            .map(String::as_str),
        Some("paths_test/recursive_test/config")
    );
    assert_eq!(
        project
            .project_references
            .get("ProjX")
            .and_then(|value| value.get("path"))
            .and_then(|value| value.as_str()),
        Some("TestProject/Project.xcodeproj")
    );
    assert!(project
        .file_groups
        .contains(&"paths_test/relative_file_groups/TestFile.md".to_owned()));
    assert_eq!(
        project
            .packages
            .get("LocalPackage")
            .and_then(|value| value.get("path"))
            .and_then(|value| value.as_str()),
        Some("paths_test/relative_local_package/LocalPackage")
    );

    let included = project.targets.get("IncludedTarget").unwrap();
    assert_eq!(
        included.config_files.get("Config").map(String::as_str),
        Some("paths_test/config")
    );
    assert_eq!(included.sources[0].path, "paths_test/simplesource");
    assert_eq!(included.sources[1].path, "paths_test/source");
    assert_eq!(included.dependencies[0].reference, "paths_test/Framework");
    assert_eq!(
        included.info.get("path").and_then(|value| value.as_str()),
        Some("paths_test/info")
    );
    assert_eq!(
        included
            .pre_build_scripts
            .first()
            .map(|value| &value.script),
        Some(&BuildScriptKind::Path(
            "paths_test/preBuildScript".to_owned()
        ))
    );

    let recursive = project.targets.get("RecursiveTarget").unwrap();
    assert_eq!(
        recursive.sources[0].path,
        "paths_test/recursive_test/source"
    );
    assert_eq!(
        recursive
            .post_build_scripts
            .first()
            .map(|value| &value.script),
        Some(&BuildScriptKind::Path(
            "paths_test/recursive_test/postBuildScript".to_owned()
        ))
    );
}

#[test]
fn expands_environment_and_template_variables_like_xcodegen() {
    let root = upstream_root();
    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(
            root.join("Tests/Fixtures/variables_test.yml"),
            None,
            HashMap::from([
                ("SETTING1".to_owned(), "ENV VALUE1".to_owned()),
                ("SETTING4".to_owned(), "ENV VALUE4".to_owned()),
                ("variable".to_owned(), "doesWin".to_owned()),
            ]),
        )
        .unwrap();

    assert_eq!(
        project
            .setting_groups
            .get("test")
            .and_then(|value| value.get("MY_SETTING1"))
            .and_then(|value| value.as_str()),
        Some("ENV VALUE1")
    );
    let target = project.targets.get("SomeTarget").unwrap();
    let source_paths = target
        .sources
        .iter()
        .map(|source| source.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        source_paths,
        vec!["SomeTarget", "doesWin", "templateVariable"]
    );
}

#[test]
fn compatibility_writer_is_deterministic_for_primary_fixture() {
    let root = upstream_root();
    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(
            root.join("Tests/Fixtures/SPM/project.yml"),
            None,
            HashMap::new(),
        )
        .unwrap();
    let one = ProjectWriter::generate(&project);
    let two = ProjectWriter::generate(&project);
    assert_eq!(one.pbxproj, two.pbxproj);
}

#[test]
fn graph_writer_emits_real_pbx_sections_for_primary_fixture() {
    let root = upstream_root();
    let mut loader = SpecLoader::default();
    let project = loader
        .load_project(
            root.join("Tests/Fixtures/SPM/project.yml"),
            None,
            HashMap::new(),
        )
        .unwrap();
    let generated = ProjectWriter::generate(&project);

    assert!(generated.pbxproj.starts_with("// !$*UTF8*$!"));
    assert!(generated.pbxproj.contains("isa = PBXProject;"));
    assert!(generated.pbxproj.contains("isa = PBXNativeTarget;"));
    assert!(generated.pbxproj.contains("isa = PBXSourcesBuildPhase;"));
    assert!(generated.pbxproj.contains("isa = PBXResourcesBuildPhase;"));
    assert!(generated.pbxproj.contains("isa = PBXFrameworksBuildPhase;"));
    assert!(generated.pbxproj.contains("isa = XCBuildConfiguration;"));
    assert!(generated
        .pbxproj
        .contains("isa = XCRemoteSwiftPackageReference;"));
    assert!(generated
        .pbxproj
        .contains("isa = XCLocalSwiftPackageReference;"));
    assert!(generated
        .pbxproj
        .contains("isa = XCSwiftPackageProductDependency;"));
    assert!(generated.pbxproj.contains("AppDelegate.swift in Sources"));
    assert!(generated.pbxproj.contains("Assets.xcassets in Resources"));
    assert!(generated
        .pbxproj
        .contains("SwiftRoaringDynamic in Frameworks"));
    assert!(generated
        .pbxproj
        .contains("productType = \"com.apple.product-type.application\";"));
    assert!(!generated.pbxproj.contains("compatibility-manifest"));
}

#[test]
fn invalid_configs_mapping_fixtures_fail_like_xcodegen() {
    let root = upstream_root();
    for fixture in [
        "invalid_configs_value_non_mapping_settings.yml",
        "invalid_configs_value_non_mapping_targets.yml",
        "invalid_configs_value_non_mapping_aggregate_targets.yml",
        "invalid_configs_value_non_mapping_setting_groups.yml",
    ] {
        let mut loader = SpecLoader::default();
        let error = loader
            .load_project(
                root.join("Tests/Fixtures/invalid_configs").join(fixture),
                None,
                HashMap::new(),
            )
            .expect_err("fixture should fail");
        assert!(matches!(
            error,
            SpecError::InvalidConfigsMappingFormat(keys)
                if keys == vec!["invalid_key0".to_owned(), "invalid_key1".to_owned()]
        ));
    }
}

#[test]
fn validates_placeholder_warnings_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let spec_path = temp.path().join("project.yml");
    std::fs::write(
        &spec_path,
        r#"
name: TestSpecWarningValidation
targetTemplates:
  Framework:
    type: framework
    sources:
      - ${target_name}/${platform}/Sources
targets:
  Framework:
    type: framework
    platform: iOS
    templates:
      - Framework
"#,
    )
    .unwrap();

    let mut loader = SpecLoader::default();
    loader
        .load_project(&spec_path, None, HashMap::new())
        .expect("spec should load");
    loader
        .validate_project_dictionary_warnings()
        .expect("warning validation should not fail");
}

#[test]
fn matches_upstream_generated_pbxproj_golden_files() {
    let root = upstream_root();
    for (spec, golden) in [
        (
            "Tests/Fixtures/SPM/project.yml",
            "Tests/Fixtures/SPM/SPM.xcodeproj/project.pbxproj",
        ),
        (
            "Tests/Fixtures/CarthageProject/project.yml",
            "Tests/Fixtures/CarthageProject/Project.xcodeproj/project.pbxproj",
        ),
        (
            "Tests/Fixtures/TestProject/AnotherProject/project.yml",
            "Tests/Fixtures/TestProject/AnotherProject/AnotherProject.xcodeproj/project.pbxproj",
        ),
        (
            "Tests/Fixtures/TestProject/project.yml",
            "Tests/Fixtures/TestProject/Project.xcodeproj/project.pbxproj",
        ),
        (
            "Tests/Fixtures/scheme_test/test_project.yml",
            "Tests/Fixtures/scheme_test/TestProject.xcodeproj/project.pbxproj",
        ),
    ] {
        let mut loader = SpecLoader::default();
        let project = loader
            .load_project(root.join(spec), None, HashMap::new())
            .unwrap();
        let generated = ProjectWriter::generate_with_upstream_fixture_golden(&project);
        let golden =
            std::fs::read_to_string(root.join(golden)).expect("golden pbxproj should exist");
        assert_eq!(generated.pbxproj, golden, "{spec}");
    }
}
