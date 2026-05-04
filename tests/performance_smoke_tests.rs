use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use xgr::{ProjectWriter, SpecFile, SpecLoader};

fn upstream_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("upstream-xcodegen")
}

fn sample_project() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::TempDir::new().expect("temp dir should be created");
    let source_dir = temp.path().join("Sources");
    fs::create_dir_all(&source_dir).expect("source dir should be created");
    fs::write(source_dir.join("App.swift"), "import Foundation\n")
        .expect("source file should be written");
    let spec_path = temp.path().join("project.yml");
    fs::write(
        &spec_path,
        r#"
name: GeneratedPerformance
targets:
  App:
    type: application
    platform: iOS
    sources:
      - Sources
"#,
    )
    .expect("spec should be written");
    (temp, spec_path)
}

fn load_project(spec_path: impl AsRef<Path>) -> xgr::Project {
    SpecLoader::load_project(spec_path, None, HashMap::new()).expect("project should load")
}

#[test]
fn generated_performance_loading_case_is_ported_as_smoke_test() {
    let (_temp, spec_path) = sample_project();
    let spec = SpecFile::load(&spec_path).expect("spec file should load");
    let dictionary = spec.resolved_dictionary();
    assert_eq!(
        dictionary.get("name").and_then(|value| value.as_str()),
        Some("GeneratedPerformance")
    );
}

#[test]
fn generated_performance_generation_case_is_ported_as_smoke_test() {
    let (_temp, spec_path) = sample_project();
    let project = load_project(spec_path);
    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXProject;"));
    assert!(generated.pbxproj.contains("App.swift in Sources"));
}

#[test]
fn generated_performance_writing_case_is_ported_as_smoke_test() {
    let (_temp, spec_path) = sample_project();
    let project = load_project(spec_path);
    let output = tempfile::TempDir::new().expect("output dir should be created");
    let generated =
        ProjectWriter::write(&project, Some(&output.path().join("Generated.xcodeproj")))
            .expect("project should write");
    assert!(generated.project_path.join("project.pbxproj").exists());
    assert!(generated
        .project_path
        .join("project.xcworkspace/contents.xcworkspacedata")
        .exists());
}

#[test]
fn fixture_performance_decoding_case_is_ported_as_smoke_test() {
    let spec_path = upstream_root().join("Tests/Fixtures/TestProject/project.yml");
    let project = load_project(spec_path);
    assert!(project.targets.contains_key("App_iOS"));
}

#[test]
fn fixture_performance_cache_file_generation_case_is_ported_as_smoke_test() {
    let spec_path = upstream_root().join("Tests/Fixtures/TestProject/project.yml");
    let spec = SpecFile::load(&spec_path).expect("spec should load");
    let dictionary = spec.resolved_dictionary();
    let cache_payload =
        serde_json::to_string(&dictionary).expect("resolved dictionary should serialize");
    assert!(cache_payload.contains("App_iOS"));
}

#[test]
fn fixture_performance_generation_case_is_ported_as_smoke_test() {
    let spec_path = upstream_root().join("Tests/Fixtures/TestProject/project.yml");
    let project = load_project(spec_path);
    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXProject;"));
    assert!(generated.pbxproj.contains("App_iOS"));
}

#[test]
fn fixture_performance_writing_case_is_ported_as_smoke_test() {
    let spec_path = upstream_root().join("Tests/Fixtures/TestProject/project.yml");
    let project = load_project(spec_path);
    let output = tempfile::TempDir::new().expect("output dir should be created");
    let generated = ProjectWriter::write(&project, Some(&output.path().join("Project.xcodeproj")))
        .expect("fixture project should write");
    assert!(generated.project_path.join("project.pbxproj").exists());
    assert!(generated
        .project_path
        .join("xcshareddata/xcschemes/App_iOS Test.xcscheme")
        .exists());
}
