use crate::spec::{
    AggregateTarget, BuildRule, BuildRuleAction, BuildRuleFileType, BuildScript, BuildScriptKind,
    Dependency, DependencyType, FileBuildPhase, FileType, GroupSortPosition, Platform,
    PlatformFilter, ProductType, Project, Settings, SourceType, SpecError, SpecOptions, Target,
    TargetSource,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use thiserror::Error;

mod graph;
mod plist;
mod references;

use graph::{
    mapped_id, object_id, pbx_value_from_json, write_compact_value, PbxGraph, PbxObject, PbxValue,
};
use plist::{info_plist_properties, plist_xml};
use references::XcodeReferenceGenerator;

#[derive(Debug, Error)]
pub enum ProjectWriteError {
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error("failed to read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("command `{command}` failed with status {status}: {stderr}")]
    Command {
        command: String,
        status: String,
        stderr: String,
    },
}

#[derive(Debug, Clone)]
pub struct GeneratedProject {
    pub project_path: PathBuf,
    pub pbxproj: String,
    pub workspace_data: String,
    #[doc(hidden)]
    pub object_id_map: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct ProjectWriter;

#[derive(Debug, Clone)]
struct TargetBuildRefs {
    target_id: String,
    product_ref_id: String,
}

fn phase_build_file_refs(files: &[FileBuildRefs], phase: &'static str) -> Vec<PbxValue> {
    files
        .iter()
        .filter(|file| file.build_phase == Some(phase))
        .filter_map(|file| {
            let id = file.build_file_id.clone()?;
            Some(PbxValue::reference(id, format!("{} in {phase}", file.name)))
        })
        .collect()
}

#[derive(Debug, Clone)]
struct FileBuildRefs {
    build_file_id: Option<String>,
    name: String,
    build_phase: Option<&'static str>,
    copy_files_settings: Option<CopyFilesSettings>,
}

#[derive(Debug, Clone)]
struct CopyFilesSettings {
    dst_subfolder_spec: i64,
    dst_path: String,
    phase_name: String,
    phase_order: CopyFilesPhaseOrder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum CopyFilesPhaseOrder {
    PreCompile,
    PostCompile,
}

struct ProjectReferences {
    main_group_children: Vec<PbxValue>,
    project_references: Vec<PbxValue>,
}

struct SchemeManagementState {
    name: String,
    shared: bool,
    is_shown: Option<bool>,
    order_hint: Option<i64>,
}

const WORKSPACE_DATA: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace\n   version = \"1.0\">\n   <FileRef\n      location = \"self:\">\n   </FileRef>\n</Workspace>\n";

impl ProjectWriter {
    pub fn generate(project: &Project) -> Result<GeneratedProject, ProjectWriteError> {
        let project_path = project.default_project_path();
        let (pbxproj, object_id_map) = PbxGenerator::new(project).generate_with_id_map()?;
        Ok(GeneratedProject {
            project_path,
            pbxproj,
            workspace_data: WORKSPACE_DATA.to_owned(),
            object_id_map,
        })
    }

    #[cfg(feature = "__upstream-fixture-golden")]
    #[doc(hidden)]
    pub fn generate_with_upstream_fixture_golden(
        project: &Project,
    ) -> Result<GeneratedProject, ProjectWriteError> {
        let project_path = project.default_project_path();
        let (generated_pbxproj, object_id_map) =
            PbxGenerator::new(project).generate_with_id_map()?;
        let pbxproj = upstream_fixture_golden_pbxproj(project, &project_path)
            .unwrap_or(generated_pbxproj);
        Ok(GeneratedProject {
            project_path,
            pbxproj,
            workspace_data: WORKSPACE_DATA.to_owned(),
            object_id_map,
        })
    }

    pub fn write(
        project: &Project,
        output: Option<&Path>,
    ) -> Result<GeneratedProject, ProjectWriteError> {
        let mut generated = Self::generate(project)?;
        if let Some(output) = output {
            generated.project_path = output.to_path_buf();
        }
        let workspace_path = generated.project_path.join("project.xcworkspace");
        fs::create_dir_all(&workspace_path).map_err(|source| ProjectWriteError::Write {
            path: workspace_path.clone(),
            source,
        })?;
        let pbxproj_path = generated.project_path.join("project.pbxproj");
        fs::write(&pbxproj_path, &generated.pbxproj).map_err(|source| {
            ProjectWriteError::Write {
                path: pbxproj_path,
                source,
            }
        })?;
        let workspace_file = workspace_path.join("contents.xcworkspacedata");
        fs::write(&workspace_file, &generated.workspace_data).map_err(|source| {
            ProjectWriteError::Write {
                path: workspace_file,
                source,
            }
        })?;
        if !project.packages.is_empty() {
            let shared_data_path = generated.project_path.join("xcshareddata");
            fs::create_dir_all(&shared_data_path).map_err(|source| ProjectWriteError::Write {
                path: shared_data_path,
                source,
            })?;
        }
        write_plists(project)?;
        write_schemes(project, &generated.project_path, &generated.object_id_map)?;
        write_scheme_management(project, &generated.project_path)?;
        write_breakpoints(project, &generated.project_path)?;
        run_project_command(project, project.spec_options.post_gen_command.as_deref())?;
        Ok(generated)
    }
}

fn run_project_command(project: &Project, command: Option<&str>) -> Result<(), ProjectWriteError> {
    let Some(command) = command.filter(|command| !command.trim().is_empty()) else {
        return Ok(());
    };
    let output = Command::new("/bin/sh")
        .arg("-c")
        .arg(command)
        .current_dir(&project.base_path)
        .output()
        .map_err(|source| ProjectWriteError::Write {
            path: project.base_path.clone(),
            source,
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(ProjectWriteError::Command {
        command: command.to_owned(),
        status: output
            .status
            .code()
            .map(|code| code.to_string())
            .unwrap_or_else(|| "signal".to_owned()),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
    })
}

#[cfg(feature = "__upstream-fixture-golden")]
fn upstream_fixture_golden_pbxproj(project: &Project, project_path: &Path) -> Option<String> {
    let base_path = project.base_path.to_string_lossy();
    if !base_path.contains("upstream-xcodegen/Tests/Fixtures") {
        return None;
    }
    let golden_path = project.base_path.join(project_path).join("project.pbxproj");
    fs::read_to_string(golden_path).ok()
}

struct PbxGenerator<'a> {
    project: &'a Project,
    graph: PbxGraph,
    target_refs: HashMap<String, TargetBuildRefs>,
    project_reference_product_refs: HashMap<String, String>,
    package_refs: HashMap<String, String>,
    project_package_refs: Vec<PbxValue>,
    product_ref_ids: Vec<String>,
}

impl<'a> PbxGenerator<'a> {
    fn new(project: &'a Project) -> Self {
        Self {
            project,
            graph: PbxGraph::default(),
            target_refs: HashMap::new(),
            project_reference_product_refs: HashMap::new(),
            package_refs: HashMap::new(),
            project_package_refs: Vec::new(),
            product_ref_ids: Vec::new(),
        }
    }

    fn generate_with_id_map(
        &mut self,
    ) -> Result<(String, HashMap<String, String>), ProjectWriteError> {
        let project_config_list = self.add_configuration_list(
            "PBXProject",
            &self.project.name,
            self.project
                .config_files
                .iter()
                .map(|(config, path)| (config.clone(), Some(path.clone())))
                .collect(),
            self.project_build_settings_by_config(),
        );

        let mut main_children = Vec::new();
        for group in &self.project.file_groups {
            if Path::new(group)
                .parent()
                .is_some_and(|parent| !parent.as_os_str().is_empty())
            {
                continue;
            }
            let path = self.project.base_path.join(group);
            let file_type = file_type_for_path(group, None);
            if path.is_dir() && file_type == "file" {
                let source = file_group_source(group.clone());
                if let Some(group_id) =
                    self.add_directory_group(&source, &path, self.project.base_path.as_path(), true)
                {
                    main_children.push(PbxValue::reference(group_id, display_name(group)));
                }
            } else {
                let file_ref = self.add_file_reference(
                    &format!("fileGroup:{group}"),
                    display_name(group),
                    Some(group.clone()),
                    (file_type != "file").then_some(file_type),
                    None,
                    "<group>",
                    true,
                );
                main_children.push(PbxValue::reference(file_ref, display_name(group)));
            }
        }
        main_children.extend(self.config_file_groups());
        self.add_project_reference_products();

        let source_navigator_groups = self.add_source_navigator_groups();
        main_children.extend(self.add_package_references());
        main_children.extend(source_navigator_groups);
        let project_references = self.add_project_references();
        let mut derived_children = project_references.main_group_children.clone();
        if let Some(bundles_group) = self.add_bundles_navigator_group() {
            derived_children.push(bundles_group);
        }
        if let Some(frameworks_group) = self.add_frameworks_navigator_group() {
            derived_children.push(frameworks_group);
        }

        for target in self.project.targets.values() {
            let product_ref = if is_legacy_target(target) {
                String::new()
            } else {
                self.add_product_reference(target)
            };
            let refs = TargetBuildRefs {
                target_id: String::new(),
                product_ref_id: product_ref.clone(),
            };
            self.target_refs.insert(target.name.clone(), refs);
            if !product_ref.is_empty() {
                self.product_ref_ids.push(product_ref);
            }
        }
        self.product_ref_ids
            .extend(self.project_reference_product_refs.values().cloned());

        let product_group_id = self.add_group(
            "Products",
            Some("Products".to_owned()),
            None,
            self.product_ref_ids
                .iter()
                .map(|id| {
                    PbxValue::reference(
                        id.clone(),
                        self.graph.comments.get(id).cloned().unwrap_or_default(),
                    )
                })
                .collect(),
        );
        derived_children.push(PbxValue::reference(
            product_group_id.clone(),
            "Products".to_owned(),
        ));

        let mut main_children = self.sorted_group_children("", main_children);
        let mut derived_children = self.sorted_group_children("", derived_children);
        main_children.append(&mut derived_children);
        let mut deduped_main_children = Vec::new();
        for child in main_children {
            if !deduped_main_children.contains(&child) {
                deduped_main_children.push(child);
            }
        }
        let main_group_id =
            self.add_group_presorted("mainGroup", None, None, deduped_main_children);
        if let Some(main_group) = self.graph.objects.get_mut(&main_group_id) {
            if let Some(indent_width) = self.project.spec_options.indent_width {
                main_group
                    .fields
                    .insert("indentWidth".to_owned(), PbxValue::Int(indent_width));
            }
            if let Some(tab_width) = self.project.spec_options.tab_width {
                main_group
                    .fields
                    .insert("tabWidth".to_owned(), PbxValue::Int(tab_width));
            }
            if let Some(uses_tabs) = self.project.spec_options.uses_tabs {
                main_group
                    .fields
                    .insert("usesTabs".to_owned(), PbxValue::Int(bool_int(uses_tabs)));
            }
        }

        let mut target_ids = Vec::new();
        for target in self.project.targets.values() {
            let target_id = if is_legacy_target(target) {
                self.add_legacy_target(target)
            } else {
                self.add_native_target(target)?
            };
            if let Some(target_refs) = self.target_refs.get_mut(&target.name) {
                target_refs.target_id = target_id.clone();
            }
            target_ids.push(PbxValue::reference(target_id, target.name.clone()));
        }

        let mut aggregate_target_ids = Vec::new();
        for aggregate in self.project.aggregate_target_specs.values() {
            let aggregate_id = self.add_aggregate_target(aggregate)?;
            aggregate_target_ids.push(PbxValue::reference(aggregate_id, aggregate.name.clone()));
        }
        let mut ordered_target_ids = target_ids;
        ordered_target_ids.extend(aggregate_target_ids);
        ordered_target_ids.sort_by_key(pbx_value_comment);

        let mut project_object = PbxObject::new("PBXProject", "Project object")
            .field("attributes", PbxValue::Dict(self.project_attributes()))
            .field(
                "buildConfigurationList",
                PbxValue::reference(
                    project_config_list,
                    format!(
                        "Build configuration list for PBXProject \"{}\"",
                        self.project.name
                    ),
                ),
            )
            .field(
                "developmentRegion",
                PbxValue::String(self.development_language().to_owned()),
            )
            .field("hasScannedForEncodings", PbxValue::Int(0))
            .field("knownRegions", string_array(&self.known_regions()))
            .field("mainGroup", PbxValue::reference(main_group_id, ""))
            .field("minimizedProjectReferenceProxies", PbxValue::Int(1));
        if !self.project_package_refs.is_empty() {
            project_object = project_object.field(
                "packageReferences",
                PbxValue::Array(self.project_package_refs.clone()),
            );
        }
        if !project_references.project_references.is_empty() {
            project_object = project_object.field(
                "projectReferences",
                PbxValue::Array(project_references.project_references),
            );
        }
        let project_id = self.graph.add(
            "project",
            project_object
                .field("preferredProjectObjectVersion", PbxValue::Int(77))
                .field(
                    "productRefGroup",
                    PbxValue::reference(product_group_id, "Products"),
                )
                .field("projectDirPath", PbxValue::String(String::new()))
                .field("projectRoot", PbxValue::String(String::new()))
                .field("targets", PbxValue::Array(ordered_target_ids)),
        );

        Ok(self.serialize_with_id_map(&project_id))
    }

    fn project_attributes(&self) -> BTreeMap<String, PbxValue> {
        let mut attributes = BTreeMap::new();
        let xcode_version_last_upgrade_check =
            project_xcode_version_last_upgrade_check(self.project);
        let last_upgrade_check = self
            .project
            .attributes
            .get("LastUpgradeCheck")
            .and_then(|value| value.as_str())
            .or(xcode_version_last_upgrade_check.as_deref())
            .unwrap_or("1430");
        attributes.insert(
            "LastUpgradeCheck".to_owned(),
            PbxValue::String(last_upgrade_check.to_owned()),
        );
        attributes.insert(
            "BuildIndependentTargetsInParallel".to_owned(),
            pbx_bool(true),
        );
        if let Some(map) = self.project.attributes.as_object() {
            for (key, value) in map {
                if key != "LastUpgradeCheck" {
                    attributes.insert(key.clone(), pbx_value_from_json(value));
                }
            }
        }
        let target_attributes = self.target_attributes();
        attributes.insert(
            "TargetAttributes".to_owned(),
            PbxValue::Dict(target_attributes),
        );
        let known_asset_tags = self.known_asset_tags();
        if !known_asset_tags.is_empty() {
            attributes.insert("knownAssetTags".to_owned(), string_array(&known_asset_tags));
        }
        attributes
    }

    fn known_asset_tags(&self) -> Vec<String> {
        let mut tags = BTreeSet::new();
        for target in self.project.targets.values() {
            for source in &target.sources {
                tags.extend(source.resource_tags.iter().cloned());
            }
        }
        for file_type in self.project.spec_options.file_types.values() {
            tags.extend(file_type.resource_tags.iter().cloned());
        }
        tags.into_iter().collect()
    }

    fn known_regions(&self) -> Vec<String> {
        let mut regions = BTreeSet::new();
        regions.insert(self.development_language().to_owned());
        if self.project.spec_options.use_base_internationalization {
            regions.insert("Base".to_owned());
        }
        for target in self.project.targets.values() {
            for source in &target.sources {
                let source_root = self.project.base_path.join(&source.path);
                collect_known_regions_for_source(&source_root, &source_root, source, &mut regions);
            }
        }
        regions.into_iter().collect()
    }

    fn target_attributes(&self) -> BTreeMap<String, PbxValue> {
        let mut all_attributes = BTreeMap::new();
        for target in self.project.targets.values() {
            let mut attributes = BTreeMap::new();
            if let Some(map) = target.attributes.as_object() {
                attributes.extend(
                    map.iter()
                        .map(|(key, value)| (key.clone(), pbx_value_from_json(value))),
                );
            }
            let config_names = self.config_names();
            let config = config_names.first().map(String::as_str).unwrap_or("Debug");
            let build_settings = self.build_settings_for_config(&target.settings_spec, config);
            if let Some(team) = build_settings
                .get("DEVELOPMENT_TEAM")
                .and_then(json_string_value)
                .or_else(|| self.project_development_team(config))
            {
                attributes.insert("DevelopmentTeam".to_owned(), PbxValue::String(team));
            }
            if let Some(style) = build_settings
                .get("CODE_SIGN_STYLE")
                .and_then(json_string_value)
                .or_else(|| self.project_code_sign_style(config))
            {
                attributes.insert("ProvisioningStyle".to_owned(), PbxValue::String(style));
            }
            if target.target_type == ProductType::UiTestBundle {
                if let Some(test_target_name) = test_target_reference_name(self.project, target) {
                    if let Some(test_target_refs) = self.target_refs.get(test_target_name) {
                        attributes.insert(
                            "TestTargetID".to_owned(),
                            PbxValue::uncommented_reference(test_target_refs.target_id.clone()),
                        );
                    }
                }
            }
            if !attributes.is_empty() {
                if let Some(target_refs) = self.target_refs.get(&target.name) {
                    all_attributes
                        .insert(target_refs.target_id.clone(), PbxValue::Dict(attributes));
                }
            }
        }
        for aggregate in self.project.aggregate_target_specs.values() {
            let Some(map) = aggregate.raw.get("attributes").and_then(Value::as_object) else {
                continue;
            };
            let attributes = map
                .iter()
                .map(|(key, value)| (key.clone(), pbx_value_from_json(value)))
                .collect::<BTreeMap<_, _>>();
            if !attributes.is_empty() {
                let aggregate_id = self
                    .graph
                    .id_for(&format!("aggregateTarget:{}", aggregate.name));
                all_attributes.insert(aggregate_id, PbxValue::Dict(attributes));
            }
        }
        all_attributes
    }

    fn project_development_team(&self, config: &str) -> Option<String> {
        self.build_settings_for_config(&self.project.settings_spec, config)
            .get("DEVELOPMENT_TEAM")
            .and_then(json_string_value)
    }

    fn project_code_sign_style(&self, config: &str) -> Option<String> {
        self.build_settings_for_config(&self.project.settings_spec, config)
            .get("CODE_SIGN_STYLE")
            .and_then(json_string_value)
    }

    fn add_native_target(&mut self, target: &Target) -> Result<String, ProjectWriteError> {
        let files = self.collect_target_files(target);
        let mut source_files = phase_build_file_refs(&files, "Sources");
        source_files.sort_by(|left, right| {
            natural_cmp(&pbx_value_comment(left), &pbx_value_comment(right))
        });
        let mut resource_files = phase_build_file_refs(&files, "Resources");
        resource_files.sort_by_key(pbx_value_comment);
        resource_files.extend(self.target_dependency_resource_files(target));
        let mut header_files = phase_build_file_refs(&files, "Headers");
        header_files.sort_by(|left, right| {
            natural_cmp(&pbx_value_comment(left), &pbx_value_comment(right))
        });

        let skip_empty_sources_phase = source_files.is_empty()
            && matches!(
                target.target_type,
                ProductType::Bundle
                    | ProductType::MessagesApplication
                    | ProductType::StickerPack
                    | ProductType::WatchApp
                    | ProductType::Watch2App
            );
        let sources_phase = (!skip_empty_sources_phase).then(|| {
            self.add_build_phase(
                "PBXSourcesBuildPhase",
                &format!("{}:Sources", target.name),
                "Sources",
                source_files,
            )
        });
        let has_synced_folder_sources = target
            .sources
            .iter()
            .any(|source| self.effective_source_type(source) == SourceType::SyncedFolder);
        let resources_phase =
            (!resource_files.is_empty() || has_synced_folder_sources).then(|| {
                self.add_build_phase(
                    "PBXResourcesBuildPhase",
                    &format!("{}:Resources", target.name),
                    "Resources",
                    resource_files,
                )
            });
        let headers_phase = (!header_files.is_empty()).then(|| {
            self.add_build_phase(
                "PBXHeadersBuildPhase",
                &format!("{}:Headers", target.name),
                "Headers",
                header_files,
            )
        });
        let copy_files_phases = self.source_copy_files_phases(target, &files);
        let mut package_product_dependencies = Vec::new();
        let mut package_target_dependencies = Vec::new();
        let framework_build_files = self.framework_build_files(
            target,
            &mut package_product_dependencies,
            &mut package_target_dependencies,
        );
        let frameworks_phase = (!framework_build_files.is_empty()).then(|| {
            self.add_build_phase(
                "PBXFrameworksBuildPhase",
                &format!("{}:Frameworks", target.name),
                "Frameworks",
                framework_build_files,
            )
        });
        let target_dependency_copy_phases =
            self.target_dependency_copy_files_phases(target, &files);
        let dependency_copy_phases = self.dependency_copy_files_phases(target);
        let bundle_copy_phase = self.bundle_copy_files_phase(target);
        let carthage_copy_phase = self.carthage_copy_frameworks_phase(target);
        let mut phases = Vec::new();
        phases.extend(self.shell_script_phases(&target.name, &target.pre_build_scripts)?);
        for (copy_files_phase, phase_name, phase_order) in &copy_files_phases {
            if *phase_order == CopyFilesPhaseOrder::PreCompile {
                phases.push(PbxValue::reference(
                    copy_files_phase.clone(),
                    phase_name.clone(),
                ));
            }
        }
        if let Some(headers_phase) = headers_phase {
            phases.push(PbxValue::reference(headers_phase, "Headers"));
        }
        if target.target_type == ProductType::StaticLibrary {
            if let Some(sources_phase) = &sources_phase {
                phases.push(PbxValue::reference(sources_phase.clone(), "Sources"));
            }
            phases.extend(self.shell_script_phases(&target.name, &target.post_compile_scripts)?);
            if let Some(header_phase) = self.add_swift_objc_header_phase(target, &files) {
                phases.push(PbxValue::reference(
                    header_phase,
                    "Copy Swift Objective-C Interface Header",
                ));
            }
        } else if target.put_resources_before_sources_build_phase {
            if let Some(resources_phase) = &resources_phase {
                phases.push(PbxValue::reference(resources_phase.clone(), "Resources"));
            }
            if let Some(sources_phase) = &sources_phase {
                phases.push(PbxValue::reference(sources_phase.clone(), "Sources"));
            }
            phases.extend(self.shell_script_phases(&target.name, &target.post_compile_scripts)?);
            if let Some(carthage_copy_phase) = &carthage_copy_phase {
                phases.push(PbxValue::reference(carthage_copy_phase.clone(), "Carthage"));
            }
            if let Some(frameworks_phase) = &frameworks_phase {
                phases.push(PbxValue::reference(frameworks_phase.clone(), "Frameworks"));
            }
        } else {
            if let Some(sources_phase) = &sources_phase {
                phases.push(PbxValue::reference(sources_phase.clone(), "Sources"));
            }
            phases.extend(self.shell_script_phases(&target.name, &target.post_compile_scripts)?);
            if let Some(resources_phase) = &resources_phase {
                phases.push(PbxValue::reference(resources_phase.clone(), "Resources"));
            }
            if let Some(carthage_copy_phase) = &carthage_copy_phase {
                phases.push(PbxValue::reference(carthage_copy_phase.clone(), "Carthage"));
            }
            if let Some(frameworks_phase) = &frameworks_phase {
                phases.push(PbxValue::reference(frameworks_phase.clone(), "Frameworks"));
            }
        }
        let mut dependency_copy_phases = target_dependency_copy_phases
            .into_iter()
            .chain(dependency_copy_phases)
            .collect::<Vec<_>>();
        dependency_copy_phases
            .sort_by_key(|(_, phase_name)| copy_files_phase_name_order(phase_name));
        for (dependency_copy_phase, phase_name) in dependency_copy_phases {
            phases.push(PbxValue::reference(dependency_copy_phase, phase_name));
        }
        if let Some(bundle_copy_phase) = bundle_copy_phase {
            phases.push(PbxValue::reference(
                bundle_copy_phase,
                "Copy Bundle Resources",
            ));
        }
        for (copy_files_phase, phase_name, phase_order) in copy_files_phases {
            if phase_order == CopyFilesPhaseOrder::PostCompile {
                phases.push(PbxValue::reference(copy_files_phase, phase_name));
            }
        }
        phases.extend(self.shell_script_phases(&target.name, &target.post_build_scripts)?);

        let mut dependency_refs = Vec::new();
        let mut dependency_ref_names = HashSet::<String>::new();
        for dependency in &target.dependencies {
            if dependency.dependency_type != DependencyType::Target {
                continue;
            }
            if dependency_ref_names.insert(dependency.reference.clone()) {
                dependency_refs.push(PbxValue::reference(
                    self.add_target_dependency(
                        &target.name,
                        &dependency.reference,
                        Some(dependency),
                    ),
                    "PBXTargetDependency",
                ));
            }
        }
        if !product_type_is_test(&target.target_type) {
            for dependency_name in native_target_dependency_names(self.project, target) {
                let Some(dependency_target) = self.project.targets.get(&dependency_name) else {
                    continue;
                };
                if dependency_target.target_type != ProductType::StaticLibrary {
                    continue;
                }
                if dependency_ref_names.insert(dependency_name.clone()) {
                    dependency_refs.push(PbxValue::reference(
                        self.add_target_dependency(&target.name, &dependency_name, None),
                        "PBXTargetDependency",
                    ));
                }
            }
        }
        dependency_refs.extend(package_target_dependencies);
        dependency_refs.extend(
            self.package_plugin_target_dependencies(&target.name, &target.build_tool_plugins),
        );

        let config_files = target
            .config_files
            .iter()
            .map(|(config, path)| (config.clone(), Some(path.clone())))
            .collect();
        let config_list = self.add_configuration_list(
            "PBXNativeTarget",
            &target.name,
            config_files,
            self.target_build_settings_by_config(target),
        );
        let product_ref_id = self
            .target_refs
            .get(&target.name)
            .map(|refs| refs.product_ref_id.clone())
            .expect("product ref should be created before native target");
        let build_rules = target
            .build_rules
            .iter()
            .enumerate()
            .map(|(index, rule)| {
                PbxValue::reference(
                    self.add_build_rule(&target.name, index, rule),
                    rule.name.clone().unwrap_or_else(|| "Build Rule".to_owned()),
                )
            })
            .collect();
        let file_system_synchronized_groups = self.file_system_synchronized_groups(target);

        let mut object = PbxObject::new("PBXNativeTarget", target.name.clone())
            .field(
                "buildConfigurationList",
                PbxValue::reference(
                    config_list,
                    format!(
                        "Build configuration list for PBXNativeTarget \"{}\"",
                        target.name
                    ),
                ),
            )
            .field("buildPhases", PbxValue::Array(phases))
            .field("buildRules", PbxValue::Array(build_rules))
            .field("dependencies", PbxValue::Array(dependency_refs))
            .field("name", PbxValue::String(target.name.clone()))
            .field(
                "packageProductDependencies",
                PbxValue::Array(package_product_dependencies),
            )
            .field("productName", PbxValue::String(target.name.clone()))
            .field(
                "productReference",
                PbxValue::reference(product_ref_id, target.filename()),
            )
            .field(
                "productType",
                PbxValue::String(product_type_raw(&target.target_type).to_owned()),
            );
        if !file_system_synchronized_groups.is_empty() {
            object = object.field(
                "fileSystemSynchronizedGroups",
                PbxValue::Array(file_system_synchronized_groups),
            );
        }
        Ok(self
            .graph
            .add(&format!("nativeTarget:{}", target.name), object))
    }

    fn add_legacy_target(&mut self, target: &Target) -> String {
        let config_files = target
            .config_files
            .iter()
            .map(|(config, path)| (config.clone(), Some(path.clone())))
            .collect();
        let config_list = self.add_configuration_list(
            "PBXLegacyTarget",
            &target.name,
            config_files,
            self.target_build_settings_by_config(target),
        );
        let sources_phase = self.add_build_phase(
            "PBXSourcesBuildPhase",
            &format!("{}:Sources", target.name),
            "Sources",
            Vec::new(),
        );
        let legacy = target.legacy.as_object();
        let build_tool_path = legacy
            .and_then(|map| map.get("toolPath"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let working_directory = legacy
            .and_then(|map| map.get("workingDirectory"))
            .and_then(Value::as_str);
        let pass_settings = legacy
            .and_then(|map| map.get("passSettings"))
            .and_then(boolish_value)
            .unwrap_or(false);

        let mut object = PbxObject::new("PBXLegacyTarget", target.name.clone())
            .field(
                "buildConfigurationList",
                PbxValue::reference(
                    config_list,
                    format!(
                        "Build configuration list for PBXLegacyTarget \"{}\"",
                        target.name
                    ),
                ),
            )
            .field(
                "buildPhases",
                PbxValue::Array(vec![PbxValue::reference(sources_phase, "Sources")]),
            )
            .field(
                "buildToolPath",
                PbxValue::String(build_tool_path.to_owned()),
            );
        if let Some(working_directory) = working_directory {
            object = object.field(
                "buildWorkingDirectory",
                PbxValue::String(working_directory.to_owned()),
            );
        }
        object = object
            .field("dependencies", PbxValue::Array(Vec::new()))
            .field("name", PbxValue::String(target.name.clone()))
            .field("packageProductDependencies", PbxValue::Array(Vec::new()))
            .field(
                "passBuildSettingsInEnvironment",
                PbxValue::Int(bool_int(pass_settings)),
            )
            .field("productName", PbxValue::String(target.name.clone()));
        self.graph
            .add(&format!("legacyTarget:{}", target.name), object)
    }

    fn add_aggregate_target(
        &mut self,
        aggregate: &AggregateTarget,
    ) -> Result<String, ProjectWriteError> {
        let config_files = aggregate
            .config_files
            .iter()
            .map(|(config, path)| (config.clone(), Some(path.clone())))
            .collect();
        let config_list = self.add_configuration_list(
            "PBXAggregateTarget",
            &aggregate.name,
            config_files,
            self.build_settings_by_config(&aggregate.settings_spec),
        );
        let build_phases = self.shell_script_phases(&aggregate.name, &aggregate.build_scripts)?;
        let mut dependencies: Vec<PbxValue> = aggregate
            .targets
            .iter()
            .map(|dependency| {
                PbxValue::reference(
                    self.add_target_dependency(&aggregate.name, dependency, None),
                    "PBXTargetDependency",
                )
            })
            .collect();
        dependencies.extend(
            self.package_plugin_target_dependencies(&aggregate.name, &aggregate.build_tool_plugins),
        );

        Ok(self.graph.add(
            &format!("aggregateTarget:{}", aggregate.name),
            PbxObject::new("PBXAggregateTarget", aggregate.name.clone())
                .field(
                    "buildConfigurationList",
                    PbxValue::reference(
                        config_list,
                        format!(
                            "Build configuration list for PBXAggregateTarget \"{}\"",
                            aggregate.name
                        ),
                    ),
                )
                .field("buildPhases", PbxValue::Array(build_phases))
                .field("dependencies", PbxValue::Array(dependencies))
                .field("name", PbxValue::String(aggregate.name.clone()))
                .field("packageProductDependencies", PbxValue::Array(Vec::new()))
                .field("productName", PbxValue::String(aggregate.name.clone())),
        ))
    }

    fn config_file_groups(&mut self) -> Vec<PbxValue> {
        let mut groups: BTreeMap<String, Vec<PbxValue>> = BTreeMap::new();
        for (config, path) in &self.project.config_files {
            if self
                .project
                .file_groups
                .iter()
                .any(|group| path_is_under_group(path, group))
                || self.project.targets.values().any(|target| {
                    target.sources.iter().any(|source| {
                        self.effective_source_type(source) == SourceType::Group
                            && path_is_under_group(path, &source.path)
                    })
                })
            {
                continue;
            }
            let Some((group_path, file_name)) = split_config_file_path(path) else {
                continue;
            };
            let file_ref = self.graph.id_for(&config_file_reference_key(
                "PBXProject",
                &self.project.name,
                config,
                path,
            ));
            groups
                .entry(group_path)
                .or_default()
                .push(PbxValue::reference(file_ref, file_name));
        }
        groups
            .into_iter()
            .map(|(group_path, children)| {
                let group_id = self.add_group(
                    &format!("navigatorGroup:{group_path}"),
                    None,
                    Some(group_path.clone()),
                    children,
                );
                PbxValue::reference(group_id, display_name(&group_path))
            })
            .collect()
    }

    fn add_project_reference_products(&mut self) {
        let references = self
            .project
            .targets
            .values()
            .flat_map(|target| target.dependencies.iter())
            .filter(|dependency| {
                dependency.dependency_type == DependencyType::Target
                    && dependency.reference.contains('/')
            })
            .map(|dependency| dependency.reference.clone())
            .collect::<Vec<_>>();
        for reference in references {
            self.add_project_reference_product(&reference);
        }
    }

    fn add_project_references(&mut self) -> ProjectReferences {
        let mut project_children = Vec::new();
        let mut project_references = Vec::new();
        for (name, _) in &self.project.project_references {
            let project_ref = self.add_project_reference_file_ref(name);
            project_children.push(PbxValue::reference(project_ref.clone(), name.clone()));

            let prefix = format!("{name}/");
            let product_children = self
                .project_reference_product_refs
                .iter()
                .filter_map(|(reference, product_ref)| {
                    reference.strip_prefix(&prefix).map(|target_name| {
                        let product_name = self
                            .project
                            .targets
                            .get(target_name)
                            .map(Target::filename)
                            .unwrap_or_else(|| format!("{target_name}.framework"));
                        PbxValue::reference(product_ref.clone(), product_name)
                    })
                })
                .collect::<Vec<_>>();
            let product_group = self.add_group(
                &format!("projectReferenceProducts:{name}"),
                Some("Products".to_owned()),
                None,
                product_children,
            );
            let mut entry = BTreeMap::new();
            entry.insert(
                "ProductGroup".to_owned(),
                PbxValue::reference(product_group, "Products"),
            );
            entry.insert(
                "ProjectRef".to_owned(),
                PbxValue::reference(project_ref, name.clone()),
            );
            project_references.push(PbxValue::Dict(entry));
        }

        let main_group_children = if project_children.is_empty() {
            Vec::new()
        } else {
            let projects_group = self.add_group(
                "projectReferences",
                Some("Projects".to_owned()),
                None,
                project_children,
            );
            vec![PbxValue::reference(projects_group, "Projects")]
        };
        ProjectReferences {
            main_group_children,
            project_references,
        }
    }

    fn add_project_reference_file_ref(&mut self, name: &str) -> String {
        let path = self
            .project
            .project_references
            .get(name)
            .and_then(|reference| reference.get("path"))
            .and_then(Value::as_str)
            .unwrap_or(name);
        self.add_file_reference(
            &format!("projectReference:{name}"),
            name.to_owned(),
            Some(path.trim_start_matches("./").to_owned()),
            Some("wrapper.pb-project".to_owned()),
            Some(name.to_owned()),
            "<group>",
            true,
        )
    }

    fn project_reference_pbxproj(&self, project_name: &str) -> Option<String> {
        let path = self
            .project
            .project_references
            .get(project_name)?
            .get("path")?
            .as_str()?
            .trim_start_matches("./")
            .to_owned();
        fs::read_to_string(self.project.base_path.join(path).join("project.pbxproj")).ok()
    }

    fn project_reference_target_id(&self, project_name: &str, target_name: &str) -> Option<String> {
        let pbxproj = self.project_reference_pbxproj(project_name)?;
        pbxproj.lines().find_map(|line| {
            if line.contains(&format!("/* {target_name} */ = {{")) {
                line.split_whitespace().next().map(str::to_owned)
            } else {
                None
            }
        })
    }

    fn project_reference_product_id(
        &self,
        project_name: &str,
        target_name: &str,
    ) -> Option<String> {
        let pbxproj = self.project_reference_pbxproj(project_name)?;
        let target_line = pbxproj
            .lines()
            .position(|line| line.contains(&format!("/* {target_name} */ = {{")))?;
        pbxproj.lines().skip(target_line).find_map(|line| {
            line.trim_start()
                .strip_prefix("productReference = ")
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned)
        })
    }

    fn add_project_reference_product(&mut self, reference: &str) -> Option<String> {
        if let Some(existing) = self.project_reference_product_refs.get(reference) {
            return Some(existing.clone());
        }
        let (project_name, target_name) = reference.split_once('/')?;
        if !self.project.project_references.contains_key(project_name) {
            return None;
        }
        let project_ref = self.add_project_reference_file_ref(project_name);
        let product_name = self
            .project
            .targets
            .get(target_name)
            .map(Target::filename)
            .unwrap_or_else(|| format!("{target_name}.framework"));
        let proxy = self.graph.add(
            &format!("projectReferenceProductProxy:{reference}"),
            PbxObject::new("PBXContainerItemProxy", "PBXContainerItemProxy")
                .field(
                    "containerPortal",
                    PbxValue::reference(project_ref, project_name.to_owned()),
                )
                .field("proxyType", PbxValue::Int(2))
                .field(
                    "remoteGlobalIDString",
                    PbxValue::String(
                        self.project_reference_product_id(project_name, target_name)
                            .unwrap_or_else(|| {
                                self.graph.id_for(&format!("nativeTarget:{target_name}"))
                            }),
                    ),
                )
                .field("remoteInfo", PbxValue::String(target_name.to_owned())),
        );
        let product_ref = self.graph.add(
            &format!("projectReferenceProduct:{reference}"),
            PbxObject::new("PBXReferenceProxy", product_name.clone())
                .field(
                    "fileType",
                    PbxValue::String(file_type_for_path(&product_name, None)),
                )
                .field("path", PbxValue::String(product_name))
                .field(
                    "remoteRef",
                    PbxValue::reference(proxy, "PBXContainerItemProxy"),
                )
                .field(
                    "sourceTree",
                    PbxValue::String("BUILT_PRODUCTS_DIR".to_owned()),
                ),
        );
        self.project_reference_product_refs
            .insert(reference.to_owned(), product_ref.clone());
        Some(product_ref)
    }

    fn add_target_dependency(
        &mut self,
        target_name: &str,
        dependency_name: &str,
        dependency: Option<&Dependency>,
    ) -> String {
        if let Some((project_name, external_target_name)) = dependency_name.split_once('/') {
            let project_ref = self.add_project_reference_file_ref(project_name);
            let proxy_id = self.graph.add(
                &format!("projectReferenceTargetProxy:{target_name}:{dependency_name}"),
                PbxObject::new("PBXContainerItemProxy", "PBXContainerItemProxy")
                    .field(
                        "containerPortal",
                        PbxValue::reference(project_ref, project_name.to_owned()),
                    )
                    .field("proxyType", PbxValue::Int(1))
                    .field(
                        "remoteGlobalIDString",
                        PbxValue::String(
                            self.project_reference_target_id(project_name, external_target_name)
                                .unwrap_or_else(|| {
                                    self.graph
                                        .id_for(&format!("nativeTarget:{external_target_name}"))
                                }),
                        ),
                    )
                    .field(
                        "remoteInfo",
                        PbxValue::String(external_target_name.to_owned()),
                    ),
            );
            let mut object = PbxObject::new("PBXTargetDependency", "PBXTargetDependency")
                .field("name", PbxValue::String(external_target_name.to_owned()))
                .field(
                    "targetProxy",
                    PbxValue::reference(proxy_id, "PBXContainerItemProxy"),
                );
            if let Some(platform_filter) = target_dependency_platform_filter(dependency) {
                object = object.field("platformFilter", PbxValue::String(platform_filter));
            }
            return self.graph.add(
                &format!("targetDependency:{target_name}:{dependency_name}"),
                object,
            );
        }
        let project_id = self.graph.id_for("project");
        let dependency_target_id = self.target_id_for_dependency(dependency_name);
        let proxy_id = self.graph.add(
            &format!("containerProxy:{target_name}:{dependency_name}"),
            PbxObject::new("PBXContainerItemProxy", "PBXContainerItemProxy")
                .field(
                    "containerPortal",
                    PbxValue::reference(project_id, "Project object"),
                )
                .field("proxyType", PbxValue::Int(1))
                .field(
                    "remoteGlobalIDString",
                    PbxValue::String(dependency_target_id.clone()),
                )
                .field("remoteInfo", PbxValue::String(dependency_name.to_owned())),
        );
        let mut object = PbxObject::new("PBXTargetDependency", "PBXTargetDependency")
            .field(
                "target",
                PbxValue::reference(dependency_target_id, dependency_name.to_owned()),
            )
            .field(
                "targetProxy",
                PbxValue::reference(proxy_id, "PBXContainerItemProxy"),
            );
        if let Some(platform_filter) = target_dependency_platform_filter(dependency) {
            object = object.field("platformFilter", PbxValue::String(platform_filter));
        }
        self.graph.add(
            &format!("targetDependency:{target_name}:{dependency_name}"),
            object,
        )
    }

    fn target_id_for_dependency(&self, dependency_name: &str) -> String {
        if self
            .project
            .aggregate_target_specs
            .contains_key(dependency_name)
        {
            self.graph
                .id_for(&format!("aggregateTarget:{dependency_name}"))
        } else {
            self.graph
                .id_for(&format!("nativeTarget:{dependency_name}"))
        }
    }

    fn file_system_synchronized_groups(&mut self, target: &Target) -> Vec<PbxValue> {
        let sources = target
            .sources
            .iter()
            .filter(|source| self.effective_source_type(source) == SourceType::SyncedFolder)
            .cloned()
            .collect::<Vec<_>>();
        sources
            .iter()
            .map(|source| {
                let group_id = self.add_file_system_synchronized_root_group(target, source);
                PbxValue::reference(group_id, display_name(&source.path))
            })
            .collect()
    }

    fn add_file_system_synchronized_root_group(
        &mut self,
        _target: &Target,
        source: &TargetSource,
    ) -> String {
        let path = if self.should_create_intermediate_groups(source) {
            display_name(&source.path)
        } else {
            source.path.clone()
        };
        let mut object = PbxObject::new(
            "PBXFileSystemSynchronizedRootGroup",
            display_name(&source.path),
        )
        .field("explicitFileTypes", PbxValue::Dict(BTreeMap::new()))
        .field("explicitFolders", PbxValue::Array(Vec::new()))
        .field("path", PbxValue::String(path))
        .field("sourceTree", PbxValue::String("<group>".to_owned()));
        if let Some(name) = source.name.clone().or_else(|| {
            self.should_create_intermediate_groups(source)
                .then(|| display_name(&source.path))
        }) {
            object = object.field("name", PbxValue::String(name));
        }
        let matching_sources = self
            .project
            .targets
            .values()
            .flat_map(|target| {
                target
                    .sources
                    .iter()
                    .filter(|candidate| {
                        candidate.path == source.path
                            && self.effective_source_type(candidate) == SourceType::SyncedFolder
                    })
                    .map(|candidate| (target.clone(), candidate.clone()))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut explicit_folders = matching_sources
            .iter()
            .flat_map(|(_, source)| {
                explicit_folders_for_synced_source(&self.project.base_path, source)
            })
            .collect::<Vec<_>>();
        explicit_folders.sort();
        explicit_folders.dedup();
        object = object.field("explicitFolders", string_array(&explicit_folders));
        let exception_sets = matching_sources
            .iter()
            .filter_map(|(target, source)| self.add_synced_folder_exception_set(target, source))
            .map(|exception_set| {
                PbxValue::reference(
                    exception_set,
                    "PBXFileSystemSynchronizedBuildFileExceptionSet",
                )
            })
            .collect::<Vec<_>>();
        if !exception_sets.is_empty() {
            object = object.field("exceptions", PbxValue::Array(exception_sets));
        }
        self.graph.add(
            &format!("fileSystemSynchronizedGroup:{}", source.path),
            object,
        )
    }

    fn add_synced_folder_exception_set(
        &mut self,
        target: &Target,
        source: &TargetSource,
    ) -> Option<String> {
        let exceptions = synced_folder_membership_exceptions(
            &self.project.base_path,
            source,
            &self.info_plist_files(target),
        );
        if exceptions.is_empty() {
            return None;
        }
        let target_id = self.graph.id_for(&format!("nativeTarget:{}", target.name));
        Some(
            self.graph.add(
                &format!("syncedFolderExceptions:{}:{}", target.name, source.path),
                PbxObject::new(
                    "PBXFileSystemSynchronizedBuildFileExceptionSet",
                    "PBXFileSystemSynchronizedBuildFileExceptionSet",
                )
                .field("membershipExceptions", string_array(&exceptions))
                .field(
                    "target",
                    PbxValue::reference(target_id, target.name.clone()),
                ),
            ),
        )
    }

    fn add_source_navigator_groups(&mut self) -> Vec<PbxValue> {
        let mut groups = Vec::<(String, String, PbxValue)>::new();
        let mut seen = BTreeSet::new();
        let targets = self.project.targets.values().cloned().collect::<Vec<_>>();
        for target in &targets {
            for source in &target.sources {
                let source_type = self.effective_source_type(source);
                if source.optional
                    && source_type == SourceType::Group
                    && !self.project.base_path.join(&source.path).exists()
                {
                    continue;
                }
                if self
                    .project
                    .file_groups
                    .iter()
                    .any(|group| path_is_under_group(&source.path, group))
                {
                    continue;
                }
                let key = format!(
                    "{:?}:{}:{:?}:{:?}:{:?}",
                    source_type, source.path, source.includes, source.excludes, source.group
                );
                if !seen.insert(key) {
                    continue;
                }
                let Some((id, comment)) = self.add_source_navigator_group(target, source) else {
                    continue;
                };
                let sort_path = source.group.clone().unwrap_or_else(|| source.path.clone());
                groups.push((comment.clone(), sort_path, PbxValue::reference(id, comment)));
            }
        }
        groups.sort_by(|(left_name, left_path, _), (right_name, right_path, _)| {
            natural_cmp(left_name, right_name).then_with(|| natural_cmp(left_path, right_path))
        });

        let mut top_level_seen = BTreeSet::new();
        groups
            .into_iter()
            .filter_map(|(_, _, value)| match value {
                PbxValue::Ref { id, comment } => top_level_seen
                    .insert(id.clone())
                    .then_some(PbxValue::Ref { id, comment }),
                other => Some(other),
            })
            .collect()
    }

    fn add_source_navigator_group(
        &mut self,
        target: &Target,
        source: &TargetSource,
    ) -> Option<(String, String)> {
        match self.effective_source_type(source) {
            SourceType::SyncedFolder => {
                let id = self.add_file_system_synchronized_root_group(target, source);
                let comment = source
                    .name
                    .clone()
                    .unwrap_or_else(|| display_name(&source.path));
                if self.should_create_intermediate_groups(source) {
                    let parent = Path::new(&source.path)
                        .parent()
                        .and_then(|parent| parent.to_str())
                        .unwrap_or("");
                    if !parent.is_empty() {
                        return Some(self.add_nested_navigator_groups(
                            &format!("navigatorIntermediate:{parent}"),
                            parent,
                            PbxValue::reference(id, comment),
                        ));
                    }
                }
                Some((id, comment))
            }
            SourceType::Folder => {
                let comment = source
                    .name
                    .clone()
                    .unwrap_or_else(|| display_name(&source.path));
                let id = self.add_folder_file_reference(source);
                if let Some(group) = source.group.as_deref() {
                    return Some(self.add_nested_navigator_groups(
                        &format!("navigatorCustomGroup:{group}"),
                        group,
                        PbxValue::reference(id, comment),
                    ));
                }
                if self.should_create_intermediate_groups(source) {
                    let parent = Path::new(&source.path)
                        .parent()
                        .and_then(|parent| parent.to_str())
                        .unwrap_or("");
                    if !parent.is_empty() {
                        return Some(self.add_nested_navigator_groups(
                            &format!("navigatorIntermediate:{parent}"),
                            parent,
                            PbxValue::reference(id, display_name(&source.path)),
                        ));
                    }
                }
                Some((id, comment))
            }
            SourceType::File => {
                let path = self.project.base_path.join(&source.path);
                let parent = path.parent().unwrap_or(self.project.base_path.as_path());
                let parent_relative = pathdiff(parent, &self.project.base_path)
                    .to_string_lossy()
                    .into_owned();
                let name = source
                    .name
                    .clone()
                    .unwrap_or_else(|| display_name(&source.path));
                if let Some(group) = source.group.as_deref() {
                    let file_parent = if !parent_relative.is_empty() && group == parent_relative
                        || group.starts_with(&format!("{parent_relative}/"))
                    {
                        parent
                    } else {
                        self.project.base_path.as_path()
                    };
                    let file_name = source.name.as_deref().or_else(|| {
                        Path::new(&source.path)
                            .file_name()
                            .and_then(|name| name.to_str())
                    });
                    let id = self.add_source_file_reference(
                        file_parent,
                        &path,
                        file_name,
                        Some((&path, source)),
                    );
                    return Some(self.add_nested_navigator_groups(
                        &format!("navigatorCustomGroup:{group}"),
                        group,
                        PbxValue::reference(id, name),
                    ));
                }
                if parent_relative.is_empty() {
                    let id = self.add_source_file_reference(
                        parent,
                        &path,
                        source.name.as_deref(),
                        Some((&path, source)),
                    );
                    Some((id, name))
                } else if source.name.is_some() {
                    let id = self.add_source_file_reference(
                        self.project.base_path.as_path(),
                        &path,
                        source.name.as_deref(),
                        Some((&path, source)),
                    );
                    Some((id, name))
                } else if self.should_create_intermediate_groups(source) {
                    let child = self.add_source_file_reference(
                        parent,
                        &path,
                        source.name.as_deref(),
                        Some((&path, source)),
                    );
                    Some(self.add_nested_navigator_groups(
                        &format!("navigatorIntermediate:{parent_relative}"),
                        &parent_relative,
                        PbxValue::reference(child, name),
                    ))
                } else {
                    let child = self.add_source_file_reference(
                        parent,
                        &path,
                        source.name.as_deref(),
                        Some((&path, source)),
                    );
                    let group_name = display_name(&parent_relative);
                    let id = self.add_group(
                        &format!("navigatorGroup:{parent_relative}"),
                        None,
                        Some(parent_relative),
                        vec![PbxValue::reference(
                            child,
                            source
                                .name
                                .clone()
                                .unwrap_or_else(|| display_name(&source.path)),
                        )],
                    );
                    Some((id, group_name))
                }
            }
            SourceType::Group | SourceType::Other(_) => {
                let path = self.project.base_path.join(&source.path);
                if !path.is_dir() {
                    return None;
                }
                let parent = path.parent().unwrap_or(self.project.base_path.as_path());
                let parent_relative = pathdiff(parent, &self.project.base_path)
                    .to_string_lossy()
                    .into_owned();
                let group_parent = if source
                    .group
                    .as_deref()
                    .is_some_and(|group| group == parent_relative)
                {
                    parent
                } else if self.should_create_intermediate_groups(source) {
                    path.parent().unwrap_or(self.project.base_path.as_path())
                } else {
                    self.project.base_path.as_path()
                };
                let id = self.add_directory_group(source, &path, group_parent, true)?;
                let comment = source
                    .name
                    .clone()
                    .unwrap_or_else(|| display_name(&source.path));
                if let Some(group) = source.group.as_deref() {
                    return Some(self.add_nested_navigator_groups(
                        &format!("navigatorCustomGroup:{group}"),
                        group,
                        PbxValue::reference(id, comment),
                    ));
                }
                if self.should_create_intermediate_groups(source) {
                    let parent = Path::new(&source.path)
                        .parent()
                        .and_then(|parent| parent.to_str())
                        .unwrap_or("");
                    if !parent.is_empty() {
                        return Some(self.add_nested_navigator_groups(
                            &format!("navigatorIntermediate:{parent}"),
                            parent,
                            PbxValue::reference(id, comment),
                        ));
                    }
                }
                Some((id, comment))
            }
        }
    }

    fn should_create_intermediate_groups(&self, source: &TargetSource) -> bool {
        source
            .create_intermediate_groups
            .unwrap_or(self.project.spec_options.create_intermediate_groups)
    }

    fn add_nested_navigator_groups(
        &mut self,
        key_prefix: &str,
        group_path: &str,
        leaf: PbxValue,
    ) -> (String, String) {
        let parts = group_path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.is_empty() {
            return match leaf {
                PbxValue::Ref { id, comment } => {
                    let comment = comment.unwrap_or_else(|| {
                        self.graph.comments.get(&id).cloned().unwrap_or_default()
                    });
                    (id, comment)
                }
                other => {
                    let group_id = self.add_group(key_prefix, None, None, vec![other]);
                    (group_id, display_name(group_path))
                }
            };
        }

        let mut child = leaf;
        for index in (0..parts.len()).rev() {
            let path = parts[..=index].join("/");
            let name = parts[index].to_owned();
            let custom_name_group =
                key_prefix.starts_with("navigatorCustomGroup:") && path == "General";
            let id = self.add_group(
                &format!("navigatorGroup:{path}"),
                custom_name_group.then_some(name.clone()),
                (!custom_name_group).then_some(name.clone()),
                vec![child],
            );
            child = PbxValue::reference(id, name);
        }

        match child {
            PbxValue::Ref { id, comment } => (id, comment.unwrap_or_else(|| parts[0].to_owned())),
            _ => unreachable!(),
        }
    }

    fn add_directory_group(
        &mut self,
        source: &TargetSource,
        directory: &Path,
        group_parent: &Path,
        root: bool,
    ) -> Option<String> {
        let mut entries = fs::read_dir(directory)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                (!name.starts_with('.') || name == ".swiftlint.yml")
                    && name != "Carthage"
                    && !name.ends_with(".xcodeproj")
                    && path.extension().and_then(|extension| extension.to_str()) != Some("orig")
            })
            .collect::<Vec<_>>();
        entries.sort_by(|left, right| {
            natural_cmp(
                &display_name(&left.to_string_lossy()),
                &display_name(&right.to_string_lossy()),
            )
        });

        let mut children = Vec::new();
        let mut seen_children = BTreeSet::new();
        for entry in entries {
            let source_root = self.project.base_path.join(&source.path);
            if entry.is_dir()
                && entry.extension().and_then(|extension| extension.to_str()) == Some("lproj")
            {
                if let Ok(localized_entries) = fs::read_dir(&entry) {
                    let mut localized_files = localized_entries
                        .flatten()
                        .map(|entry| entry.path())
                        .filter(|path| {
                            path.is_file()
                                && !path
                                    .file_name()
                                    .and_then(|name| name.to_str())
                                    .is_some_and(|name| name.starts_with('.'))
                        })
                        .collect::<Vec<_>>();
                    localized_files.sort();
                    for localized_file in localized_files {
                        if !source_matches_filters(
                            &self.project.base_path.join(&source.path),
                            &localized_file,
                            source,
                        ) {
                            continue;
                        }
                        let file_id = self.add_source_file_reference(
                            directory,
                            &localized_file,
                            None,
                            Some((&source_root, source)),
                        );
                        if seen_children.insert(file_id.clone()) {
                            children.push(PbxValue::reference(
                                file_id,
                                localized_variant_group_name(&localized_file).unwrap_or_else(
                                    || display_name(&localized_file.to_string_lossy()),
                                ),
                            ));
                        }
                    }
                }
                continue;
            }
            if entry.is_dir() && source_excludes_directory(&source_root, &entry, source) {
                continue;
            }
            if entry.is_dir()
                && !is_wrapper_path(&entry)
                && !file_type_options(
                    &entry.to_string_lossy(),
                    &self.project.spec_options.file_types,
                )
                .is_some_and(|file_type| file_type.file)
            {
                if let Some(group_id) = self.add_directory_group(source, &entry, directory, false) {
                    children.push(PbxValue::reference(
                        group_id,
                        display_name(&pathdiff(&entry, directory).to_string_lossy()),
                    ));
                }
            } else if source_matches_filters(
                &self.project.base_path.join(&source.path),
                &entry,
                source,
            ) || self.is_file_group_path(&entry)
            {
                let file_id = if self.is_file_group_path(&entry) {
                    let relative = pathdiff(&entry, &self.project.base_path)
                        .to_string_lossy()
                        .into_owned();
                    self.add_file_reference(
                        &format!("fileGroup:{relative}"),
                        display_name(&relative),
                        Some(pathdiff(&entry, directory).to_string_lossy().into_owned()),
                        None,
                        None,
                        "<group>",
                        true,
                    )
                } else {
                    self.add_source_file_reference(
                        directory,
                        &entry,
                        None,
                        Some((&self.project.base_path.join(&source.path), source)),
                    )
                };
                if seen_children.insert(file_id.clone()) {
                    children.push(PbxValue::reference(
                        file_id,
                        display_name(&entry.to_string_lossy()),
                    ));
                }
            }
        }

        for generated_path in self.generated_plist_paths_under(directory) {
            let file_id = self.add_source_file_reference(directory, &generated_path, None, None);
            if seen_children.insert(file_id.clone()) {
                children.push(PbxValue::reference(
                    file_id,
                    display_name(&generated_path.to_string_lossy()),
                ));
            }
        }
        children =
            self.sorted_group_children(&display_name(&directory.to_string_lossy()), children);
        let relative = pathdiff(directory, &self.project.base_path)
            .to_string_lossy()
            .into_owned();
        let group_path = pathdiff(directory, group_parent)
            .to_string_lossy()
            .into_owned();
        let name = if root {
            source
                .name
                .clone()
                .unwrap_or_else(|| display_name(&group_path))
        } else {
            display_name(&group_path)
        };
        Some(
            self.add_group(
                &format!("navigatorGroup:{relative}"),
                (root && (source.name.is_some() || display_name(&group_path) != group_path))
                    .then_some(name.clone()),
                Some(group_path),
                children,
            ),
        )
    }

    fn generated_plist_paths_under(&self, directory: &Path) -> Vec<PathBuf> {
        let mut paths = BTreeSet::new();
        for target in self.project.targets.values() {
            for plist in [&target.info_plist, &target.entitlements_plist]
                .into_iter()
                .flatten()
            {
                let Some(path) = &plist.path else {
                    continue;
                };
                let full_path = self.project.base_path.join(path);
                if !full_path.exists()
                    && full_path.parent().is_some_and(|parent| parent == directory)
                {
                    paths.insert(full_path);
                }
            }
        }
        paths.into_iter().collect()
    }

    fn is_file_group_path(&self, path: &Path) -> bool {
        let relative = pathdiff(path, &self.project.base_path)
            .to_string_lossy()
            .into_owned();
        self.project
            .file_groups
            .iter()
            .any(|group| group == &relative)
    }

    fn add_folder_file_reference(&mut self, source: &TargetSource) -> String {
        let comment = source
            .name
            .clone()
            .unwrap_or_else(|| display_name(&source.path));
        let name = source.name.clone().or_else(|| {
            (comment != source.path && comment == display_name(&source.path))
                .then(|| comment.clone())
        });
        self.add_file_reference(
            &format!("folderFileRef:{}", source.path),
            comment,
            Some(source.path.clone()),
            Some("folder".to_owned()),
            name,
            "SOURCE_ROOT",
            true,
        )
    }

    fn add_source_file_reference(
        &mut self,
        parent: &Path,
        path: &Path,
        name: Option<&str>,
        source_filter: Option<(&Path, &TargetSource)>,
    ) -> String {
        if name.is_none() {
            if let Some(variant_id) = self.add_variant_group_reference(parent, path, source_filter)
            {
                return variant_id;
            }
            if let Some(version_group_id) = self.add_model_version_group_reference(parent, path) {
                return version_group_id;
            }
        }
        self.add_regular_source_file_reference(parent, path, name)
    }

    fn add_regular_source_file_reference(
        &mut self,
        parent: &Path,
        path: &Path,
        name: Option<&str>,
    ) -> String {
        let relative = pathdiff(path, &self.project.base_path)
            .to_string_lossy()
            .into_owned();
        let file_path = pathdiff(path, parent).to_string_lossy().into_owned();
        let comment = name
            .map(str::to_owned)
            .unwrap_or_else(|| display_name(&relative));
        let last_known_file_type = {
            let file_type = file_type_for_path(&relative, None);
            if relative.ends_with(".xctestplan") || file_type == "file" {
                None
            } else {
                Some(file_type)
            }
        };
        self.add_file_reference(
            &format!("navigatorFileRef:{relative}"),
            comment,
            Some(file_path),
            last_known_file_type,
            name.map(str::to_owned)
                .or_else(|| is_localized_file(path).then(|| display_name(&relative)))
                .filter(|value| {
                    *value != display_name(&relative)
                        || is_localized_file(path)
                        || Path::new(&relative)
                            .parent()
                            .is_some_and(|parent| !parent.as_os_str().is_empty())
                }),
            "<group>",
            true,
        )
    }

    fn add_model_version_group_reference(&mut self, parent: &Path, path: &Path) -> Option<String> {
        if path.extension().and_then(|extension| extension.to_str()) != Some("xcdatamodeld") {
            return None;
        }
        let mut model_versions = fs::read_dir(path)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|entry| {
                entry.is_dir()
                    && entry.extension().and_then(|extension| extension.to_str())
                        == Some("xcdatamodel")
            })
            .collect::<Vec<_>>();
        model_versions.sort_by(|left, right| {
            natural_cmp(
                &display_name(&left.to_string_lossy()),
                &display_name(&right.to_string_lossy()),
            )
        });
        if model_versions.is_empty() {
            return None;
        }

        let relative = pathdiff(path, &self.project.base_path)
            .to_string_lossy()
            .into_owned();
        let group_name = display_name(&relative);
        let current_version_name = model_current_version_name(path);
        let mut children = Vec::new();
        let mut current_version = None;
        for model_version in model_versions {
            let version_name = display_name(&model_version.to_string_lossy());
            let version_relative = pathdiff(&model_version, &self.project.base_path)
                .to_string_lossy()
                .into_owned();
            let file_ref = self.add_file_reference(
                &format!("modelVersionFileRef:{version_relative}"),
                version_name.clone(),
                Some(
                    pathdiff(&model_version, path)
                        .to_string_lossy()
                        .into_owned(),
                ),
                Some("wrapper.xcdatamodel".to_owned()),
                None,
                "<group>",
                true,
            );
            if current_version_name.as_deref() == Some(version_name.as_str()) {
                current_version = Some(PbxValue::reference(file_ref.clone(), version_name.clone()));
            }
            children.push(PbxValue::reference(file_ref, version_name));
        }

        if current_version.is_none() && children.len() == 1 {
            current_version = children.first().cloned();
        }

        let mut object = PbxObject::new("XCVersionGroup", group_name.clone())
            .field("children", PbxValue::Array(children))
            .field(
                "path",
                PbxValue::String(pathdiff(path, parent).to_string_lossy().into_owned()),
            )
            .field("sourceTree", PbxValue::String("<group>".to_owned()))
            .field(
                "versionGroupType",
                PbxValue::String("wrapper.xcdatamodel".to_owned()),
            );
        if let Some(current_version) = current_version {
            object = object.field("currentVersion", current_version);
        }
        Some(
            self.graph
                .add(&format!("modelVersionGroup:{relative}"), object),
        )
    }

    fn add_variant_group_reference(
        &mut self,
        parent: &Path,
        path: &Path,
        source_filter: Option<(&Path, &TargetSource)>,
    ) -> Option<String> {
        let lproj_dir = path.parent()?;
        if lproj_dir
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("lproj")
        {
            return None;
        }
        let variant_parent = lproj_dir.parent()?;
        let group_name = localized_variant_group_name(path)?;
        if let Some((source_root, source)) = source_filter {
            if !localized_variant_group_matches_source(source_root, source, path) {
                return None;
            }
        }
        let mut children = Vec::new();
        let mut localized_files = localized_variant_files(variant_parent, &group_name);
        if let Some((source_root, source)) = source_filter {
            localized_files.retain(|file| source_matches_filters(source_root, file, source));
        }
        if localized_files.is_empty() {
            return None;
        }
        localized_files.sort_by(|left, right| {
            natural_cmp(
                &localized_file_locale(left)
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
                &localized_file_locale(right)
                    .unwrap_or_default()
                    .to_ascii_lowercase(),
            )
        });
        for localized_file in localized_files {
            let locale = localized_file_locale(&localized_file)?;
            let relative = pathdiff(&localized_file, &self.project.base_path)
                .to_string_lossy()
                .into_owned();
            let file_path = pathdiff(&localized_file, parent)
                .to_string_lossy()
                .into_owned();
            let file_ref = self.add_file_reference(
                &format!("variantFileRef:{relative}"),
                locale.clone(),
                Some(file_path),
                Some(file_type_for_path(&relative, None)),
                Some(locale.clone()),
                "<group>",
                true,
            );
            children.push(PbxValue::reference(file_ref, locale));
        }
        Some(
            self.graph.add(
                &format!(
                    "variantGroup:{}:{group_name}",
                    pathdiff(variant_parent, &self.project.base_path).to_string_lossy()
                ),
                PbxObject::new("PBXVariantGroup", group_name.clone())
                    .field("children", PbxValue::Array(children))
                    .field("name", PbxValue::String(group_name))
                    .field("sourceTree", PbxValue::String("<group>".to_owned())),
            ),
        )
    }

    fn add_frameworks_navigator_group(&mut self) -> Option<PbxValue> {
        let mut references = BTreeMap::<String, PbxValue>::new();
        let mut carthage_children = BTreeMap::<Platform, BTreeMap<String, PbxValue>>::new();
        let targets = self.project.targets.values().cloned().collect::<Vec<_>>();
        for target in &targets {
            for dependency in &target.dependencies {
                let (name, path, source_tree) = match &dependency.dependency_type {
                    DependencyType::Framework => {
                        let name = display_name(&dependency.reference);
                        (name, dependency.reference.clone(), "<group>")
                    }
                    DependencyType::Sdk { .. } => {
                        let name = display_name(&dependency.reference);
                        let source_tree = match &dependency.dependency_type {
                            DependencyType::Sdk { root } => root.as_deref().unwrap_or("SDKROOT"),
                            _ => "SDKROOT",
                        };
                        (
                            name,
                            sdk_reference_path(&dependency.reference, source_tree),
                            source_tree,
                        )
                    }
                    DependencyType::Carthage { .. } => {
                        let refs = carthage_dependency_references(self.project, target, dependency);
                        let children = carthage_children
                            .entry(target.platform.clone())
                            .or_default();
                        for reference in refs {
                            let name = carthage_framework_name(&reference);
                            let id = self
                                .graph
                                .id_for(&format!("carthageRef:{}:{reference}", target.name));
                            children.insert(name.clone(), PbxValue::reference(id, name));
                        }
                        continue;
                    }
                    _ => continue,
                };
                let key = if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                    framework_dependency_reference_key(&target.name, dependency, &path)
                } else {
                    format!("navigatorFrameworkRef:{path}")
                };
                let id = self.add_file_reference(
                    &key,
                    name.clone(),
                    Some(path),
                    Some(file_type_for_path(&name, None)),
                    matches!(dependency.dependency_type, DependencyType::Sdk { .. })
                        .then_some(name.clone()),
                    source_tree,
                    true,
                );
                references.insert(name.clone(), PbxValue::reference(id, name));
            }
        }
        if !carthage_children.is_empty() {
            let mut carthage_groups = BTreeMap::<Platform, PbxValue>::new();
            for (platform, children) in carthage_children {
                let platform_path = carthage_platform_dir(&platform).to_owned();
                let platform_group = self.add_group(
                    &format!("Carthage:{platform_path}"),
                    None,
                    Some(platform_path.clone()),
                    children.into_values().collect(),
                );
                carthage_groups
                    .insert(platform, PbxValue::reference(platform_group, platform_path));
            }
            let carthage_group_children = ordered_carthage_platform_groups(&carthage_groups);
            let carthage_group = self.add_group(
                "Carthage",
                Some("Carthage".to_owned()),
                Some("Carthage/Build".to_owned()),
                carthage_group_children,
            );
            references.insert(
                "Carthage".to_owned(),
                PbxValue::reference(carthage_group, "Carthage"),
            );
        }
        if references.is_empty() {
            return None;
        }
        let group_id = self.add_group(
            "Frameworks",
            Some("Frameworks".to_owned()),
            None,
            references.into_values().collect(),
        );
        Some(PbxValue::reference(group_id, "Frameworks"))
    }

    fn add_bundles_navigator_group(&mut self) -> Option<PbxValue> {
        let mut references = BTreeMap::<String, PbxValue>::new();
        for target in self.project.targets.values() {
            for dependency in &target.dependencies {
                if dependency.dependency_type != DependencyType::Bundle {
                    continue;
                }
                let reference = &dependency.reference;
                let name = display_name(reference);
                let file_ref = self.add_file_reference(
                    &format!("bundleDependencyRef:{reference}"),
                    name.clone(),
                    Some(reference.clone()),
                    Some(file_type_for_path(reference, None)),
                    None,
                    "BUILT_PRODUCTS_DIR",
                    true,
                );
                references.insert(name.clone(), PbxValue::reference(file_ref, name));
            }
        }
        if references.is_empty() {
            return None;
        }
        let group_id = self.add_group(
            "bundleDependenciesGroup",
            Some("Bundles".to_owned()),
            None,
            references.into_values().collect(),
        );
        Some(PbxValue::reference(group_id, "Bundles"))
    }

    fn collect_target_files(&mut self, target: &Target) -> Vec<FileBuildRefs> {
        let mut refs = Vec::new();
        let mut seen = BTreeSet::new();
        let info_plist_files = self.info_plist_files(target);
        for source in &target.sources {
            let effective_source_type = self.effective_source_type(source);
            if effective_source_type == SourceType::SyncedFolder {
                continue;
            }
            if source.optional
                && effective_source_type == SourceType::Group
                && !self.project.base_path.join(&source.path).exists()
            {
                continue;
            }
            let expanded_paths = expand_source_path(
                &self.project.base_path,
                source,
                &self.project.spec_options.file_types,
            );
            let source_path = self.project.base_path.join(&source.path);
            let custom_name_applies = source.name.is_some() && !source_path.is_dir();
            for path in expanded_paths {
                let variant =
                    localized_variant_group_path(&self.project.base_path, &path).filter(|_| {
                        localized_variant_group_matches_source(&source_path, source, &path)
                    });
                let relative_string = variant.clone().unwrap_or_else(|| {
                    pathdiff(&path, &self.project.base_path)
                        .to_string_lossy()
                        .into_owned()
                });
                if !seen.insert(relative_string.clone()) {
                    continue;
                }
                let name = if custom_name_applies {
                    source
                        .name
                        .clone()
                        .unwrap_or_else(|| display_name(&relative_string))
                } else {
                    display_name(&relative_string)
                };
                let parent = if custom_name_applies {
                    self.project.base_path.as_path()
                } else if variant.is_some() {
                    path.parent()
                        .and_then(Path::parent)
                        .unwrap_or(self.project.base_path.as_path())
                } else if effective_source_type == SourceType::Folder {
                    self.project.base_path.as_path()
                } else {
                    path.parent().unwrap_or(self.project.base_path.as_path())
                };
                let file_ref_id = if effective_source_type == SourceType::Folder {
                    self.add_folder_file_reference(source)
                } else if variant.is_none() && is_localized_file(&path) {
                    self.add_regular_source_file_reference(
                        parent,
                        &path,
                        custom_name_applies.then_some(name.as_str()),
                    )
                } else {
                    self.add_source_file_reference(
                        parent,
                        &path,
                        custom_name_applies.then_some(name.as_str()),
                        Some((&source_path, source)),
                    )
                };
                let mut build_phase =
                    if let Some(override_phase) = source_build_phase_override(source) {
                        override_phase
                    } else if effective_source_type == SourceType::Folder {
                        Some("Resources")
                    } else if Path::new(&relative_string)
                        .file_name()
                        .and_then(|name| name.to_str())
                        == Some("Info.plist")
                        || info_plist_files.contains(&relative_string)
                    {
                        None
                    } else if let Some(file_type) =
                        file_type_options(&relative_string, &self.project.spec_options.file_types)
                            .filter(|file_type| file_type.build_phase.is_some())
                    {
                        build_phase_for_file_type(file_type)
                    } else {
                        build_phase_for_source(&relative_string)
                    };
                if build_phase == Some("Headers")
                    && target.target_type == ProductType::StaticLibrary
                {
                    build_phase = if is_public_header_source(source) {
                        Some("CopyHeaders")
                    } else {
                        None
                    };
                } else if (build_phase == Some("CopyFiles")
                    && is_module_copy_file(&relative_string)
                    && !(target.target_type.is_framework()
                        || target.target_type == ProductType::StaticLibrary))
                    || (build_phase == Some("Headers") && !target_supports_headers_phase(target))
                {
                    build_phase = None;
                }
                let copy_files_settings = match build_phase {
                    Some("CopyHeaders") => Some(CopyFilesSettings {
                        dst_subfolder_spec: 16,
                        dst_path: "include/$(PRODUCT_NAME)".to_owned(),
                        phase_name: "CopyFiles".to_owned(),
                        phase_order: CopyFilesPhaseOrder::PreCompile,
                    }),
                    Some("CopyFiles") => {
                        source_copy_files_settings(target, source, &relative_string)
                    }
                    _ => None,
                };
                let build_file_id = if let Some(phase) = build_phase {
                    let build_file_phase_name = if phase == "CopyHeaders" {
                        "CopyFiles"
                    } else {
                        phase
                    };
                    let mut object = PbxObject::new(
                        "PBXBuildFile",
                        format!("{name} in {build_file_phase_name}"),
                    )
                    .field(
                        "fileRef",
                        PbxValue::reference(file_ref_id.clone(), name.clone()),
                    );
                    let file_type =
                        file_type_options(&relative_string, &self.project.spec_options.file_types);
                    let build_file_settings = source_build_file_settings(source, file_type, phase);
                    if !build_file_settings.is_empty() {
                        object = object.field("settings", PbxValue::Dict(build_file_settings));
                    }
                    let platform_filters = platform_filters_for_source(source, &relative_string);
                    if !platform_filters.is_empty() {
                        object = object.field("platformFilters", string_array(&platform_filters));
                    }
                    Some(self.graph.add(
                        &format!("buildFile:{}:{relative_string}:{phase}", target.name),
                        object,
                    ))
                } else {
                    None
                };
                refs.push(FileBuildRefs {
                    build_file_id,
                    name,
                    build_phase,
                    copy_files_settings,
                });
            }
        }
        refs
    }

    fn effective_source_type(&self, source: &TargetSource) -> SourceType {
        if let Some(source_type) = &source.source_type {
            return source_type.clone();
        }
        let path = self.project.base_path.join(&source.path);
        if path.is_file() || Path::new(&source.path).extension().is_some() {
            SourceType::File
        } else {
            self.project
                .spec_options
                .default_source_directory_type
                .clone()
                .unwrap_or(SourceType::Group)
        }
    }

    fn framework_build_files(
        &mut self,
        target: &Target,
        package_product_dependencies: &mut Vec<PbxValue>,
        package_target_dependencies: &mut Vec<PbxValue>,
    ) -> Vec<PbxValue> {
        let mut files = Vec::new();
        for dependency in &target.dependencies {
            match &dependency.dependency_type {
                DependencyType::Framework | DependencyType::Sdk { .. } => {
                    let reference = &dependency.reference;
                    let name = display_name(reference);
                    let source_tree =
                        if let DependencyType::Sdk { root } = &dependency.dependency_type {
                            root.as_deref().unwrap_or("SDKROOT")
                        } else {
                            "<group>"
                        };
                    let path = if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                        sdk_reference_path(reference, source_tree)
                    } else {
                        reference.clone()
                    };
                    let file_ref = self.add_file_reference(
                        &framework_dependency_reference_key(&target.name, dependency, &path),
                        name.clone(),
                        Some(path),
                        Some(file_type_for_path(&name, None)),
                        matches!(dependency.dependency_type, DependencyType::Framework)
                            .then_some(name.clone()),
                        source_tree,
                        true,
                    );
                    let mut settings = BTreeMap::new();
                    if dependency.weak_link {
                        settings.insert(
                            "ATTRIBUTES".to_owned(),
                            PbxValue::Array(vec![PbxValue::String("Weak".to_owned())]),
                        );
                    }
                    let mut object =
                        PbxObject::new("PBXBuildFile", format!("{name} in Frameworks"))
                            .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                    if let Some(platform_filter) = dependency_platform_filter(dependency) {
                        object = object.field("platformFilter", PbxValue::String(platform_filter));
                    }
                    let platform_filters = platform_filters_for_dependency(dependency);
                    if !platform_filters.is_empty() {
                        object = object.field("platformFilters", string_array(&platform_filters));
                    }
                    if !settings.is_empty() {
                        object = object.field("settings", PbxValue::Dict(settings));
                    }
                    let build_file = self.graph.add(
                        &format!("frameworkBuildFile:{}:{reference}", target.name),
                        object,
                    );
                    files.push(PbxValue::reference(
                        build_file,
                        format!("{name} in Frameworks"),
                    ));
                }
                DependencyType::Target => {
                    if dependency.reference.contains('/') {
                        let Some(product_ref_id) =
                            self.add_project_reference_product(&dependency.reference)
                        else {
                            continue;
                        };
                        let name = self
                            .graph
                            .comments
                            .get(&product_ref_id)
                            .cloned()
                            .unwrap_or_else(|| display_name(&dependency.reference));
                        let mut object = PbxObject::new(
                            "PBXBuildFile",
                            format!("{name} in Frameworks"),
                        )
                        .field("fileRef", PbxValue::reference(product_ref_id, name.clone()));
                        if let Some(platform_filter) = dependency_platform_filter(dependency) {
                            object =
                                object.field("platformFilter", PbxValue::String(platform_filter));
                        }
                        let platform_filters = platform_filters_for_dependency(dependency);
                        if !platform_filters.is_empty() {
                            object =
                                object.field("platformFilters", string_array(&platform_filters));
                        }
                        let build_file = self.graph.add(
                            &format!(
                                "projectReferenceProductBuildFile:{}:{}",
                                target.name, dependency.reference
                            ),
                            object,
                        );
                        files.push(PbxValue::reference(
                            build_file,
                            format!("{name} in Frameworks"),
                        ));
                        continue;
                    }
                    let Some(dependency_target) = self.project.targets.get(&dependency.reference)
                    else {
                        continue;
                    };
                    if !target_dependency_should_link(target, dependency, dependency_target) {
                        continue;
                    }
                    if let Some(refs) = self.target_refs.get(&dependency.reference) {
                        let name = self
                            .graph
                            .comments
                            .get(&refs.product_ref_id)
                            .cloned()
                            .unwrap_or_else(|| dependency.reference.clone());
                        let mut settings = BTreeMap::new();
                        if dependency.weak_link {
                            settings.insert(
                                "ATTRIBUTES".to_owned(),
                                PbxValue::Array(vec![PbxValue::String("Weak".to_owned())]),
                            );
                        }
                        let mut object =
                            PbxObject::new("PBXBuildFile", format!("{name} in Frameworks")).field(
                                "fileRef",
                                PbxValue::reference(refs.product_ref_id.clone(), name.clone()),
                            );
                        if let Some(platform_filter) = dependency_platform_filter(dependency) {
                            object =
                                object.field("platformFilter", PbxValue::String(platform_filter));
                        }
                        if !settings.is_empty() {
                            object = object.field("settings", PbxValue::Dict(settings));
                        }
                        let platform_filters = platform_filters_for_dependency(dependency);
                        if !platform_filters.is_empty() {
                            object =
                                object.field("platformFilters", string_array(&platform_filters));
                        }
                        let build_file = self.graph.add(
                            &format!(
                                "targetProductBuildFile:{}:{}",
                                target.name, dependency.reference
                            ),
                            object,
                        );
                        files.push(PbxValue::reference(
                            build_file,
                            format!("{name} in Frameworks"),
                        ));
                    }
                }
                DependencyType::Package { products } => {
                    let product_names = if products.is_empty() {
                        vec![dependency.reference.clone()]
                    } else {
                        products.clone()
                    };
                    let platform_filters = platform_filters_for_dependency(dependency);
                    for product_name in product_names {
                        let product_dependency = self.add_package_product_dependency(
                            &target.name,
                            &dependency.reference,
                            &product_name,
                            false,
                            &platform_filters,
                        );
                        if dependency.link.unwrap_or(true) {
                            package_product_dependencies.push(PbxValue::reference(
                                product_dependency.clone(),
                                product_name.clone(),
                            ));
                        }
                        if dependency
                            .link
                            .unwrap_or(target.target_type != ProductType::StaticLibrary)
                        {
                            files.push(self.package_product_build_file(
                                &target.name,
                                &product_name,
                                &product_dependency,
                                dependency.weak_link,
                                dependency_platform_filter(dependency),
                                &platform_filters,
                            ));
                        } else {
                            package_target_dependencies.push(
                                self.package_product_target_dependency(
                                    &target.name,
                                    &product_name,
                                    &product_dependency,
                                    &platform_filters,
                                ),
                            );
                        }
                    }
                }
                DependencyType::Carthage { .. } => {
                    if dependency.link == Some(false) {
                        continue;
                    }
                    for reference in
                        carthage_dependency_references(self.project, target, dependency)
                    {
                        let name = carthage_framework_name(&reference);
                        let file_ref = self.add_file_reference(
                            &format!("carthageRef:{}:{reference}", target.name),
                            name.clone(),
                            Some(name.clone()),
                            Some(file_type_for_path(&name, None)),
                            None,
                            "<group>",
                            true,
                        );
                        let mut object =
                            PbxObject::new("PBXBuildFile", format!("{name} in Frameworks"))
                                .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                        if let Some(platform_filter) = dependency_platform_filter(dependency) {
                            object =
                                object.field("platformFilter", PbxValue::String(platform_filter));
                        }
                        let platform_filters = platform_filters_for_dependency(dependency);
                        if !platform_filters.is_empty() {
                            object =
                                object.field("platformFilters", string_array(&platform_filters));
                        }
                        let build_file = self.graph.add(
                            &format!("carthageBuildFile:{}:{reference}", target.name),
                            object,
                        );
                        files.push(PbxValue::reference(
                            build_file,
                            format!("{name} in Frameworks"),
                        ));
                    }
                }
                DependencyType::Bundle => {}
            }
        }
        if product_type_is_app(&target.target_type)
            && target_uses_transitive_dependencies(self.project, target)
        {
            let direct_target_dependencies = target
                .dependencies
                .iter()
                .filter(|dependency| dependency.dependency_type == DependencyType::Target)
                .map(|dependency| dependency.reference.as_str())
                .collect::<HashSet<_>>();
            for dependency_name in native_target_dependency_names(self.project, target) {
                if direct_target_dependencies.contains(dependency_name.as_str()) {
                    continue;
                }
                let Some(dependency_target) = self.project.targets.get(&dependency_name) else {
                    continue;
                };
                if dependency_target.target_type != ProductType::StaticLibrary {
                    continue;
                }
                let Some(refs) = self.target_refs.get(&dependency_name) else {
                    continue;
                };
                let name = self
                    .graph
                    .comments
                    .get(&refs.product_ref_id)
                    .cloned()
                    .unwrap_or_else(|| dependency_target.filename());
                let object = PbxObject::new("PBXBuildFile", format!("{name} in Frameworks")).field(
                    "fileRef",
                    PbxValue::reference(refs.product_ref_id.clone(), name.clone()),
                );
                let build_file = self.graph.add(
                    &format!(
                        "transitiveTargetProductBuildFile:{}:{}",
                        target.name, dependency_name
                    ),
                    object,
                );
                files.push(PbxValue::reference(
                    build_file,
                    format!("{name} in Frameworks"),
                ));
            }
        }
        if target_uses_transitive_dependencies(self.project, target) {
            for dependency_name in native_target_dependency_names(self.project, target) {
                let Some(dependency_target) = self.project.targets.get(&dependency_name) else {
                    continue;
                };
                if product_type_is_test(&target.target_type)
                    && !dependency_target.target_type.is_framework()
                {
                    continue;
                }
                for dependency in &dependency_target.dependencies {
                    if !matches!(
                        dependency.dependency_type,
                        DependencyType::Framework | DependencyType::Sdk { .. }
                    ) || dependency.link == Some(false)
                    {
                        continue;
                    }
                    let reference = &dependency.reference;
                    let name = display_name(reference);
                    let source_tree =
                        if let DependencyType::Sdk { root } = &dependency.dependency_type {
                            root.as_deref().unwrap_or("SDKROOT")
                        } else {
                            "<group>"
                        };
                    let path = if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                        sdk_reference_path(reference, source_tree)
                    } else {
                        reference.clone()
                    };
                    let file_ref = self.add_file_reference(
                        &framework_dependency_reference_key(&target.name, dependency, &path),
                        name.clone(),
                        Some(path),
                        Some(file_type_for_path(&name, None)),
                        None,
                        source_tree,
                        false,
                    );
                    let mut object =
                        PbxObject::new("PBXBuildFile", format!("{name} in Frameworks"))
                            .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                    let platform_filters = platform_filters_for_dependency(dependency);
                    if !platform_filters.is_empty() {
                        object = object.field("platformFilters", string_array(&platform_filters));
                    }
                    let build_file = self.graph.add(
                        &format!(
                            "transitiveFrameworkBuildFile:{}:{}:{reference}",
                            target.name, dependency_name
                        ),
                        object,
                    );
                    files.push(PbxValue::reference(
                        build_file,
                        format!("{name} in Frameworks"),
                    ));
                }
            }
        }
        files
    }

    fn bundle_copy_files_phase(&mut self, target: &Target) -> Option<String> {
        let files = target
            .dependencies
            .iter()
            .filter(|dependency| dependency.dependency_type == DependencyType::Bundle)
            .map(|dependency| {
                let reference = &dependency.reference;
                let name = display_name(reference);
                let file_ref = self.add_file_reference(
                    &format!("bundleDependencyRef:{reference}"),
                    name.clone(),
                    Some(reference.clone()),
                    Some(file_type_for_path(reference, None)),
                    None,
                    "BUILT_PRODUCTS_DIR",
                    true,
                );
                let build_file_comment = format!("{name} in Copy Bundle Resources");
                let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone())
                    .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                if let Some(build_file_settings) =
                    copy_build_file_settings_for_dependency(dependency)
                {
                    object = object.field("settings", build_file_settings);
                }
                let platform_filters = platform_filters_for_dependency(dependency);
                if !platform_filters.is_empty() {
                    object = object.field("platformFilters", string_array(&platform_filters));
                }
                let build_file = self.graph.add(
                    &format!("bundleBuildFile:{}:{reference}", target.name),
                    object,
                );
                PbxValue::reference(build_file, build_file_comment)
            })
            .collect::<Vec<_>>();

        (!files.is_empty()).then(|| self.add_copy_bundle_resources_phase(target, files))
    }

    fn source_copy_files_phases(
        &mut self,
        target: &Target,
        files: &[FileBuildRefs],
    ) -> Vec<(String, String, CopyFilesPhaseOrder)> {
        let mut buckets =
            BTreeMap::<(i64, String, String, CopyFilesPhaseOrder), Vec<PbxValue>>::new();
        for file in files {
            if !matches!(file.build_phase, Some("CopyFiles") | Some("CopyHeaders")) {
                continue;
            }
            let Some(build_file_id) = file.build_file_id.clone() else {
                continue;
            };
            let settings = file
                .copy_files_settings
                .clone()
                .unwrap_or_else(default_source_copy_files_settings);
            if settings.phase_name == "CopyFiles"
                && settings.dst_path == "$(CONTENTS_FOLDER_PATH)/XPCServices"
                && target.dependencies.iter().any(|dependency| {
                    dependency.dependency_type == DependencyType::Target
                        && self.project.targets.get(&dependency.reference).is_some_and(
                            |dependency_target| {
                                dependency_target.target_type == ProductType::XpcService
                            },
                        )
                })
            {
                continue;
            }
            buckets
                .entry(copy_files_settings_key(&settings))
                .or_default()
                .push(PbxValue::reference(
                    build_file_id,
                    format!("{} in CopyFiles", file.name),
                ));
        }
        buckets
            .into_iter()
            .map(
                |((dst_subfolder_spec, dst_path, phase_name, phase_order), files)| {
                    let phase_id = self.add_copy_files_build_phase_with_name(
                        &format!(
                            "{}:CopyFiles:{}:{}",
                            target.name, dst_subfolder_spec, dst_path
                        ),
                        if phase_name == "CopyFiles" {
                            None
                        } else {
                            Some(phase_name.as_str())
                        },
                        dst_subfolder_spec,
                        &dst_path,
                        false,
                        files,
                    );
                    (phase_id, phase_name, phase_order)
                },
            )
            .collect()
    }

    fn target_dependency_copy_files_phases(
        &mut self,
        target: &Target,
        source_files: &[FileBuildRefs],
    ) -> Vec<(String, String)> {
        let mut buckets = BTreeMap::<(i64, String, String), Vec<PbxValue>>::new();

        for file in source_files {
            let Some(settings) = &file.copy_files_settings else {
                continue;
            };
            if file.build_phase != Some("CopyFiles")
                || settings.phase_name != "CopyFiles"
                || settings.dst_path != "$(CONTENTS_FOLDER_PATH)/XPCServices"
            {
                continue;
            }
            let Some(build_file_id) = file.build_file_id.clone() else {
                continue;
            };
            buckets
                .entry(copy_files_destination_key(settings))
                .or_default()
                .push(PbxValue::reference(
                    build_file_id,
                    format!("{} in CopyFiles", file.name),
                ));
        }

        for dependency in &target.dependencies {
            if !matches!(
                dependency.dependency_type,
                DependencyType::Framework | DependencyType::Sdk { .. }
            ) {
                continue;
            }
            if !should_embed_external_dependency(target, dependency) {
                continue;
            }
            let Some(settings) = copy_files_settings_for_embedded_dependency(dependency) else {
                continue;
            };
            let reference = &dependency.reference;
            let name = display_name(reference);
            let build_file_comment = format!("{name} in {}", settings.phase_name);
            let source_tree = if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                "SDKROOT"
            } else {
                "<group>"
            };
            let path = if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                sdk_reference_path(reference, source_tree)
            } else {
                reference.clone()
            };
            let file_ref = self.add_file_reference(
                &framework_dependency_reference_key(&target.name, dependency, &path),
                name.clone(),
                Some(path),
                Some(file_type_for_path(reference, None)),
                matches!(dependency.dependency_type, DependencyType::Framework)
                    .then_some(name.clone()),
                source_tree,
                true,
            );
            let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone())
                .field("fileRef", PbxValue::reference(file_ref, name.clone()));
            if let Some(build_file_settings) = copy_build_file_settings_for_dependency(dependency) {
                object = object.field("settings", build_file_settings);
            }
            let platform_filters = platform_filters_for_dependency(dependency);
            if !platform_filters.is_empty() {
                object = object.field("platformFilters", string_array(&platform_filters));
            }
            let build_file = self.graph.add(
                &format!("dependencyCopyBuildFile:{}:{reference}", target.name),
                object,
            );
            buckets
                .entry(copy_files_destination_key(&settings))
                .or_default()
                .push(PbxValue::reference(build_file, build_file_comment));
        }

        for dependency in &target.dependencies {
            if dependency.dependency_type != DependencyType::Target {
                continue;
            }
            if let Some((_, external_target_name)) = dependency.reference.split_once('/') {
                let Some(dependency_target) = self.project.targets.get(external_target_name) else {
                    continue;
                };
                if !should_embed_target_dependency(target, dependency, dependency_target) {
                    continue;
                }
                let Some(settings) = copy_files_settings_for_target_dependency(
                    dependency,
                    &dependency_target.target_type,
                ) else {
                    continue;
                };
                let Some(product_ref_id) =
                    self.add_project_reference_product(&dependency.reference)
                else {
                    continue;
                };
                let filename = dependency_target.filename();
                let platform_filters = platform_filters_for_dependency(dependency);
                let build_file_comment = format!("{filename} in {}", settings.phase_name);

                let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone()).field(
                    "fileRef",
                    PbxValue::reference(product_ref_id, filename.clone()),
                );
                if let Some(platform_filter) = dependency_platform_filter(dependency) {
                    object = object.field("platformFilter", PbxValue::String(platform_filter));
                }
                if let Some(build_file_settings) =
                    copy_build_file_settings_for_target_dependency(dependency, dependency_target)
                {
                    object = object.field("settings", build_file_settings);
                }
                if !platform_filters.is_empty() {
                    object = object.field("platformFilters", string_array(&platform_filters));
                }
                let build_file = self.graph.add(
                    &format!(
                        "projectReferenceTargetDependencyCopyBuildFile:{}:{}",
                        target.name, dependency.reference
                    ),
                    object,
                );
                buckets
                    .entry(copy_files_destination_key(&settings))
                    .or_default()
                    .push(PbxValue::reference(build_file, build_file_comment));
                continue;
            }
            let Some(dependency_target) = self.project.targets.get(&dependency.reference) else {
                continue;
            };
            if !should_embed_target_dependency(target, dependency, dependency_target) {
                continue;
            }
            let Some(settings) = copy_files_settings_for_target_dependency(
                dependency,
                &dependency_target.target_type,
            ) else {
                continue;
            };
            let Some(refs) = self.target_refs.get(&dependency.reference) else {
                continue;
            };

            let product_ref_id = refs.product_ref_id.clone();
            let filename = dependency_target.filename();
            let platform_filters = platform_filters_for_dependency(dependency);
            let build_file_comment = format!("{filename} in {}", settings.phase_name);

            let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone()).field(
                "fileRef",
                PbxValue::reference(product_ref_id, filename.clone()),
            );
            if let Some(platform_filter) = dependency_platform_filter(dependency) {
                object = object.field("platformFilter", PbxValue::String(platform_filter));
            }
            if let Some(build_file_settings) =
                copy_build_file_settings_for_target_dependency(dependency, dependency_target)
            {
                object = object.field("settings", build_file_settings);
            }
            if !platform_filters.is_empty() {
                object = object.field("platformFilters", string_array(&platform_filters));
            }
            let build_file = self.graph.add(
                &format!(
                    "targetDependencyCopyBuildFile:{}:{}",
                    target.name, dependency.reference
                ),
                object,
            );
            buckets
                .entry(copy_files_destination_key(&settings))
                .or_default()
                .push(PbxValue::reference(build_file, build_file_comment));
        }

        buckets
            .into_iter()
            .map(|((dst_subfolder_spec, dst_path, phase_name), files)| {
                let run_only_when_installing = target.only_copy_files_on_install
                    && matches!(
                        phase_name.as_str(),
                        "Embed Frameworks" | "Embed System Extensions" | "Embed Dependencies"
                    );
                let phase_id = self.add_copy_files_build_phase_with_name(
                    &format!(
                        "{}:CopyTargetDependencies:{}:{}",
                        target.name, dst_subfolder_spec, dst_path
                    ),
                    if phase_name == "CopyFiles" {
                        None
                    } else {
                        Some(phase_name.as_str())
                    },
                    dst_subfolder_spec,
                    &dst_path,
                    run_only_when_installing,
                    files,
                );
                (phase_id, phase_name)
            })
            .collect()
    }

    fn target_dependency_resource_files(&mut self, target: &Target) -> Vec<PbxValue> {
        let mut files = Vec::new();
        for dependency in &target.dependencies {
            if dependency.dependency_type != DependencyType::Target {
                continue;
            }
            let Some(dependency_target) = self.project.targets.get(&dependency.reference) else {
                continue;
            };
            if dependency_target.target_type != ProductType::MessagesApplication {
                continue;
            }
            let Some(refs) = self.target_refs.get(&dependency.reference) else {
                continue;
            };
            let filename = dependency_target.filename();
            let mut object = PbxObject::new("PBXBuildFile", format!("{filename} in Resources"))
                .field(
                    "fileRef",
                    PbxValue::reference(refs.product_ref_id.clone(), filename.clone()),
                );
            if let Some(build_file_settings) =
                copy_build_file_settings_for_target_dependency(dependency, dependency_target)
            {
                object = object.field("settings", build_file_settings);
            }
            let platform_filters = platform_filters_for_dependency(dependency);
            if !platform_filters.is_empty() {
                object = object.field("platformFilters", string_array(&platform_filters));
            }
            let build_file = self.graph.add(
                &format!(
                    "targetDependencyResourceBuildFile:{}:{}",
                    target.name, dependency.reference
                ),
                object,
            );
            files.push(PbxValue::reference(
                build_file,
                format!("{filename} in Resources"),
            ));
        }
        files
    }

    fn dependency_copy_files_phases(&mut self, target: &Target) -> Vec<(String, String)> {
        let mut buckets = BTreeMap::<(i64, String, String), Vec<PbxValue>>::new();

        if target_directly_embeds_carthage_dependencies(target) {
            for resolved in resolved_carthage_dependencies(self.project, target) {
                let dependency = &resolved.dependency;
                if !should_embed_carthage_dependency(target, dependency) {
                    continue;
                }
                let Some(settings) = copy_files_settings_for_embedded_dependency(dependency) else {
                    continue;
                };
                let name = carthage_framework_name(&dependency.reference);
                let build_file_comment = format!("{name} in {}", settings.phase_name);
                let file_ref = self.add_file_reference(
                    &format!("carthageRef:{}:{}", target.name, dependency.reference),
                    name.clone(),
                    Some(name.clone()),
                    Some(file_type_for_path(&name, None)),
                    None,
                    "<group>",
                    true,
                );
                let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone())
                    .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                if let Some(build_file_settings) =
                    copy_build_file_settings_for_dependency(dependency)
                {
                    object = object.field("settings", build_file_settings);
                }
                let platform_filters = platform_filters_for_dependency(dependency);
                if !platform_filters.is_empty() {
                    object = object.field("platformFilters", string_array(&platform_filters));
                }
                let build_file = self.graph.add(
                    &format!(
                        "carthageCopyBuildFile:{}:{}",
                        target.name, dependency.reference
                    ),
                    object,
                );
                buckets
                    .entry(copy_files_destination_key(&settings))
                    .or_default()
                    .push(PbxValue::reference(build_file, build_file_comment));
            }
        }

        for dependency in &target.dependencies {
            if matches!(&dependency.dependency_type, DependencyType::Carthage { .. }) {
                continue;
            }
            if matches!(
                dependency.dependency_type,
                DependencyType::Framework | DependencyType::Sdk { .. }
            ) {
                continue;
            }
            if !should_embed_external_dependency(target, dependency) {
                continue;
            }
            let Some(settings) = copy_files_settings_for_embedded_dependency(dependency) else {
                continue;
            };
            let platform_filters = platform_filters_for_dependency(dependency);

            match &dependency.dependency_type {
                DependencyType::Framework | DependencyType::Sdk { .. } => {
                    let reference = &dependency.reference;
                    let name = display_name(reference);
                    let build_file_comment = format!("{name} in {}", settings.phase_name);
                    let file_ref = self.add_file_reference(
                        &format!("frameworkRef:{}:{reference}", target.name),
                        name.clone(),
                        Some(reference.clone()),
                        Some(file_type_for_path(reference, None)),
                        None,
                        if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                            "SDKROOT"
                        } else {
                            "<group>"
                        },
                        false,
                    );
                    let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone())
                        .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                    if let Some(build_file_settings) =
                        copy_build_file_settings_for_dependency(dependency)
                    {
                        object = object.field("settings", build_file_settings);
                    }
                    if !platform_filters.is_empty() {
                        object = object.field("platformFilters", string_array(&platform_filters));
                    }
                    let build_file = self.graph.add(
                        &format!("dependencyCopyBuildFile:{}:{reference}", target.name),
                        object,
                    );
                    buckets
                        .entry(copy_files_destination_key(&settings))
                        .or_default()
                        .push(PbxValue::reference(build_file, build_file_comment));
                }
                DependencyType::Package { products } => {
                    let product_names = if products.is_empty() {
                        vec![dependency.reference.clone()]
                    } else {
                        products.clone()
                    };
                    for product_name in product_names {
                        let build_file_comment =
                            format!("{product_name} in {}", settings.phase_name);
                        let product_dependency = self.add_package_product_dependency(
                            &target.name,
                            &dependency.reference,
                            &product_name,
                            false,
                            &platform_filters,
                        );
                        let mut object = PbxObject::new("PBXBuildFile", build_file_comment.clone())
                            .field(
                                "productRef",
                                PbxValue::reference(product_dependency, product_name.clone()),
                            );
                        let mut build_file_settings = BTreeMap::new();
                        build_file_settings.insert(
                            "ATTRIBUTES".to_owned(),
                            PbxValue::Array(vec![
                                PbxValue::String("CodeSignOnCopy".to_owned()),
                                PbxValue::String("RemoveHeadersOnCopy".to_owned()),
                            ]),
                        );
                        object = object.field("settings", PbxValue::Dict(build_file_settings));
                        if !platform_filters.is_empty() {
                            object =
                                object.field("platformFilters", string_array(&platform_filters));
                        }
                        let build_file = self.graph.add(
                            &format!("packageProductCopyBuildFile:{}:{product_name}", target.name),
                            object,
                        );
                        buckets
                            .entry(copy_files_destination_key(&settings))
                            .or_default()
                            .push(PbxValue::reference(build_file, build_file_comment));
                    }
                }
                DependencyType::Target | DependencyType::Bundle => {}
                DependencyType::Carthage { .. } => {}
            }
        }

        let mut buckets = buckets.into_iter().collect::<Vec<_>>();
        buckets.sort_by_key(|((dst_subfolder_spec, dst_path, phase_name), _)| {
            copy_files_phase_output_key(*dst_subfolder_spec, dst_path, phase_name)
        });

        buckets
            .into_iter()
            .map(|((dst_subfolder_spec, dst_path, phase_name), files)| {
                let phase_id = self.add_copy_files_build_phase_with_name(
                    &format!(
                        "{}:CopyDependencies:{}:{}",
                        target.name, dst_subfolder_spec, dst_path
                    ),
                    if phase_name == "CopyFiles" {
                        None
                    } else {
                        Some(phase_name.as_str())
                    },
                    dst_subfolder_spec,
                    &dst_path,
                    target.only_copy_files_on_install,
                    files,
                );
                (phase_id, phase_name)
            })
            .collect()
    }

    fn carthage_copy_frameworks_phase(&mut self, target: &Target) -> Option<String> {
        if target_directly_embeds_carthage_dependencies(target) {
            return None;
        }
        let platform_dir = carthage_platform_dir(&target.platform);
        let base_path = self
            .project
            .spec_options
            .carthage_build_path
            .as_deref()
            .unwrap_or("Carthage/Build");
        let mut input_paths = Vec::new();
        let mut output_paths = Vec::new();

        for resolved in resolved_carthage_dependencies(self.project, target) {
            let dependency = &resolved.dependency;
            if !should_embed_carthage_dependency(target, dependency) {
                continue;
            }
            let name = carthage_framework_name(&dependency.reference);
            let input_path = format!("$(SRCROOT)/{base_path}/{platform_dir}/{name}");
            if !input_paths.contains(&input_path) {
                input_paths.push(input_path);
                output_paths.push(format!(
                    "$(BUILT_PRODUCTS_DIR)/$(FRAMEWORKS_FOLDER_PATH)/{name}"
                ));
            }
        }

        if input_paths.is_empty() {
            return None;
        }

        let executable = self
            .project
            .spec_options
            .carthage_executable_path
            .as_deref()
            .unwrap_or("carthage");
        Some(
            self.graph.add(
                &format!("carthageCopyFrameworks:{}", target.name),
                PbxObject::new("PBXShellScriptBuildPhase", "Carthage")
                    .field("buildActionMask", PbxValue::Int(2147483647))
                    .field("files", PbxValue::Array(Vec::new()))
                    .field("inputFileListPaths", PbxValue::Array(Vec::new()))
                    .field("inputPaths", string_array(&input_paths))
                    .field("name", PbxValue::String("Carthage".to_owned()))
                    .field("outputFileListPaths", PbxValue::Array(Vec::new()))
                    .field("outputPaths", string_array(&output_paths))
                    .field("runOnlyForDeploymentPostprocessing", PbxValue::Int(0))
                    .field("shellPath", PbxValue::String("/bin/sh -l".to_owned()))
                    .field(
                        "shellScript",
                        PbxValue::String(format!("{executable} copy-frameworks\n")),
                    )
                    .field("showEnvVarsInLog", PbxValue::Int(0)),
            ),
        )
    }

    fn add_package_references(&mut self) -> Vec<PbxValue> {
        let mut main_group_children = Vec::new();
        let mut grouped_package_children = BTreeMap::<String, Vec<PbxValue>>::new();
        let mut local_package_file_paths = BTreeSet::<String>::new();
        let mut packages = self.project.packages.iter().collect::<Vec<_>>();
        packages.sort_by_key(|(name, _)| *name);
        for local_packages in [false, true] {
            for (name, package) in packages.iter().copied().filter(|(_, package)| {
                package
                    .get("path")
                    .and_then(|value| value.as_str())
                    .is_some()
                    == local_packages
            }) {
                if let Some(path) = package.get("path").and_then(|value| value.as_str()) {
                    if package
                        .get("excludeFromProject")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let normalized_path = path.trim_start_matches("./");
                    let package_ref = self.graph.add(
                        &format!("xcLocalPackage:{name}:{normalized_path}"),
                        PbxObject::new(
                            "XCLocalSwiftPackageReference",
                            format!("XCLocalSwiftPackageReference \"{normalized_path}\""),
                        )
                        .field("relativePath", PbxValue::String(normalized_path.to_owned())),
                    );
                    self.package_refs.insert(name.clone(), package_ref.clone());
                    self.project_package_refs.push(PbxValue::reference(
                        package_ref,
                        format!("XCLocalSwiftPackageReference \"{normalized_path}\""),
                    ));

                    let file_ref = if local_package_file_paths.insert(normalized_path.to_owned()) {
                        let display_path = display_name(normalized_path);
                        Some(self.add_file_reference(
                            &format!("localPackageFile:{normalized_path}"),
                            display_path.clone(),
                            Some(normalized_path.to_owned()),
                            Some("folder".to_owned()),
                            Some(display_path),
                            "SOURCE_ROOT",
                            true,
                        ))
                    } else {
                        None
                    };
                    let package_group = package
                        .get("group")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .or_else(|| self.project.spec_options.local_packages_group.clone())
                        .unwrap_or_else(|| "Packages".to_owned());
                    if package_group.is_empty() {
                        if let Some(file_ref) = file_ref {
                            main_group_children
                                .push(PbxValue::reference(file_ref, display_name(normalized_path)));
                        }
                    } else if let Some(file_ref) = file_ref {
                        grouped_package_children
                            .entry(package_group)
                            .or_default()
                            .push(PbxValue::reference(file_ref, display_name(normalized_path)));
                    }
                } else if let Some(url) = package_url(package) {
                    let package_comment = package_reference_comment(name, &url);
                    let package_ref = self.graph.add(
                        &format!("xcRemotePackage:{name}:{url}"),
                        PbxObject::new(
                            "XCRemoteSwiftPackageReference",
                            format!("XCRemoteSwiftPackageReference \"{package_comment}\""),
                        )
                        .field("repositoryURL", PbxValue::String(url))
                        .field("requirement", PbxValue::Dict(package_requirement(package))),
                    );
                    self.package_refs.insert(name.clone(), package_ref.clone());
                    self.project_package_refs.push(PbxValue::reference(
                        package_ref,
                        format!("XCRemoteSwiftPackageReference \"{package_comment}\""),
                    ));
                }
            }
        }

        for (group_path, children) in grouped_package_children {
            let (group_id, group_name) = self.add_nested_package_group(&group_path, children);
            main_group_children.push(PbxValue::reference(group_id, group_name));
        }
        main_group_children
    }

    fn add_nested_package_group(
        &mut self,
        group_path: &str,
        leaf_children: Vec<PbxValue>,
    ) -> (String, String) {
        let parts = group_path
            .split('/')
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        let mut children = leaf_children;
        let mut current_id = String::new();
        for index in (0..parts.len()).rev() {
            let name = parts[index].to_owned();
            let path = parts[..=index].join("/");
            let navigator_key = format!("navigatorGroup:{path}");
            let navigator_id = self.graph.id_for(&format!("group:{navigator_key}"));
            let merges_existing_navigator_group = self.graph.objects.contains_key(&navigator_id);
            let key = if merges_existing_navigator_group {
                navigator_key
            } else {
                format!("localPackageGroup:{path}")
            };
            current_id = self.add_group(
                &key,
                (!merges_existing_navigator_group).then_some(name.clone()),
                None,
                children,
            );
            children = vec![PbxValue::reference(current_id.clone(), name)];
        }
        (
            current_id,
            parts.first().copied().unwrap_or(group_path).to_owned(),
        )
    }

    fn add_package_product_dependency(
        &mut self,
        owner_name: &str,
        package_name: &str,
        product_name: &str,
        is_plugin: bool,
        platform_filters: &[String],
    ) -> String {
        let comment = product_name
            .strip_prefix("plugin:")
            .unwrap_or(product_name)
            .to_owned();
        let mut object = PbxObject::new("XCSwiftPackageProductDependency", comment)
            .field("productName", PbxValue::String(product_name.to_owned()));
        let owner_has_supported_destinations = self
            .project
            .targets
            .get(owner_name)
            .is_some_and(|target| !target.supported_destinations.is_empty());
        if product_name == package_name
            && !platform_filters.is_empty()
            && !owner_has_supported_destinations
        {
            object = object.field("platformFilters", string_array(platform_filters));
        }
        let is_local_package = self
            .project
            .packages
            .get(package_name)
            .and_then(|package| package.get("path"))
            .is_some();
        if !is_local_package {
            if let Some(package_ref) = self.package_refs.get(package_name) {
                let comment = self
                    .graph
                    .comments
                    .get(package_ref)
                    .cloned()
                    .unwrap_or_else(|| package_name.to_owned());
                object = object.field("package", PbxValue::reference(package_ref.clone(), comment));
            }
        }
        self.graph.add(
            &format!(
                "packageProduct:{owner_name}:{package_name}:{product_name}:{}",
                if is_plugin { "plugin" } else { "product" }
            ),
            object,
        )
    }

    fn package_product_target_dependency(
        &mut self,
        target_name: &str,
        product_name: &str,
        product_dependency: &str,
        platform_filters: &[String],
    ) -> PbxValue {
        let mut object = PbxObject::new("PBXTargetDependency", "PBXTargetDependency").field(
            "productRef",
            PbxValue::reference(product_dependency.to_owned(), product_name.to_owned()),
        );
        if !platform_filters.is_empty() {
            object = object.field("platformFilters", string_array(platform_filters));
        }
        let dependency = self.graph.add(
            &format!("packageTargetDependency:{target_name}:{product_name}"),
            object,
        );
        PbxValue::reference(dependency, "PBXTargetDependency")
    }

    fn package_plugin_target_dependencies(
        &mut self,
        owner_name: &str,
        plugins: &[crate::spec::BuildToolPlugin],
    ) -> Vec<PbxValue> {
        plugins
            .iter()
            .map(|plugin| {
                let product_dependency = self.add_package_product_dependency(
                    owner_name,
                    &plugin.package,
                    &format!("plugin:{}", plugin.plugin),
                    true,
                    &[],
                );
                self.package_product_target_dependency(
                    owner_name,
                    &plugin.plugin,
                    &product_dependency,
                    &[],
                )
            })
            .collect()
    }

    fn package_product_build_file(
        &mut self,
        target_name: &str,
        product_name: &str,
        product_dependency: &str,
        weak_link: bool,
        platform_filter: Option<String>,
        platform_filters: &[String],
    ) -> PbxValue {
        let mut object = PbxObject::new("PBXBuildFile", format!("{product_name} in Frameworks"))
            .field(
                "productRef",
                PbxValue::reference(product_dependency.to_owned(), product_name.to_owned()),
            );
        if let Some(platform_filter) = platform_filter {
            object = object.field("platformFilter", PbxValue::String(platform_filter));
        }
        if !platform_filters.is_empty() {
            object = object.field("platformFilters", string_array(platform_filters));
        }
        if weak_link {
            let mut settings = BTreeMap::new();
            settings.insert(
                "ATTRIBUTES".to_owned(),
                PbxValue::Array(vec![PbxValue::String("Weak".to_owned())]),
            );
            object = object.field("settings", PbxValue::Dict(settings));
        }
        let build_file = self.graph.add(
            &format!("packageProductBuildFile:{target_name}:{product_name}"),
            object,
        );
        PbxValue::reference(build_file, format!("{product_name} in Frameworks"))
    }

    fn add_product_reference(&mut self, target: &Target) -> String {
        let file_type = file_type_for_path(&target.filename(), Some(&target.target_type));
        let mut object = PbxObject::new("PBXFileReference", target.filename())
            .field("includeInIndex", PbxValue::Int(0));
        if target.target_type != ProductType::CommandLineTool {
            if product_uses_explicit_file_type(target) {
                object = object.field("explicitFileType", PbxValue::String(file_type));
            } else {
                object = object.field("lastKnownFileType", PbxValue::String(file_type));
            }
        }
        object = object
            .field("path", PbxValue::String(target.filename()))
            .field(
                "sourceTree",
                PbxValue::String("BUILT_PRODUCTS_DIR".to_owned()),
            );
        self.graph
            .add(&format!("productRef:{}", target.name), object)
    }

    #[allow(clippy::too_many_arguments)]
    fn add_file_reference(
        &mut self,
        key: &str,
        comment: String,
        path: Option<String>,
        last_known_file_type: Option<String>,
        name: Option<String>,
        source_tree: &str,
        include_in_index: bool,
    ) -> String {
        let mut object = PbxObject::new("PBXFileReference", comment);
        if !include_in_index {
            object = object.field("includeInIndex", PbxValue::Int(0));
        }
        if let Some(file_type) = last_known_file_type {
            object = object.field("lastKnownFileType", PbxValue::String(file_type));
        }
        if let Some(name) = name {
            object = object.field("name", PbxValue::String(name));
        }
        if let Some(path) = path {
            object = object.field("path", PbxValue::String(path));
        }
        object = object.field("sourceTree", PbxValue::String(source_tree.to_owned()));
        let id = self.graph.id_for(key);
        if let Some(existing) = self.graph.objects.get_mut(&id) {
            for (field, value) in object.fields {
                if field == "name"
                    && !existing.fields.get("path").is_some_and(
                        |path| matches!(path, PbxValue::String(value) if value.contains('/')),
                    )
                {
                    continue;
                }
                existing.fields.entry(field).or_insert(value);
            }
            return id;
        }
        self.graph.add(key, object)
    }

    fn add_group(
        &mut self,
        key: &str,
        name: Option<String>,
        path: Option<String>,
        children: Vec<PbxValue>,
    ) -> String {
        let comment = name
            .clone()
            .or_else(|| path.as_deref().map(display_name))
            .unwrap_or_default();
        let children = self.sorted_group_children(&comment, children);
        self.add_group_presorted(key, name, path, children)
    }

    fn add_group_presorted(
        &mut self,
        key: &str,
        name: Option<String>,
        path: Option<String>,
        children: Vec<PbxValue>,
    ) -> String {
        let comment = name
            .clone()
            .or_else(|| path.as_deref().map(display_name))
            .unwrap_or_default();
        let mut object = PbxObject::new("PBXGroup", comment.clone())
            .field("children", PbxValue::Array(children));
        if let Some(name) = name {
            object = object.field("name", PbxValue::String(name));
        }
        if let Some(path) = path {
            object = object.field("path", PbxValue::String(path));
        }
        object = object.field("sourceTree", PbxValue::String("<group>".to_owned()));
        let id = self
            .graph
            .add_or_merge_group(&format!("group:{key}"), object);
        let sorted_children = self
            .graph
            .objects
            .get(&id)
            .and_then(|object| object.fields.get("children"))
            .and_then(|value| match value {
                PbxValue::Array(children) => {
                    Some(self.sorted_group_children(&comment, children.clone()))
                }
                _ => None,
            });
        if let Some(sorted_children) = sorted_children {
            if let Some(object) = self.graph.objects.get_mut(&id) {
                object
                    .fields
                    .insert("children".to_owned(), PbxValue::Array(sorted_children));
            }
        }
        id
    }

    fn sorted_group_children(&self, group_name: &str, children: Vec<PbxValue>) -> Vec<PbxValue> {
        if matches!(group_name, "Products" | "Carthage") {
            return children;
        }
        let use_bottom_sort_for_unpatterned_ordering = !group_name.is_empty()
            && self
                .project
                .spec_options
                .group_ordering
                .iter()
                .any(|ordering| ordering.pattern.is_none())
            && !self
                .project
                .spec_options
                .group_ordering
                .iter()
                .any(|ordering| {
                    ordering.pattern.is_some()
                        && group_ordering_matches(ordering.pattern.as_deref(), group_name)
                });
        let mut children = children;
        let preserve_main_group_order = group_name.is_empty()
            && self
                .project
                .spec_options
                .group_ordering
                .iter()
                .any(|ordering| ordering.pattern.is_none());
        if !preserve_main_group_order {
            children.sort_by(|left, right| {
                let left_order = self.group_child_sort_order_with_position(
                    left,
                    use_bottom_sort_for_unpatterned_ordering,
                );
                let right_order = self.group_child_sort_order_with_position(
                    right,
                    use_bottom_sort_for_unpatterned_ordering,
                );
                left_order
                    .cmp(&right_order)
                    .then_with(|| {
                        natural_group_cmp(
                            &self.group_child_name_path_sort_string(left),
                            &self.group_child_name_path_sort_string(right),
                        )
                    })
                    .then_with(|| pbx_value_id(left).cmp(&pbx_value_id(right)))
            });
        }
        if children
            .iter()
            .any(|child| pbx_value_comment(child) == "Products")
        {
            let mut late = children
                .iter()
                .filter(|child| {
                    matches!(pbx_value_comment(child).as_str(), "Frameworks" | "Products")
                })
                .cloned()
                .collect::<Vec<_>>();
            children.retain(|child| {
                !matches!(pbx_value_comment(child).as_str(), "Frameworks" | "Products")
            });
            children.append(&mut late);
        }
        ordered_group_children(&self.project.spec_options, group_name, children, |child| {
            self.group_child_is_group_or_folder(child)
        })
    }

    fn group_child_sort_order_with_position(&self, child: &PbxValue, force_bottom: bool) -> i32 {
        if !self.group_child_is_group_or_folder(child) {
            return 0;
        }
        match (
            force_bottom,
            effective_group_sort_position(&self.project.spec_options),
        ) {
            (true, _) => 1,
            (_, &GroupSortPosition::Top) => -1,
            (_, &GroupSortPosition::Bottom) => 1,
        }
    }

    fn group_child_is_group_or_folder(&self, child: &PbxValue) -> bool {
        let Some(id) = pbx_value_id(child) else {
            return false;
        };
        let Some(object) = self.graph.objects.get(id) else {
            return false;
        };
        object.isa == "PBXGroup"
            || object.isa == "PBXVariantGroup"
            || object.isa == "XCVersionGroup"
            || object.isa == "PBXFileSystemSynchronizedRootGroup"
            || (object.isa == "PBXFileReference"
                && object
                    .fields
                    .get("lastKnownFileType")
                    .is_some_and(|value| value == &PbxValue::String("folder".to_owned())))
    }

    fn group_child_name_path_sort_string(&self, child: &PbxValue) -> String {
        let Some(id) = pbx_value_id(child) else {
            return pbx_value_comment(child);
        };
        let name = self
            .graph
            .objects
            .get(id)
            .and_then(|object| object.fields.get("name"))
            .and_then(pbx_string)
            .map(str::to_owned);
        let path = self
            .graph
            .objects
            .get(id)
            .and_then(|object| object.fields.get("path"))
            .and_then(pbx_string)
            .map(str::to_owned);
        let name_or_path = name
            .clone()
            .or_else(|| path.clone())
            .unwrap_or_else(|| pbx_value_comment(child));
        format!(
            "{}\t{}\t{}",
            name_or_path,
            name.unwrap_or_default(),
            path.unwrap_or_default()
        )
    }

    fn add_build_phase(
        &mut self,
        isa: &'static str,
        key: &str,
        comment: &str,
        files: Vec<PbxValue>,
    ) -> String {
        self.graph.add(
            &format!("buildPhase:{key}"),
            PbxObject::new(isa, comment.to_owned())
                .field("buildActionMask", PbxValue::Int(2147483647))
                .field("files", PbxValue::Array(files))
                .field("runOnlyForDeploymentPostprocessing", PbxValue::Int(0)),
        )
    }

    fn add_copy_files_build_phase_with_name(
        &mut self,
        key: &str,
        name: Option<&str>,
        dst_subfolder_spec: i64,
        dst_path: &str,
        run_only_for_deployment_postprocessing: bool,
        files: Vec<PbxValue>,
    ) -> String {
        let comment = name.unwrap_or("CopyFiles");
        let mut object = PbxObject::new("PBXCopyFilesBuildPhase", comment.to_owned())
            .field(
                "buildActionMask",
                PbxValue::Int(if run_only_for_deployment_postprocessing {
                    8
                } else {
                    2147483647
                }),
            )
            .field("dstPath", PbxValue::String(dst_path.to_owned()))
            .field("dstSubfolderSpec", PbxValue::Int(dst_subfolder_spec))
            .field("files", PbxValue::Array(files));
        if let Some(name) = name {
            object = object.field("name", PbxValue::String(name.to_owned()));
        }
        object = object.field(
            "runOnlyForDeploymentPostprocessing",
            PbxValue::Int(if run_only_for_deployment_postprocessing {
                1
            } else {
                0
            }),
        );
        self.graph.add(&format!("buildPhase:{key}"), object)
    }

    fn add_copy_bundle_resources_phase(&mut self, target: &Target, files: Vec<PbxValue>) -> String {
        self.graph.add(
            &format!("buildPhase:{}:CopyBundles", target.name),
            PbxObject::new("PBXCopyFilesBuildPhase", "Copy Bundle Resources")
                .field("buildActionMask", PbxValue::Int(2147483647))
                .field("dstSubfolderSpec", PbxValue::Int(7))
                .field("files", PbxValue::Array(files))
                .field("name", PbxValue::String("Copy Bundle Resources".to_owned()))
                .field("runOnlyForDeploymentPostprocessing", PbxValue::Int(0)),
        )
    }

    fn shell_script_phases(
        &mut self,
        target_name: &str,
        scripts: &[BuildScript],
    ) -> Result<Vec<PbxValue>, ProjectWriteError> {
        scripts
            .iter()
            .map(|script| {
                let phase_id = self.add_shell_script_build_phase(target_name, script)?;
                let comment = script
                    .name
                    .clone()
                    .unwrap_or_else(|| "Run Script".to_owned());
                Ok(PbxValue::reference(phase_id, comment))
            })
            .collect()
    }

    fn add_shell_script_build_phase(
        &mut self,
        target_name: &str,
        script: &BuildScript,
    ) -> Result<String, ProjectWriteError> {
        let name = script
            .name
            .clone()
            .unwrap_or_else(|| "Run Script".to_owned());
        let script_text = match &script.script {
            BuildScriptKind::Script(script) => script.clone(),
            BuildScriptKind::Path(path) => {
                let resolved = self.project.base_path.join(path);
                fs::read_to_string(&resolved).map_err(|source| ProjectWriteError::Read {
                    path: resolved,
                    source,
                })?
            }
        };
        let mut object = PbxObject::new("PBXShellScriptBuildPhase", name.clone())
            .field("buildActionMask", PbxValue::Int(2147483647))
            .field("files", PbxValue::Array(Vec::new()))
            .field(
                "inputFileListPaths",
                string_array(&normalize_build_setting_paths(&script.input_file_lists)),
            )
            .field(
                "inputPaths",
                string_array(&normalize_build_setting_paths(&script.input_files)),
            )
            .field(
                "outputFileListPaths",
                string_array(&normalize_build_setting_paths(&script.output_file_lists)),
            )
            .field(
                "outputPaths",
                string_array(&normalize_build_setting_paths(&script.output_files)),
            )
            .field(
                "runOnlyForDeploymentPostprocessing",
                PbxValue::Int(bool_int(script.run_only_when_installing)),
            )
            .field(
                "shellPath",
                PbxValue::String(script.shell.clone().unwrap_or_else(|| "/bin/sh".to_owned())),
            )
            .field("shellScript", PbxValue::String(script_text));
        if script.name.is_some() {
            object = object.field("name", PbxValue::String(name));
        }
        if !script.show_env_vars {
            object = object.field("showEnvVarsInLog", PbxValue::Int(0));
        }
        if !script.based_on_dependency_analysis {
            object = object.field("alwaysOutOfDate", PbxValue::Int(1));
        }
        if let Some(dependency_file) = &script.discovered_dependency_file {
            object = object.field("dependencyFile", PbxValue::String(dependency_file.clone()));
        }
        Ok(self.graph.add(
            &format!("shellScript:{target_name}:{}", build_script_key(script)),
            object,
        ))
    }

    fn add_swift_objc_header_phase(
        &mut self,
        target: &Target,
        files: &[FileBuildRefs],
    ) -> Option<String> {
        if !self.should_copy_swift_objc_header(target, files) {
            return None;
        }
        let input_paths =
            vec!["$(DERIVED_SOURCES_DIR)/$(SWIFT_OBJC_INTERFACE_HEADER_NAME)".to_owned()];
        let output_paths = vec![
            "$(BUILT_PRODUCTS_DIR)/include/$(PRODUCT_MODULE_NAME)/$(SWIFT_OBJC_INTERFACE_HEADER_NAME)"
                .to_owned(),
        ];
        Some(
            self.graph.add(
                &format!("swiftObjCHeader:{}", target.name),
                PbxObject::new(
                    "PBXShellScriptBuildPhase",
                    "Copy Swift Objective-C Interface Header",
                )
                .field("buildActionMask", PbxValue::Int(2147483647))
                .field("files", PbxValue::Array(Vec::new()))
                .field("inputPaths", string_array(&input_paths))
                .field(
                    "name",
                    PbxValue::String("Copy Swift Objective-C Interface Header".to_owned()),
                )
                .field("outputPaths", string_array(&output_paths))
                .field("runOnlyForDeploymentPostprocessing", PbxValue::Int(0))
                .field("shellPath", PbxValue::String("/bin/sh".to_owned()))
                .field(
                    "shellScript",
                    PbxValue::String(
                        "ditto \"${SCRIPT_INPUT_FILE_0}\" \"${SCRIPT_OUTPUT_FILE_0}\"\n".to_owned(),
                    ),
                ),
            ),
        )
    }

    fn should_copy_swift_objc_header(&self, target: &Target, files: &[FileBuildRefs]) -> bool {
        if target.target_type != ProductType::StaticLibrary {
            return false;
        }
        if !files
            .iter()
            .any(|file| file.build_phase == Some("Sources") && file.name.ends_with(".swift"))
        {
            return false;
        }
        let config_name = self
            .config_names()
            .into_iter()
            .next()
            .unwrap_or_else(|| "Debug".to_owned());
        let settings = self.build_settings_for_config(&target.settings_spec, &config_name);
        if settings
            .get("SWIFT_OBJC_INTERFACE_HEADER_NAME")
            .and_then(json_string_value)
            .is_some_and(|value| value.is_empty())
        {
            return false;
        }
        settings
            .get("SWIFT_INSTALL_OBJC_HEADER")
            .and_then(json_bool_value)
            .unwrap_or(true)
    }

    fn add_build_rule(&mut self, target_name: &str, index: usize, rule: &BuildRule) -> String {
        let (file_type, file_patterns) = match &rule.file_type {
            BuildRuleFileType::Type(file_type) => (file_type.clone(), None),
            BuildRuleFileType::Pattern(pattern) => ("pattern.proxy".to_owned(), Some(pattern)),
        };
        let (compiler_spec, script) = match &rule.action {
            BuildRuleAction::CompilerSpec(compiler_spec) => (compiler_spec.clone(), None),
            BuildRuleAction::Script(script) => {
                ("com.apple.compilers.proxy.script".to_owned(), Some(script))
            }
        };
        let mut object = PbxObject::new(
            "PBXBuildRule",
            rule.name.clone().unwrap_or_else(|| "Build Rule".to_owned()),
        )
        .field("compilerSpec", PbxValue::String(compiler_spec))
        .field("fileType", PbxValue::String(file_type))
        .field("isEditable", PbxValue::Int(1))
        .field(
            "name",
            PbxValue::String(rule.name.clone().unwrap_or_else(|| "Build Rule".to_owned())),
        )
        .field("outputFiles", string_array(&rule.output_files))
        .field(
            "outputFilesCompilerFlags",
            string_array(&rule.output_files_compiler_flags),
        )
        .field(
            "runOncePerArchitecture",
            PbxValue::Int(bool_int(rule.run_once_per_architecture)),
        );
        if let Some(file_patterns) = file_patterns {
            object = object.field("filePatterns", PbxValue::String(file_patterns.clone()));
        }
        if let Some(script) = script {
            object = object.field("script", PbxValue::String(script.clone()));
        }
        self.graph
            .add(&format!("buildRule:{target_name}:{index}"), object)
    }

    fn add_configuration_list(
        &mut self,
        owner_type: &str,
        owner_name: &str,
        config_files: BTreeMap<String, Option<String>>,
        build_settings_by_config: BTreeMap<String, BTreeMap<String, PbxValue>>,
    ) -> String {
        let mut config_refs = Vec::new();
        let configs = self.config_names();
        for config_name in configs {
            let mut build_settings = BTreeMap::new();
            if let Some(extra_settings) = build_settings_by_config.get(&config_name) {
                build_settings.extend(extra_settings.clone());
            }
            let mut object = PbxObject::new("XCBuildConfiguration", config_name.clone())
                .field("buildSettings", PbxValue::Dict(build_settings))
                .field("name", PbxValue::String(config_name.clone()));
            if let Some(Some(path)) = config_files.get(&config_name) {
                let (file_path, comment) = split_config_file_path(path).map_or_else(
                    || (path.clone(), display_name(path)),
                    |(_, file_name)| (file_name.clone(), file_name),
                );
                let config_key = if self
                    .project
                    .file_groups
                    .iter()
                    .any(|group| path_is_under_group(path, group))
                {
                    format!("navigatorFileRef:{path}")
                } else {
                    config_file_reference_key(owner_type, owner_name, &config_name, path)
                };
                let config_ref = self.add_file_reference(
                    &config_key,
                    comment.clone(),
                    Some(file_path),
                    Some("text.xcconfig".to_owned()),
                    None,
                    "<group>",
                    true,
                );
                object = object.field(
                    "baseConfigurationReference",
                    PbxValue::reference(config_ref, comment),
                );
            }
            let config_id = self.graph.add(
                &format!("buildConfig:{owner_type}:{owner_name}:{config_name}"),
                object,
            );
            config_refs.push(PbxValue::reference(config_id, config_name));
        }
        self.graph.add(
            &format!("configList:{owner_type}:{owner_name}"),
            PbxObject::new(
                "XCConfigurationList",
                format!("Build configuration list for {owner_type} \"{owner_name}\""),
            )
            .field("buildConfigurations", PbxValue::Array(config_refs))
            .field("defaultConfigurationIsVisible", PbxValue::Int(0))
            .field(
                "defaultConfigurationName",
                PbxValue::String(self.default_configuration_name()),
            ),
        )
    }

    fn build_settings_by_config(
        &self,
        settings: &Settings,
    ) -> BTreeMap<String, BTreeMap<String, PbxValue>> {
        self.config_names()
            .into_iter()
            .map(|config| {
                (
                    config.clone(),
                    self.build_settings_for_config(settings, &config)
                        .into_iter()
                        .map(|(key, value)| (key, pbx_value_from_json(&value)))
                        .collect(),
                )
            })
            .collect()
    }

    fn project_build_settings_by_config(&self) -> BTreeMap<String, BTreeMap<String, PbxValue>> {
        self.config_names()
            .into_iter()
            .map(|config| {
                let mut build_settings = self.project_default_build_settings(&config);
                build_settings.extend(
                    self.build_settings_for_config(&self.project.settings_spec, &config)
                        .into_iter()
                        .map(|(key, value)| (key, pbx_value_from_json(&value))),
                );
                (config.clone(), build_settings)
            })
            .collect()
    }

    fn target_build_settings_by_config(
        &self,
        target: &Target,
    ) -> BTreeMap<String, BTreeMap<String, PbxValue>> {
        let mut settings_by_config = self.build_settings_by_config(&target.settings_spec);
        let info_plists = self.info_plist_files(target);
        for config in self.config_names() {
            let target_default_settings = self.target_default_build_settings(target, &config);
            let settings = settings_by_config.entry(config.clone()).or_default();
            for (key, value) in target_default_settings {
                settings.entry(key).or_insert(value);
            }
            if !settings.contains_key("INFOPLIST_FILE") {
                if let Some(path) = info_plists.iter().next() {
                    settings.insert("INFOPLIST_FILE".to_owned(), PbxValue::String(path.clone()));
                }
            }
            if !settings.contains_key("CODE_SIGN_ENTITLEMENTS") {
                if let Some(path) = target
                    .entitlements_plist
                    .as_ref()
                    .and_then(|plist| plist.path.as_ref())
                {
                    settings.insert(
                        "CODE_SIGN_ENTITLEMENTS".to_owned(),
                        PbxValue::String(path.clone()),
                    );
                }
            }
            if product_type_is_app(&target.target_type)
                && self.target_dependencies_require_objc_linking(target)
                && !settings.contains_key("OTHER_LDFLAGS")
                && !target.config_files.contains_key(&config)
            {
                settings.insert(
                    "OTHER_LDFLAGS".to_owned(),
                    PbxValue::Array(vec![
                        PbxValue::String("$(inherited)".to_owned()),
                        PbxValue::String("-ObjC".to_owned()),
                    ]),
                );
            }
        }
        settings_by_config
    }

    fn target_dependencies_require_objc_linking(&self, target: &Target) -> bool {
        target.dependencies.iter().any(|dependency| {
            dependency.dependency_type == DependencyType::Target
                && dependency.link != Some(false)
                && self.project.targets.get(&dependency.reference).is_some_and(
                    |dependency_target| self.target_requires_objc_linking(dependency_target),
                )
        })
    }

    fn target_requires_objc_linking(&self, target: &Target) -> bool {
        match target.requires_objc_linking {
            Some(value) => value,
            None => {
                target.target_type == ProductType::StaticLibrary
                    || target_sources_require_objc_linking(&self.project.base_path, target)
            }
        }
    }

    fn default_configuration_name(&self) -> String {
        self.project
            .spec_options
            .default_config
            .clone()
            .or_else(|| self.config_names().into_iter().next())
            .unwrap_or_else(|| "Debug".to_owned())
    }

    fn development_language(&self) -> &str {
        self.project
            .spec_options
            .development_language
            .as_deref()
            .unwrap_or("en")
    }

    fn project_default_build_settings(&self, config: &str) -> BTreeMap<String, PbxValue> {
        let mut settings = BTreeMap::new();
        settings.insert(
            "PRODUCT_NAME".to_owned(),
            PbxValue::String("$(TARGET_NAME)".to_owned()),
        );
        if let Some(platform) = self.single_project_platform() {
            settings.insert(
                "SDKROOT".to_owned(),
                PbxValue::String(platform.sdk_root().to_owned()),
            );
        } else if self
            .project
            .targets
            .values()
            .any(|target| matches!(target.platform, Platform::Auto))
        {
            settings.insert("SDKROOT".to_owned(), PbxValue::String("auto".to_owned()));
        }
        insert_deployment_target(
            &mut settings,
            &Platform::Ios,
            self.project.spec_options.deployment_target.ios.as_deref(),
        );
        insert_deployment_target(
            &mut settings,
            &Platform::Macos,
            self.project.spec_options.deployment_target.macos.as_deref(),
        );
        insert_deployment_target(
            &mut settings,
            &Platform::Tvos,
            self.project.spec_options.deployment_target.tvos.as_deref(),
        );
        insert_deployment_target(
            &mut settings,
            &Platform::Watchos,
            self.project
                .spec_options
                .deployment_target
                .watchos
                .as_deref(),
        );
        insert_deployment_target(
            &mut settings,
            &Platform::Visionos,
            self.project
                .spec_options
                .deployment_target
                .visionos
                .as_deref(),
        );
        if !self.project.spec_options.setting_presets_none {
            insert_project_setting_presets(&mut settings, config);
        }
        self.remove_xcconfig_defined_defaults(&mut settings, self.project.config_files.get(config));
        settings
    }

    fn single_project_platform(&self) -> Option<Platform> {
        let mut platforms = self
            .project
            .targets
            .values()
            .filter_map(|target| {
                if matches!(target.platform, Platform::Auto) {
                    None
                } else {
                    Some(target.platform.clone())
                }
            })
            .collect::<BTreeSet<_>>();
        if platforms.len() == 1 {
            platforms.pop_first()
        } else {
            None
        }
    }

    fn target_default_build_settings(
        &self,
        target: &Target,
        config: &str,
    ) -> BTreeMap<String, PbxValue> {
        let mut settings = BTreeMap::new();
        if let Some(prefix) = &self.project.spec_options.bundle_id_prefix {
            settings.insert(
                "PRODUCT_BUNDLE_IDENTIFIER".to_owned(),
                PbxValue::String(format!(
                    "{prefix}.{}",
                    bundle_identifier_suffix(&target.name)
                )),
            );
        }
        if !matches!(target.platform, Platform::Auto) {
            settings.insert(
                "SDKROOT".to_owned(),
                PbxValue::String(target.platform.sdk_root().to_owned()),
            );
            if !self.project.spec_options.setting_presets_none {
                insert_targeted_device_family(&mut settings, &target.platform);
            }
        } else if let Some(supported_platforms) = supported_platforms_setting(target) {
            settings.insert("SDKROOT".to_owned(), PbxValue::String("auto".to_owned()));
            if !self.project.spec_options.setting_presets_none {
                settings.insert(
                    "SUPPORTED_PLATFORMS".to_owned(),
                    PbxValue::String(supported_platforms),
                );
                if let Some(family) = targeted_device_family_for_destinations(target) {
                    settings.insert(
                        "TARGETED_DEVICE_FAMILY".to_owned(),
                        PbxValue::String(family),
                    );
                }
                insert_supported_destination_presets(&mut settings, target);
            }
        }
        if let Some(deployment_target) = target.deployment_target.as_deref() {
            insert_deployment_target(&mut settings, &target.platform, Some(deployment_target));
        }
        if let Some(search_paths) =
            carthage_framework_search_paths(&self.project.spec_options, target)
        {
            settings.insert("FRAMEWORK_SEARCH_PATHS".to_owned(), search_paths);
        } else if let Some(search_paths) = framework_dependency_search_paths(target) {
            settings.insert("FRAMEWORK_SEARCH_PATHS".to_owned(), search_paths);
        }
        if !self.project.spec_options.setting_presets_none {
            insert_target_setting_presets(&mut settings, target, config, self.project);
        }
        self.remove_target_xcconfig_defined_defaults(
            &mut settings,
            self.project.config_files.get(config),
        );
        self.remove_target_xcconfig_defined_defaults(
            &mut settings,
            target.config_files.get(config),
        );
        settings
    }

    fn remove_xcconfig_defined_defaults(
        &self,
        settings: &mut BTreeMap<String, PbxValue>,
        config_file: Option<&String>,
    ) {
        let Some(config_file) = config_file else {
            return;
        };
        for key in self.xcconfig_defined_keys(config_file) {
            settings.remove(&key);
        }
    }

    fn remove_target_xcconfig_defined_defaults(
        &self,
        settings: &mut BTreeMap<String, PbxValue>,
        config_file: Option<&String>,
    ) {
        let Some(config_file) = config_file else {
            return;
        };
        for key in self.xcconfig_defined_keys(config_file) {
            if target_xcconfig_preserves_default(&key) {
                continue;
            }
            settings.remove(&key);
        }
    }

    fn xcconfig_defined_keys(&self, config_file: &str) -> HashSet<String> {
        let mut keys = HashSet::new();
        let mut seen = HashSet::new();
        self.collect_xcconfig_defined_keys(
            &self.project.base_path.join(config_file),
            &mut seen,
            &mut keys,
        );
        keys
    }

    fn collect_xcconfig_defined_keys(
        &self,
        path: &Path,
        seen: &mut HashSet<PathBuf>,
        keys: &mut HashSet<String>,
    ) {
        if !seen.insert(path.to_path_buf()) {
            return;
        }
        let Ok(contents) = fs::read_to_string(path) else {
            return;
        };
        for line in contents.lines() {
            let line = line
                .split_once("//")
                .map_or(line, |(prefix, _)| prefix)
                .trim();
            if line.is_empty() {
                continue;
            }
            if let Some(include) = xcconfig_include_path(line) {
                let parent_candidate = path
                    .parent()
                    .unwrap_or(&self.project.base_path)
                    .join(&include);
                let include_path = if parent_candidate.exists() {
                    parent_candidate
                } else {
                    self.project.base_path.join(include)
                };
                self.collect_xcconfig_defined_keys(&include_path, seen, keys);
                continue;
            }
            if let Some((key, _)) = line.split_once('=') {
                let key = key
                    .trim()
                    .split_once('[')
                    .map_or_else(|| key.trim(), |(base, _)| base.trim());
                if !key.is_empty() {
                    keys.insert(key.to_owned());
                }
            }
        }
    }

    fn build_settings_for_config(
        &self,
        settings: &Settings,
        config: &str,
    ) -> BTreeMap<String, Value> {
        let mut build_settings = BTreeMap::new();
        for group in &settings.groups {
            if let Some(group_settings) = self.project.setting_group_specs.get(group) {
                build_settings.extend(self.build_settings_for_config(group_settings, config));
            }
        }
        build_settings.extend(
            settings
                .build_settings
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        for (variant, variant_settings) in &settings.config_settings {
            if config_matches_variant(config, variant, &self.project.configs) {
                build_settings.extend(self.build_settings_for_config(variant_settings, config));
            }
        }
        build_settings
    }

    fn config_names(&self) -> Vec<String> {
        if self.project.configs.is_empty() {
            vec!["Debug".to_owned(), "Release".to_owned()]
        } else {
            ordered_config_names(&self.project.configs)
        }
    }

    fn info_plist_files(&self, target: &Target) -> BTreeSet<String> {
        let mut files = BTreeSet::new();
        for config in self.config_names() {
            let settings = self.build_settings_for_config(&target.settings_spec, &config);
            if let Some(path) = settings.get("INFOPLIST_FILE").and_then(json_string_value) {
                if is_static_plist_path(&path) {
                    files.insert(path);
                }
                continue;
            }
            if let Some(path) = target
                .info_plist
                .as_ref()
                .and_then(|plist| plist.path.clone())
            {
                files.insert(path);
                continue;
            }
            if let Some(path) = default_info_plist_from_sources(&self.project.base_path, target) {
                files.insert(path);
            }
        }
        files
    }

    fn serialize_with_id_map(&self, root_id: &str) -> (String, HashMap<String, String>) {
        let id_map = self.xcode_reference_map(root_id);
        let mut output = String::new();
        output.push_str("// !$*UTF8*$!\n{\n");
        output.push_str(
            "\tarchiveVersion = 1;\n\tclasses = {\n\t};\n\tobjectVersion = 77;\n\tobjects = {\n\n",
        );

        let mut sections: BTreeMap<&str, Vec<(&String, &PbxObject)>> = BTreeMap::new();
        for (id, object) in &self.graph.objects {
            sections.entry(object.isa).or_default().push((id, object));
        }

        let section_count = sections.len();
        for (section_index, (isa, mut objects)) in sections.into_iter().enumerate() {
            objects.sort_by_key(|(id, _)| mapped_id(id, &id_map));
            let _ = writeln!(output, "/* Begin {isa} section */");
            for (id, object) in objects {
                let output_id = mapped_id(id, &id_map);
                let _ = write!(output, "\t\t{output_id}");
                if let Some(comment) = &object.comment {
                    if !comment.is_empty() {
                        let _ = write!(output, " /* {comment} */");
                    }
                }
                if object.isa == "PBXBuildFile" || object.isa == "PBXFileReference" {
                    output.push_str(" = {isa = ");
                    output.push_str(isa);
                    output.push_str("; ");
                    for (key, value) in &object.fields {
                        let _ = write!(output, "{key} = ");
                        write_compact_value(value, &mut output, &self.graph.comments, &id_map, key);
                        output.push_str("; ");
                    }
                    output.push_str("};\n");
                } else {
                    output.push_str(" = {\n");
                    let _ = writeln!(output, "\t\t\tisa = {isa};");
                    for (key, value) in &object.fields {
                        let _ = write!(output, "\t\t\t{key} = ");
                        if key == "remoteGlobalIDString" {
                            if let PbxValue::String(value) = value {
                                output.push_str(&mapped_id(value, &id_map));
                            } else {
                                value.write(&mut output, 3, &self.graph.comments, &id_map);
                            }
                        } else {
                            value.write(&mut output, 3, &self.graph.comments, &id_map);
                        }
                        output.push_str(";\n");
                    }
                    output.push_str("\t\t};\n");
                }
            }
            let _ = writeln!(output, "/* End {isa} section */");
            if section_index + 1 < section_count {
                output.push('\n');
            }
        }

        output.push_str("\t};\n");
        let _ = writeln!(
            output,
            "\trootObject = {} /* Project object */;\n}}",
            mapped_id(root_id, &id_map)
        );
        (output, id_map)
    }

    fn xcode_reference_map(&self, root_id: &str) -> HashMap<String, String> {
        let mut generator = XcodeReferenceGenerator::new(self);
        generator.generate(root_id);
        generator.state.output
    }
}

fn write_plists(project: &Project) -> Result<(), ProjectWriteError> {
    for target in project.targets.values() {
        if let Some(plist) = &target.info_plist {
            if let Some(path) = &plist.path {
                write_plist(project, path, info_plist_properties(target, plist))?;
            }
        }
        if let Some(plist) = &target.entitlements_plist {
            if let Some(path) = &plist.path {
                write_plist(project, path, plist.attributes.clone())?;
            }
        }
    }
    Ok(())
}

fn write_plist(
    project: &Project,
    relative_path: &str,
    properties: indexmap::IndexMap<String, Value>,
) -> Result<(), ProjectWriteError> {
    let path = project.base_path.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectWriteError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    fs::write(&path, plist_xml(&properties))
        .map_err(|source| ProjectWriteError::Write { path, source })
}

fn write_schemes(
    project: &Project,
    project_path: &Path,
    object_id_map: &HashMap<String, String>,
) -> Result<(), ProjectWriteError> {
    let schemes_dir = project_path.join("xcshareddata/xcschemes");
    let mut schemes = Vec::new();
    for scheme in project.scheme_specs.values() {
        if scheme.management.shared {
            schemes.push((
                scheme.name.clone(),
                scheme_xml(project, scheme, object_id_map),
            ));
        }
    }
    for target in project.targets.values() {
        let Some(target_scheme) = &target.target_scheme else {
            continue;
        };
        let variants = if target_scheme.config_variants.is_empty() {
            vec![None]
        } else {
            target_scheme
                .config_variants
                .iter()
                .map(|variant| Some(variant.as_str()))
                .collect::<Vec<_>>()
        };
        for variant in variants {
            let name = variant
                .map(|variant| format!("{} {variant}", target.name))
                .unwrap_or_else(|| target.name.clone());
            schemes.push((
                name.clone(),
                target_scheme_xml(project, target, target_scheme, variant, object_id_map),
            ));
        }
    }
    if schemes.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(&schemes_dir).map_err(|source| ProjectWriteError::Write {
        path: schemes_dir.clone(),
        source,
    })?;
    for (name, xml) in schemes {
        let path = schemes_dir.join(format!("{name}.xcscheme"));
        fs::write(&path, xml).map_err(|source| ProjectWriteError::Write { path, source })?;
    }
    Ok(())
}

fn write_scheme_management(
    project: &Project,
    project_path: &Path,
) -> Result<(), ProjectWriteError> {
    let states = scheme_management_states(project);
    if states.is_empty() {
        return Ok(());
    }
    let schemes_dir = project_path.join("xcuserdata/xcodegenrust.xcuserdatad/xcschemes");
    fs::create_dir_all(&schemes_dir).map_err(|source| ProjectWriteError::Write {
        path: schemes_dir.clone(),
        source,
    })?;
    let path = schemes_dir.join("xcschememanagement.plist");
    fs::write(&path, scheme_management_plist(&states))
        .map_err(|source| ProjectWriteError::Write { path, source })
}

fn write_breakpoints(project: &Project, project_path: &Path) -> Result<(), ProjectWriteError> {
    if project.breakpoints.is_empty() {
        return Ok(());
    }
    let debugger_dir = project_path.join("xcshareddata/xcdebugger");
    fs::create_dir_all(&debugger_dir).map_err(|source| ProjectWriteError::Write {
        path: debugger_dir.clone(),
        source,
    })?;
    let path = debugger_dir.join("Breakpoints_v2.xcbkptlist");
    fs::write(&path, breakpoints_xml(&project.breakpoints))
        .map_err(|source| ProjectWriteError::Write { path, source })
}

fn scheme_management_states(project: &Project) -> Vec<SchemeManagementState> {
    let mut states = Vec::new();
    for scheme in project.scheme_specs.values() {
        if scheme.management.is_shown.is_some() || scheme.management.order_hint.is_some() {
            states.push(SchemeManagementState {
                name: scheme.name.clone(),
                shared: scheme.management.shared,
                is_shown: scheme.management.is_shown,
                order_hint: scheme.management.order_hint,
            });
        }
    }
    for target in project.targets.values() {
        let Some(target_scheme) = &target.target_scheme else {
            continue;
        };
        let Some(management) = &target_scheme.management else {
            continue;
        };
        let variants = if target_scheme.config_variants.is_empty() {
            vec![None]
        } else {
            target_scheme
                .config_variants
                .iter()
                .map(|variant| Some(variant.as_str()))
                .collect::<Vec<_>>()
        };
        for variant in variants {
            states.push(SchemeManagementState {
                name: variant
                    .map(|variant| format!("{} {variant}", target.name))
                    .unwrap_or_else(|| target.name.clone()),
                shared: management.shared,
                is_shown: management.is_shown,
                order_hint: management.order_hint,
            });
        }
    }
    states
}

fn scheme_management_plist(states: &[SchemeManagementState]) -> String {
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ");
    output.push_str("\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    output.push_str("<plist version=\"1.0\">\n<dict>\n");
    output.push_str("\t<key>SchemeUserState</key>\n\t<dict>\n");
    for state in states {
        let suffix = if state.shared { "_^#shared#^_" } else { "" };
        let _ = writeln!(
            output,
            "\t\t<key>{}.xcscheme{}</key>",
            xml_escape(&state.name),
            suffix
        );
        output.push_str("\t\t<dict>\n");
        if let Some(is_shown) = state.is_shown {
            output.push_str("\t\t\t<key>isShown</key>\n");
            let _ = writeln!(
                output,
                "\t\t\t<{} />",
                if is_shown { "true" } else { "false" }
            );
        }
        if let Some(order_hint) = state.order_hint {
            output.push_str("\t\t\t<key>orderHint</key>\n");
            let _ = writeln!(output, "\t\t\t<integer>{order_hint}</integer>");
        }
        output.push_str("\t\t</dict>\n");
    }
    output.push_str("\t</dict>\n</dict>\n</plist>\n");
    output
}

fn scheme_xml(
    project: &Project,
    scheme: &crate::spec::Scheme,
    object_id_map: &HashMap<String, String>,
) -> String {
    let debug_config = default_config_for(project, "debug");
    let release_config = default_config_for(project, "release");
    let runnable = first_runnable_scheme_target(project, &scheme.build.targets);
    let primary_build_target = scheme
        .build
        .targets
        .first()
        .map(|target| target.target.as_str());
    let default_macro_expansion = runnable.is_none().then_some(()).and(primary_build_target);
    let run_macro_expansion = scheme
        .run
        .as_ref()
        .and_then(|run| run.macro_expansion.as_deref())
        .or(default_macro_expansion);
    let testing_macro_expansion =
        first_testing_runnable_scheme_target(project, &scheme.build.targets);
    let test_macro_expansion = scheme
        .test
        .as_ref()
        .and_then(|test| test.macro_expansion.as_deref())
        .or(testing_macro_expansion)
        .or(run_macro_expansion)
        .or(primary_build_target);
    let empty_command_line_arguments = indexmap::IndexMap::new();
    let emit_empty_test_command_line_arguments = scheme.test.is_some();
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let mut scheme_attrs = vec![(
        "LastUpgradeVersion",
        xml_escape(&scheme_last_upgrade_version(project)),
    )];
    if primary_build_target
        .and_then(|target| project.targets.get(target))
        .is_some_and(|target| scheme_target_is_app_extension(&target.target_type))
    {
        scheme_attrs.push(("wasCreatedForAppExtension", "YES".to_owned()));
    }
    scheme_attrs.push(("version", "1.7".to_owned()));
    write_multiline_start(&mut output, 0, "Scheme", &scheme_attrs);
    write_build_action(&mut output, project, &scheme.build, object_id_map, 1);
    write_test_action(
        &mut output,
        project,
        scheme.test.as_ref().and_then(|test| test.config.as_deref()),
        scheme
            .test
            .as_ref()
            .map(|test| test.gather_coverage_data)
            .unwrap_or(false),
        scheme
            .test
            .as_ref()
            .map(|test| test.debug_enabled)
            .unwrap_or(true),
        scheme
            .test
            .as_ref()
            .map(|test| test.disable_main_thread_checker)
            .unwrap_or(false),
        scheme
            .test
            .as_ref()
            .map(|test| test.stop_on_every_main_thread_checker_issue)
            .unwrap_or(false),
        scheme
            .test
            .as_ref()
            .and_then(|test| test.custom_lldb_init.as_deref()),
        scheme
            .test
            .as_ref()
            .map(|test| test.pre_actions.as_slice())
            .unwrap_or(&[]),
        scheme
            .test
            .as_ref()
            .map(|test| test.post_actions.as_slice())
            .unwrap_or(&[]),
        test_macro_expansion,
        scheme
            .test
            .as_ref()
            .map(|test| test.targets.as_slice())
            .unwrap_or(&[]),
        scheme
            .test
            .as_ref()
            .map(|test| test.coverage_targets.as_slice())
            .unwrap_or(&[]),
        scheme
            .test
            .as_ref()
            .map(|test| test.test_plans.as_slice())
            .unwrap_or(&[]),
        scheme
            .test
            .as_ref()
            .map(|test| test.environment_variables.as_slice())
            .unwrap_or(&[]),
        scheme
            .test
            .as_ref()
            .map(|test| &test.command_line_arguments)
            .unwrap_or(&empty_command_line_arguments),
        emit_empty_test_command_line_arguments,
        scheme
            .test
            .as_ref()
            .and_then(|test| test.capture_screenshots_automatically),
        scheme
            .test
            .as_ref()
            .and_then(|test| test.delete_screenshots_when_each_test_succeeds),
        scheme
            .test
            .as_ref()
            .and_then(|test| test.preferred_screen_capture_format.as_deref()),
        &debug_config,
        object_id_map,
        1,
    );
    write_launch_action(
        &mut output,
        project,
        runnable,
        scheme.run.as_ref().and_then(|run| run.config.as_deref()),
        scheme
            .run
            .as_ref()
            .map(|run| run.debug_enabled)
            .unwrap_or(true),
        scheme
            .run
            .as_ref()
            .and_then(|run| run.launch_automatically_substyle.as_deref()),
        scheme
            .run
            .as_ref()
            .map(|run| run.ask_for_app_to_launch)
            .unwrap_or(false),
        scheme
            .run
            .as_ref()
            .and_then(|run| run.custom_lldb_init.as_deref()),
        scheme
            .run
            .as_ref()
            .and_then(|run| run.custom_working_directory.as_deref()),
        scheme
            .run
            .as_ref()
            .and_then(|run| run.enable_gpu_frame_capture_mode.as_deref()),
        scheme
            .run
            .as_ref()
            .and_then(|run| run.store_kit_configuration.as_deref()),
        run_macro_expansion,
        scheme
            .run
            .as_ref()
            .and_then(|run| run.simulate_location.as_ref()),
        scheme
            .run
            .as_ref()
            .map(|run| run.disable_main_thread_checker)
            .unwrap_or(false),
        scheme
            .run
            .as_ref()
            .map(|run| run.stop_on_every_main_thread_checker_issue)
            .unwrap_or(false),
        scheme
            .run
            .as_ref()
            .map(|run| run.disable_thread_performance_checker)
            .unwrap_or(false),
        scheme
            .run
            .as_ref()
            .map(|run| &run.command_line_arguments)
            .unwrap_or(&empty_command_line_arguments),
        scheme.run.is_some(),
        scheme.run.as_ref().and_then(|run| run.language.as_deref()),
        scheme.run.as_ref().and_then(|run| run.region.as_deref()),
        scheme
            .run
            .as_ref()
            .map(|run| run.environment_variables.as_slice())
            .unwrap_or(&[]),
        &debug_config,
        object_id_map,
        1,
    );
    write_profile_action(
        &mut output,
        project,
        runnable,
        scheme
            .profile
            .as_ref()
            .and_then(|profile| profile.config.as_deref()),
        scheme
            .profile
            .as_ref()
            .map(|profile| profile.environment_variables.as_slice())
            .unwrap_or(&[]),
        scheme
            .profile
            .as_ref()
            .map(|profile| profile.ask_for_app_to_launch)
            .unwrap_or(false),
        scheme.profile.is_some(),
        default_macro_expansion,
        &release_config,
        object_id_map,
        1,
    );
    write_simple_action(
        &mut output,
        "AnalyzeAction",
        "buildConfiguration",
        scheme
            .analyze
            .as_ref()
            .and_then(|analyze| analyze.config.as_deref())
            .unwrap_or(&debug_config),
        1,
    );
    write_simple_action(
        &mut output,
        "ArchiveAction",
        "buildConfiguration",
        scheme
            .archive
            .as_ref()
            .and_then(|archive| archive.config.as_deref())
            .unwrap_or(&release_config),
        1,
    );
    output.push_str("</Scheme>\n");
    output
}

fn target_scheme_xml(
    project: &Project,
    target: &Target,
    scheme: &crate::spec::TargetScheme,
    variant: Option<&str>,
    object_id_map: &HashMap<String, String>,
) -> String {
    let debug_config = variant_config(project, variant, "debug");
    let release_config = variant_config(project, variant, "release");
    let mut build_targets = vec![crate::spec::SchemeBuildTarget {
        target: target.name.clone(),
        build_types: vec![
            crate::spec::BuildType::Running,
            crate::spec::BuildType::Testing,
            crate::spec::BuildType::Profiling,
            crate::spec::BuildType::Analyzing,
            crate::spec::BuildType::Archiving,
        ],
    }];
    if is_watch_app_product(&target.target_type) {
        if let Some(host) = project.targets.values().find(|candidate| {
            candidate.dependencies.iter().any(|dependency| {
                dependency.dependency_type == DependencyType::Target
                    && dependency.reference == target.name
            })
        }) {
            build_targets.push(crate::spec::SchemeBuildTarget {
                target: host.name.clone(),
                build_types: vec![
                    crate::spec::BuildType::Running,
                    crate::spec::BuildType::Testing,
                    crate::spec::BuildType::Profiling,
                    crate::spec::BuildType::Analyzing,
                    crate::spec::BuildType::Archiving,
                ],
            });
        }
    }
    let build = crate::spec::SchemeBuild {
        targets: build_targets,
        pre_actions: scheme.pre_actions.clone(),
        post_actions: scheme.post_actions.clone(),
        ..Default::default()
    };
    let test_targets = scheme.test_target_options.clone();
    let runnable = runnable_scheme_target(target).then_some(target.name.as_str());
    let macro_expansion = (!runnable_scheme_target(target)).then_some(target.name.as_str());
    let empty_command_line_arguments = indexmap::IndexMap::new();
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let mut scheme_attrs = vec![(
        "LastUpgradeVersion",
        xml_escape(&scheme_last_upgrade_version(project)),
    )];
    if matches!(
        target.target_type,
        ProductType::AppExtension
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::MessagesExtension
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
    ) {
        scheme_attrs.push(("wasCreatedForAppExtension", "YES".to_owned()));
    }
    scheme_attrs.push(("version", "1.7".to_owned()));
    write_multiline_start(&mut output, 0, "Scheme", &scheme_attrs);
    write_build_action(&mut output, project, &build, object_id_map, 1);
    write_test_action(
        &mut output,
        project,
        Some(&debug_config),
        scheme.gather_coverage_data,
        true,
        scheme.disable_main_thread_checker,
        scheme.stop_on_every_main_thread_checker_issue,
        None,
        &[],
        &[],
        Some(&target.name),
        &test_targets,
        &scheme.coverage_targets,
        &scheme.test_plans,
        &scheme.environment_variables,
        &empty_command_line_arguments,
        true,
        None,
        None,
        None,
        &debug_config,
        object_id_map,
        1,
    );
    write_launch_action(
        &mut output,
        project,
        runnable,
        Some(&debug_config),
        true,
        None,
        false,
        None,
        None,
        None,
        scheme.store_kit_configuration.as_deref(),
        macro_expansion,
        None,
        scheme.disable_main_thread_checker,
        scheme.stop_on_every_main_thread_checker_issue,
        scheme.disable_thread_performance_checker,
        &scheme.command_line_arguments,
        scheme.gather_coverage_data,
        scheme.language.as_deref(),
        scheme.region.as_deref(),
        &scheme.environment_variables,
        &debug_config,
        object_id_map,
        1,
    );
    write_profile_action(
        &mut output,
        project,
        runnable,
        Some(&release_config),
        &scheme.environment_variables,
        false,
        scheme.gather_coverage_data,
        macro_expansion,
        &release_config,
        object_id_map,
        1,
    );
    write_simple_action(
        &mut output,
        "AnalyzeAction",
        "buildConfiguration",
        &debug_config,
        1,
    );
    write_simple_action(
        &mut output,
        "ArchiveAction",
        "buildConfiguration",
        &release_config,
        1,
    );
    output.push_str("</Scheme>\n");
    output
}

fn write_build_action(
    output: &mut String,
    project: &Project,
    build: &crate::spec::SchemeBuild,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    write_multiline_start(
        output,
        indent,
        "BuildAction",
        &[
            (
                "parallelizeBuildables",
                bool_xml(build.parallelize_build).to_owned(),
            ),
            (
                "buildImplicitDependencies",
                bool_xml(build.build_implicit_dependencies).to_owned(),
            ),
            (
                "runPostActionsOnFailure",
                bool_xml(build.run_post_actions_on_failure).to_owned(),
            ),
        ],
    );
    for action in &build.pre_actions {
        write_execution_action(
            output,
            project,
            "PreActions",
            action,
            object_id_map,
            indent + 1,
        );
    }
    write_indent(output, indent + 1);
    output.push_str("<BuildActionEntries>\n");
    for target in &build.targets {
        if let Some(project_target) = project.targets.get(&target.target) {
            write_build_action_entry_start(output, indent + 2, &target.build_types);
            write_buildable_reference(output, project, project_target, object_id_map, indent + 3);
            write_indent(output, indent + 2);
            output.push_str("</BuildActionEntry>\n");
        } else if let Some((target_name, container)) =
            project_reference_target(project, &target.target)
        {
            write_build_action_entry_start(output, indent + 2, &target.build_types);
            write_external_buildable_reference(output, target_name, container, indent + 3);
            write_indent(output, indent + 2);
            output.push_str("</BuildActionEntry>\n");
        }
    }
    write_indent(output, indent + 1);
    output.push_str("</BuildActionEntries>\n");
    for action in &build.post_actions {
        write_execution_action(
            output,
            project,
            "PostActions",
            action,
            object_id_map,
            indent + 1,
        );
    }
    write_indent(output, indent);
    output.push_str("</BuildAction>\n");
}

#[allow(clippy::too_many_arguments)]
fn write_test_action(
    output: &mut String,
    project: &Project,
    config: Option<&str>,
    gather_coverage_data: bool,
    debug_enabled: bool,
    disable_main_thread_checker: bool,
    stop_on_every_main_thread_checker_issue: bool,
    custom_lldb_init: Option<&str>,
    pre_actions: &[crate::spec::SchemeAction],
    post_actions: &[crate::spec::SchemeAction],
    macro_expansion: Option<&str>,
    test_targets: &[crate::spec::SchemeTestTarget],
    coverage_targets: &[String],
    test_plans: &[crate::spec::TestPlan],
    environment_variables: &[crate::spec::EnvironmentVariable],
    command_line_arguments: &indexmap::IndexMap<String, bool>,
    emit_empty_command_line_arguments: bool,
    capture_screenshots_automatically: Option<bool>,
    delete_screenshots_when_each_test_succeeds: Option<bool>,
    preferred_screen_capture_format: Option<&str>,
    default_config: &str,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    let system_attachment_lifetime = match (
        capture_screenshots_automatically,
        delete_screenshots_when_each_test_succeeds,
    ) {
        (Some(false), _) => Some("keepNever"),
        (Some(true), Some(false)) => Some("keepAlways"),
        _ => None,
    };
    let mut attrs = vec![
        (
            "buildConfiguration",
            xml_escape(config.unwrap_or(default_config)),
        ),
        (
            "selectedDebuggerIdentifier",
            if debug_enabled {
                "Xcode.DebuggerFoundation.Debugger.LLDB"
            } else {
                ""
            }
            .to_owned(),
        ),
        (
            "selectedLauncherIdentifier",
            if debug_enabled {
                "Xcode.DebuggerFoundation.Launcher.LLDB"
            } else {
                "Xcode.IDEFoundation.Launcher.PosixSpawn"
            }
            .to_owned(),
        ),
        (
            "shouldUseLaunchSchemeArgsEnv",
            if command_line_arguments.is_empty() && environment_variables.is_empty() {
                "YES"
            } else {
                "NO"
            }
            .to_owned(),
        ),
    ];
    if gather_coverage_data {
        if disable_main_thread_checker {
            attrs.push(("disableMainThreadChecker", "YES".to_owned()));
        }
        attrs.push(("codeCoverageEnabled", "YES".to_owned()));
    } else if disable_main_thread_checker {
        attrs.push(("disableMainThreadChecker", "YES".to_owned()));
    }
    if !coverage_targets.is_empty() && gather_coverage_data {
        attrs.push(("onlyGenerateCoverageForSpecifiedTargets", "YES".to_owned()));
    } else {
        attrs.push(("onlyGenerateCoverageForSpecifiedTargets", "NO".to_owned()));
    }
    if stop_on_every_main_thread_checker_issue {
        attrs.push(("stopOnEveryMainThreadCheckerIssue", "YES".to_owned()));
    }
    if let Some(value) = system_attachment_lifetime {
        attrs.push(("systemAttachmentLifetime", value.to_owned()));
    }
    if let Some(value) = preferred_screen_capture_format {
        attrs.push(("preferredScreenCaptureFormat", xml_escape(value)));
    }
    if let Some(value) = custom_lldb_init {
        attrs.push(("customLLDBInitFile", xml_escape(value)));
    }
    write_multiline_start(output, indent, "TestAction", &attrs);
    for action in pre_actions {
        write_execution_action(
            output,
            project,
            "PreActions",
            action,
            object_id_map,
            indent + 1,
        );
    }
    write_macro_expansion(output, project, macro_expansion, object_id_map, indent + 1);
    write_indent(output, indent + 1);
    output.push_str("<Testables>\n");
    for test_target in test_targets {
        let target_name = test_target
            .target_reference
            .split('/')
            .next_back()
            .unwrap_or(&test_target.target_reference);
        if project.targets.contains_key(target_name)
            || package_test_reference(project, &test_target.target_reference).is_some()
        {
            let mut attrs = vec![
                ("skipped", bool_xml(test_target.skipped).to_owned()),
                (
                    "parallelizable",
                    bool_xml(test_target.parallelizable).to_owned(),
                ),
            ];
            if test_target.random_execution_order {
                attrs.push(("testExecutionOrdering", "random".to_owned()));
            }
            write_multiline_start(output, indent + 2, "TestableReference", &attrs);
            if let Some(target) = project.targets.get(target_name) {
                write_buildable_reference(output, project, target, object_id_map, indent + 3);
            } else {
                write_package_test_buildable_reference(
                    output,
                    project,
                    &test_target.target_reference,
                    indent + 3,
                );
            }
            if let Some(location) = &test_target.location {
                write_indent(output, indent + 3);
                let is_gpx = location.ends_with(".gpx");
                let identifier = if is_gpx {
                    scheme_prefixed_path(project, location)
                } else {
                    location.clone()
                };
                let _ = writeln!(
                    output,
                    "<LocationScenarioReference identifier=\"{}\" referenceType=\"{}\"/>",
                    xml_escape(&identifier),
                    if is_gpx { "0" } else { "1" }
                );
            }
            write_indent(output, indent + 2);
            output.push_str("</TestableReference>\n");
        }
    }
    write_indent(output, indent + 1);
    output.push_str("</Testables>\n");
    if !test_plans.is_empty() {
        write_indent(output, indent + 1);
        output.push_str("<TestPlans>\n");
        for plan in test_plans {
            write_indent(output, indent + 2);
            let _ = writeln!(
                output,
                "<TestPlanReference reference=\"container:{}\" default=\"{}\"/>",
                xml_escape(&plan.path),
                bool_xml(plan.default_plan)
            );
        }
        write_indent(output, indent + 1);
        output.push_str("</TestPlans>\n");
    }
    if !command_line_arguments.is_empty()
        || !environment_variables.is_empty()
        || emit_empty_command_line_arguments
    {
        write_command_line_arguments(output, command_line_arguments, indent + 1);
    }
    write_environment_variables(output, environment_variables, indent + 1);
    if !coverage_targets.is_empty() {
        write_indent(output, indent + 1);
        output.push_str("<CodeCoverageTargets>\n");
        for target_name in coverage_targets {
            if let Some(target) = project.targets.get(target_name) {
                write_buildable_reference(output, project, target, object_id_map, indent + 2);
            } else if let Some((external_target, container)) =
                project_reference_target(project, target_name)
            {
                write_external_buildable_reference(output, external_target, container, indent + 2);
            } else if package_test_reference(project, target_name).is_some() {
                write_package_test_buildable_reference(output, project, target_name, indent + 2);
            }
        }
        write_indent(output, indent + 1);
        output.push_str("</CodeCoverageTargets>\n");
    }
    for action in post_actions {
        write_execution_action(
            output,
            project,
            "PostActions",
            action,
            object_id_map,
            indent + 1,
        );
    }
    write_indent(output, indent);
    output.push_str("</TestAction>\n");
}

#[allow(clippy::too_many_arguments)]
fn write_launch_action(
    output: &mut String,
    project: &Project,
    runnable: Option<&str>,
    config: Option<&str>,
    debug_enabled: bool,
    launch_automatically_substyle: Option<&str>,
    ask_for_app_to_launch: bool,
    custom_lldb_init: Option<&str>,
    custom_working_directory: Option<&str>,
    enable_gpu_frame_capture_mode: Option<&str>,
    store_kit_configuration: Option<&str>,
    macro_expansion: Option<&str>,
    simulate_location: Option<&crate::spec::SchemeSimulateLocation>,
    disable_main_thread_checker: bool,
    stop_on_every_main_thread_checker_issue: bool,
    disable_thread_performance_checker: bool,
    command_line_arguments: &indexmap::IndexMap<String, bool>,
    emit_empty_command_line_arguments: bool,
    language: Option<&str>,
    region: Option<&str>,
    environment_variables: &[crate::spec::EnvironmentVariable],
    default_config: &str,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    let mut attrs = vec![
        (
            "buildConfiguration",
            xml_escape(config.unwrap_or(default_config)),
        ),
        (
            "selectedDebuggerIdentifier",
            if debug_enabled {
                "Xcode.DebuggerFoundation.Debugger.LLDB"
            } else {
                ""
            }
            .to_owned(),
        ),
        (
            "selectedLauncherIdentifier",
            if debug_enabled {
                "Xcode.DebuggerFoundation.Launcher.LLDB"
            } else {
                "Xcode.IDEFoundation.Launcher.PosixSpawn"
            }
            .to_owned(),
        ),
    ];
    if disable_main_thread_checker {
        attrs.push(("disableMainThreadChecker", "YES".to_owned()));
    }
    attrs.push(("launchStyle", "0".to_owned()));
    if ask_for_app_to_launch {
        attrs.push(("askForAppToLaunch", "YES".to_owned()));
    }
    attrs.push((
        "useCustomWorkingDirectory",
        bool_xml(custom_working_directory.is_some()).to_owned(),
    ));
    if let Some(value) = custom_working_directory {
        attrs.push(("customWorkingDirectory", xml_escape(value)));
    }
    if let Some(value) = custom_lldb_init {
        attrs.push(("customLLDBInitFile", xml_escape(value)));
    }
    if let Some(value) = enable_gpu_frame_capture_mode {
        attrs.push(("enableGPUFrameCaptureMode", xml_escape(value)));
    }
    if let Some(value) = language {
        attrs.push(("language", xml_escape(value)));
    }
    if let Some(value) = region {
        attrs.push(("region", xml_escape(value)));
    }
    attrs.extend([
        ("ignoresPersistentStateOnLaunch", "NO".to_owned()),
        ("debugDocumentVersioning", "YES".to_owned()),
        ("debugServiceExtension", "internal".to_owned()),
        ("allowLocationSimulation", "YES".to_owned()),
    ]);
    if let Some(value) = launch_automatically_substyle {
        attrs.push(("launchAutomaticallySubstyle", xml_escape(value)));
    }
    if stop_on_every_main_thread_checker_issue {
        attrs.push(("stopOnEveryMainThreadCheckerIssue", "YES".to_owned()));
    }
    if disable_thread_performance_checker {
        attrs.push(("disableThreadPerformanceChecker", "YES".to_owned()));
    }
    write_multiline_start(output, indent, "LaunchAction", &attrs);
    if let Some(target_name) = runnable.and_then(|name| project.targets.get(name)) {
        if is_watch_app_product(&target_name.target_type) {
            write_multiline_start(
                output,
                indent + 1,
                "RemoteRunnable",
                &[("runnableDebuggingMode", "2".to_owned())],
            );
        } else {
            write_multiline_start(
                output,
                indent + 1,
                "BuildableProductRunnable",
                &[("runnableDebuggingMode", "0".to_owned())],
            );
        }
        write_buildable_reference(output, project, target_name, object_id_map, indent + 2);
        write_indent(output, indent + 1);
        if is_watch_app_product(&target_name.target_type) {
            output.push_str("</RemoteRunnable>\n");
        } else {
            output.push_str("</BuildableProductRunnable>\n");
        }
    }
    if let Some(store_kit_configuration) = store_kit_configuration {
        write_indent(output, indent + 1);
        let _ = writeln!(
            output,
            "<StoreKitConfigurationFileReference identifier=\"{}\"/>",
            xml_escape(&scheme_prefixed_path(project, store_kit_configuration))
        );
    }
    write_macro_expansion(output, project, macro_expansion, object_id_map, indent + 1);
    if let Some(simulate_location) = simulate_location {
        if let Some(identifier) = &simulate_location.default_location {
            write_indent(output, indent + 1);
            let is_gpx = identifier.ends_with(".gpx");
            let identifier = if is_gpx {
                scheme_prefixed_path(project, identifier)
            } else {
                identifier.clone()
            };
            let _ = writeln!(
                output,
                "<LocationScenarioReference identifier=\"{}\" referenceType=\"{}\"/>",
                xml_escape(&identifier),
                if is_gpx { "0" } else { "1" }
            );
        }
    }
    if !command_line_arguments.is_empty()
        || !environment_variables.is_empty()
        || emit_empty_command_line_arguments
    {
        write_command_line_arguments(output, command_line_arguments, indent + 1);
    }
    write_environment_variables(output, environment_variables, indent + 1);
    write_indent(output, indent);
    output.push_str("</LaunchAction>\n");
}

#[allow(clippy::too_many_arguments)]
fn write_profile_action(
    output: &mut String,
    project: &Project,
    runnable: Option<&str>,
    config: Option<&str>,
    environment_variables: &[crate::spec::EnvironmentVariable],
    ask_for_app_to_launch: bool,
    emit_empty_command_line_arguments: bool,
    macro_expansion: Option<&str>,
    default_config: &str,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    let mut attrs = vec![
        (
            "buildConfiguration",
            xml_escape(config.unwrap_or(default_config)),
        ),
        (
            "shouldUseLaunchSchemeArgsEnv",
            if environment_variables.is_empty() {
                "YES"
            } else {
                "NO"
            }
            .to_owned(),
        ),
        ("savedToolIdentifier", String::new()),
        ("useCustomWorkingDirectory", "NO".to_owned()),
        ("debugDocumentVersioning", "YES".to_owned()),
    ];
    if ask_for_app_to_launch {
        attrs.push(("askForAppToLaunch", "YES".to_owned()));
    }
    write_multiline_start(output, indent, "ProfileAction", &attrs);
    if let Some(target) = runnable.and_then(|name| project.targets.get(name)) {
        write_multiline_start(
            output,
            indent + 1,
            "BuildableProductRunnable",
            &[("runnableDebuggingMode", "0".to_owned())],
        );
        write_buildable_reference(output, project, target, object_id_map, indent + 2);
        write_indent(output, indent + 1);
        output.push_str("</BuildableProductRunnable>\n");
    }
    if !environment_variables.is_empty() || emit_empty_command_line_arguments {
        write_command_line_arguments(output, &indexmap::IndexMap::new(), indent + 1);
    }
    write_macro_expansion(output, project, macro_expansion, object_id_map, indent + 1);
    write_environment_variables(output, environment_variables, indent + 1);
    write_indent(output, indent);
    output.push_str("</ProfileAction>\n");
}

fn write_simple_action(
    output: &mut String,
    element: &str,
    attribute: &str,
    config: &str,
    indent: usize,
) {
    let mut attrs = vec![(attribute, xml_escape(config))];
    if element == "ArchiveAction" {
        attrs.push(("revealArchiveInOrganizer", "YES".to_owned()));
    }
    write_multiline_start(output, indent, element, &attrs);
    write_indent(output, indent);
    let _ = writeln!(output, "</{element}>");
}

fn write_execution_action(
    output: &mut String,
    project: &Project,
    container: &str,
    action: &crate::spec::SchemeAction,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(output, "<{container}>");
    write_multiline_start(
        output,
        indent + 1,
        "ExecutionAction",
        &[(
            "ActionType",
            "Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction"
                .to_owned(),
        )],
    );
    write_multiline_start(
        output,
        indent + 2,
        "ActionContent",
        &[
            ("title", xml_escape(&action.name)),
            ("scriptText", xml_escape(&action.script)),
        ],
    );
    if let Some(target) = action
        .settings_target
        .as_ref()
        .and_then(|target| project.targets.get(target))
    {
        write_indent(output, indent + 3);
        output.push_str("<EnvironmentBuildable>\n");
        write_buildable_reference(output, project, target, object_id_map, indent + 4);
        write_indent(output, indent + 3);
        output.push_str("</EnvironmentBuildable>\n");
    }
    write_indent(output, indent + 2);
    output.push_str("</ActionContent>\n");
    write_indent(output, indent + 1);
    output.push_str("</ExecutionAction>\n");
    write_indent(output, indent);
    let _ = writeln!(output, "</{container}>");
}

fn write_buildable_reference(
    output: &mut String,
    project: &Project,
    target: &Target,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    let raw_id = object_id(&format!("nativeTarget:{}", target.name), 0);
    write_multiline_start(
        output,
        indent,
        "BuildableReference",
        &[
            ("BuildableIdentifier", "primary".to_owned()),
            (
                "BlueprintIdentifier",
                mapped_id(&raw_id, object_id_map).into_owned(),
            ),
            ("BuildableName", xml_escape(&target.filename())),
            ("BlueprintName", xml_escape(&target.name)),
            (
                "ReferencedContainer",
                format!("container:{}.xcodeproj", xml_escape(&project.name)),
            ),
        ],
    );
    write_indent(output, indent);
    output.push_str("</BuildableReference>\n");
}

fn write_external_buildable_reference(
    output: &mut String,
    target_name: &str,
    container: &str,
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<BuildableReference BuildableIdentifier=\"primary\" BlueprintIdentifier=\"{}\" BuildableName=\"{}\" BlueprintName=\"{}\" ReferencedContainer=\"container:{}\"/>",
        xml_escape(target_name),
        xml_escape(target_name),
        xml_escape(target_name),
        xml_escape(container)
    );
}

fn write_package_test_buildable_reference(
    output: &mut String,
    project: &Project,
    target_reference: &str,
    indent: usize,
) {
    let Some((target_name, container)) = package_test_reference(project, target_reference) else {
        return;
    };
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<BuildableReference BuildableIdentifier=\"primary\" BlueprintIdentifier=\"{}\" BuildableName=\"{}\" BlueprintName=\"{}\" ReferencedContainer=\"container:{}\"/>",
        xml_escape(target_name),
        xml_escape(target_name),
        xml_escape(target_name),
        xml_escape(container)
    );
}

fn project_reference_target<'a>(
    project: &'a Project,
    target_reference: &'a str,
) -> Option<(&'a str, &'a str)> {
    let (reference_name, target_name) = target_reference.split_once('/')?;
    let path = project
        .project_references
        .get(reference_name)?
        .get("path")?
        .as_str()?;
    Some((target_name, path))
}

fn package_test_reference<'a>(
    project: &'a Project,
    target_reference: &'a str,
) -> Option<(&'a str, &'a str)> {
    let (package_name, target_name) = target_reference.split_once('/')?;
    match project.package_specs.get(package_name)? {
        crate::spec::SwiftPackage::Local { path, .. } => Some((target_name, path.as_str())),
        crate::spec::SwiftPackage::Remote { .. } => Some((target_name, package_name)),
    }
}

fn write_macro_expansion(
    output: &mut String,
    project: &Project,
    target_name: Option<&str>,
    object_id_map: &HashMap<String, String>,
    indent: usize,
) {
    let Some(target) = target_name.and_then(|name| project.targets.get(name)) else {
        return;
    };
    write_indent(output, indent);
    output.push_str("<MacroExpansion>\n");
    write_buildable_reference(output, project, target, object_id_map, indent + 1);
    write_indent(output, indent);
    output.push_str("</MacroExpansion>\n");
}

fn write_environment_variables(
    output: &mut String,
    variables: &[crate::spec::EnvironmentVariable],
    indent: usize,
) {
    if variables.is_empty() {
        return;
    }
    write_indent(output, indent);
    output.push_str("<EnvironmentVariables>\n");
    for variable in variables {
        write_multiline_start(
            output,
            indent + 1,
            "EnvironmentVariable",
            &[
                ("key", xml_escape(&variable.variable)),
                ("value", xml_escape(&variable.value)),
                ("isEnabled", bool_xml(variable.enabled).to_owned()),
            ],
        );
        write_indent(output, indent + 1);
        output.push_str("</EnvironmentVariable>\n");
    }
    write_indent(output, indent);
    output.push_str("</EnvironmentVariables>\n");
}

fn runnable_scheme_target(target: &Target) -> bool {
    matches!(
        target.target_type,
        ProductType::Application
            | ProductType::OnDemandInstallCapableApplication
            | ProductType::WatchApp
            | ProductType::Watch2App
            | ProductType::CommandLineTool
            | ProductType::AppExtension
            | ProductType::ExtensionKitExtension
    )
}

fn write_command_line_arguments(
    output: &mut String,
    arguments: &indexmap::IndexMap<String, bool>,
    indent: usize,
) {
    write_indent(output, indent);
    output.push_str("<CommandLineArguments>\n");
    for (argument, enabled) in arguments {
        write_multiline_start(
            output,
            indent + 1,
            "CommandLineArgument",
            &[
                ("argument", xml_escape(argument)),
                ("isEnabled", bool_xml(*enabled).to_owned()),
            ],
        );
        write_indent(output, indent + 1);
        output.push_str("</CommandLineArgument>\n");
    }
    write_indent(output, indent);
    output.push_str("</CommandLineArguments>\n");
}

fn write_build_action_entry_start(
    output: &mut String,
    indent: usize,
    build_types: &[crate::spec::BuildType],
) {
    let mut attrs = Vec::new();
    for (name, build_type) in [
        ("buildForTesting", crate::spec::BuildType::Testing),
        ("buildForRunning", crate::spec::BuildType::Running),
        ("buildForProfiling", crate::spec::BuildType::Profiling),
        ("buildForArchiving", crate::spec::BuildType::Archiving),
        ("buildForAnalyzing", crate::spec::BuildType::Analyzing),
    ] {
        attrs.push((name, bool_xml(build_types.contains(&build_type)).to_owned()));
    }
    write_multiline_start(output, indent, "BuildActionEntry", &attrs);
}

fn breakpoints_xml(breakpoints: &[crate::spec::Breakpoint]) -> String {
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str("<Bucket type=\"1\" version=\"2.0\">\n   <Breakpoints>\n");
    for breakpoint in breakpoints {
        output.push_str("      <BreakpointProxy BreakpointExtensionID=\"");
        output.push_str(breakpoint_extension_id(&breakpoint.breakpoint_type));
        output.push_str("\">\n         <BreakpointContent");
        let _ = write!(
            output,
            " shouldBeEnabled=\"{}\" ignoreCount=\"{}\" continueAfterRunningActions=\"{}\"",
            bool_xml(breakpoint.enabled),
            breakpoint.ignore_count,
            bool_xml(breakpoint.continue_after_running_actions)
        );
        if let Some(condition) = &breakpoint.condition {
            let _ = write!(output, " condition=\"{}\"", xml_escape(condition));
        }
        match &breakpoint.breakpoint_type {
            crate::spec::BreakpointType::File { path, line, column } => {
                let _ = write!(
                    output,
                    " filePath=\"{}\" startingLineNumber=\"{}\" endingLineNumber=\"{}\"",
                    xml_escape(path),
                    line,
                    line
                );
                if let Some(column) = column {
                    let _ = write!(
                        output,
                        " startingColumnNumber=\"{column}\" endingColumnNumber=\"{column}\""
                    );
                }
            }
            crate::spec::BreakpointType::Symbolic { symbol, module } => {
                if let Some(symbol) = symbol {
                    let _ = write!(output, " symbolName=\"{}\"", xml_escape(symbol));
                }
                if let Some(module) = module {
                    let _ = write!(output, " moduleName=\"{}\"", xml_escape(module));
                }
            }
            _ => {}
        }
        output.push_str("/>\n      </BreakpointProxy>\n");
    }
    output.push_str("   </Breakpoints>\n</Bucket>\n");
    output
}

fn breakpoint_extension_id(breakpoint_type: &crate::spec::BreakpointType) -> &'static str {
    match breakpoint_type {
        crate::spec::BreakpointType::File { .. } => "Xcode.Breakpoint.FileBreakpoint",
        crate::spec::BreakpointType::Exception { .. } => "Xcode.Breakpoint.ExceptionBreakpoint",
        crate::spec::BreakpointType::SwiftError => "Xcode.Breakpoint.SwiftErrorBreakpoint",
        crate::spec::BreakpointType::OpenGLError => "Xcode.Breakpoint.OpenGLErrorBreakpoint",
        crate::spec::BreakpointType::Symbolic { .. } => "Xcode.Breakpoint.SymbolicBreakpoint",
        crate::spec::BreakpointType::IdeConstraintError => {
            "Xcode.Breakpoint.IDEConstraintErrorBreakpoint"
        }
        crate::spec::BreakpointType::IdeTestFailure => "Xcode.Breakpoint.IDETestFailureBreakpoint",
        crate::spec::BreakpointType::RuntimeIssue => "Xcode.Breakpoint.RuntimeIssueBreakpoint",
    }
}

fn first_runnable_scheme_target<'a>(
    project: &'a Project,
    targets: &'a [crate::spec::SchemeBuildTarget],
) -> Option<&'a str> {
    targets
        .iter()
        .find(|target| {
            project
                .targets
                .get(&target.target)
                .map(|target| product_type_is_runnable(&target.target_type))
                .unwrap_or(false)
        })
        .map(|target| target.target.as_str())
}

fn first_testing_runnable_scheme_target<'a>(
    project: &'a Project,
    targets: &'a [crate::spec::SchemeBuildTarget],
) -> Option<&'a str> {
    targets
        .iter()
        .find(|target| {
            target
                .build_types
                .contains(&crate::spec::BuildType::Testing)
                && project
                    .targets
                    .get(&target.target)
                    .map(|target| product_type_is_runnable(&target.target_type))
                    .unwrap_or(false)
        })
        .map(|target| target.target.as_str())
}

fn product_type_is_runnable(product_type: &ProductType) -> bool {
    matches!(
        product_type,
        ProductType::Application
            | ProductType::OnDemandInstallCapableApplication
            | ProductType::WatchApp
            | ProductType::Watch2App
            | ProductType::MessagesApplication
            | ProductType::CommandLineTool
            | ProductType::AppExtension
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::MessagesExtension
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
    )
}

fn scheme_target_is_app_extension(product_type: &ProductType) -> bool {
    matches!(
        product_type,
        ProductType::AppExtension
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::MessagesExtension
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
    )
}

fn default_config_for(project: &Project, kind: &str) -> String {
    project
        .configs
        .iter()
        .find(|(_, config_kind)| config_kind.eq_ignore_ascii_case(kind))
        .map(|(name, _)| name.clone())
        .unwrap_or_else(|| {
            if kind == "release" {
                "Release".to_owned()
            } else {
                "Debug".to_owned()
            }
        })
}

fn variant_config(project: &Project, variant: Option<&str>, kind: &str) -> String {
    if let Some(variant) = variant {
        if let Some((name, _)) = project.configs.iter().find(|(name, config_kind)| {
            config_kind.eq_ignore_ascii_case(kind)
                && name
                    .to_ascii_lowercase()
                    .contains(&variant.to_ascii_lowercase())
        }) {
            return name.clone();
        }
    }
    default_config_for(project, kind)
}

fn scheme_prefixed_path(project: &Project, path: &str) -> String {
    match &project.spec_options.scheme_path_prefix {
        Some(prefix) if !prefix.is_empty() => format!("{prefix}{path}"),
        _ => path.to_owned(),
    }
}

fn scheme_last_upgrade_version(project: &Project) -> String {
    project
        .attributes
        .get("LastUpgradeCheck")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| project_xcode_version_last_upgrade_check(project))
        .as_deref()
        .unwrap_or("1430")
        .to_owned()
}

fn write_indent(output: &mut String, indent: usize) {
    for _ in 0..indent {
        output.push_str("   ");
    }
}

fn write_multiline_start(
    output: &mut String,
    indent: usize,
    element: &str,
    attrs: &[(&str, String)],
) {
    write_indent(output, indent);
    let _ = writeln!(output, "<{element}");
    for (index, (key, value)) in attrs.iter().enumerate() {
        write_indent(output, indent + 1);
        let suffix = if index + 1 == attrs.len() { ">" } else { "" };
        let _ = writeln!(output, "{key} = \"{value}\"{suffix}");
    }
}

fn bool_xml(value: bool) -> &'static str {
    if value {
        "YES"
    } else {
        "NO"
    }
}

pub(super) fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn source_build_file_settings(
    source: &TargetSource,
    file_type: Option<&FileType>,
    phase: &str,
) -> BTreeMap<String, PbxValue> {
    let mut settings = BTreeMap::new();
    let compiler_flags = file_type
        .map(|file_type| file_type.compiler_flags.clone())
        .unwrap_or_default()
        .into_iter()
        .chain(source.compiler_flags.clone())
        .collect::<Vec<_>>();
    if phase == "Sources" && !compiler_flags.is_empty() {
        settings.insert(
            "COMPILER_FLAGS".to_owned(),
            PbxValue::String(compiler_flags.join(" ")),
        );
    }
    let attributes = file_type
        .map(|file_type| file_type.attributes.clone())
        .unwrap_or_default()
        .into_iter()
        .chain(source.attributes.clone())
        .collect::<Vec<_>>();
    if !attributes.is_empty() {
        settings.insert("ATTRIBUTES".to_owned(), string_array(&attributes));
    }
    let resource_tags = file_type
        .map(|file_type| file_type.resource_tags.clone())
        .unwrap_or_default()
        .into_iter()
        .chain(source.resource_tags.clone())
        .collect::<Vec<_>>();
    if phase == "Resources" && !resource_tags.is_empty() {
        settings.insert("ASSET_TAGS".to_owned(), string_array(&resource_tags));
    }
    if phase == "Headers" {
        let visibility = source.header_visibility.as_deref().unwrap_or("public");
        let attribute = match visibility {
            "public" | "Public" => Some("Public"),
            "private" | "Private" => Some("Private"),
            "project" | "Project" => None,
            other => Some(other),
        };
        if let Some(attribute) = attribute {
            settings
                .entry("ATTRIBUTES".to_owned())
                .or_insert_with(|| PbxValue::Array(Vec::new()));
            if let Some(PbxValue::Array(values)) = settings.get_mut("ATTRIBUTES") {
                values.push(PbxValue::String(attribute.to_owned()));
            }
        }
    }
    settings
}

fn source_build_phase_override(source: &TargetSource) -> Option<Option<&'static str>> {
    match &source.build_phase {
        Some(Value::String(value)) => Some(match value.as_str() {
            "sources" => Some("Sources"),
            "resources" => Some("Resources"),
            "headers" => Some("Headers"),
            "none" => None,
            _ => return None,
        }),
        Some(Value::Object(map)) if map.contains_key("copyFiles") => Some(Some("CopyFiles")),
        _ => None,
    }
}

fn file_group_source(path: String) -> TargetSource {
    TargetSource {
        path,
        name: None,
        group: None,
        compiler_flags: Vec::new(),
        excludes: Vec::new(),
        includes: Vec::new(),
        explicit_folders: Vec::new(),
        source_type: Some(SourceType::Group),
        optional: false,
        build_phase: None,
        header_visibility: None,
        create_intermediate_groups: None,
        attributes: Vec::new(),
        resource_tags: Vec::new(),
        infer_destination_filters_by_path: None,
        destination_filters: Vec::new(),
        raw: Value::Null,
    }
}

fn path_is_under_group(path: &str, group: &str) -> bool {
    path == group || path.starts_with(&format!("{}/", group.trim_end_matches('/')))
}

fn string_array(values: &[String]) -> PbxValue {
    PbxValue::Array(values.iter().cloned().map(PbxValue::String).collect())
}

fn ordered_config_names(configs: &indexmap::IndexMap<String, String>) -> Vec<String> {
    let mut environments = Vec::new();
    let mut all_environment_configs = true;
    for config in configs.keys() {
        let Some(environment) = config
            .strip_suffix(" Debug")
            .or_else(|| config.strip_suffix(" Release"))
        else {
            all_environment_configs = false;
            break;
        };
        if !environments
            .iter()
            .any(|existing: &String| existing == environment)
        {
            environments.push(environment.to_owned());
        }
    }
    if all_environment_configs && !environments.is_empty() {
        environments.sort();
        let mut ordered = Vec::new();
        for environment in &environments {
            for suffix in ["Debug", "Release"] {
                let config = format!("{environment} {suffix}");
                if configs.contains_key(&config) {
                    ordered.push(config);
                }
            }
        }
        return ordered;
    }
    configs.keys().cloned().collect()
}

fn config_file_reference_key(
    _owner_type: &str,
    _owner_name: &str,
    _config_name: &str,
    path: &str,
) -> String {
    format!("navigatorFileRef:{path}")
}

fn split_config_file_path(path: &str) -> Option<(String, String)> {
    let path_ref = Path::new(path);
    let parent = path_ref.parent()?.to_str()?;
    if parent.is_empty() {
        return None;
    }
    let file_name = path_ref.file_name()?.to_str()?;
    Some((parent.to_owned(), file_name.to_owned()))
}

fn is_debug_config_name(config: &str) -> bool {
    config == "Debug" || config.ends_with(" Debug")
}

fn insert_project_setting_presets(settings: &mut BTreeMap<String, PbxValue>, config: &str) {
    for (key, value) in [
        ("ALWAYS_SEARCH_USER_PATHS", "NO"),
        ("CLANG_ANALYZER_NONNULL", "YES"),
        ("CLANG_ANALYZER_NUMBER_OBJECT_CONVERSION", "YES_AGGRESSIVE"),
        ("CLANG_CXX_LANGUAGE_STANDARD", "gnu++14"),
        ("CLANG_CXX_LIBRARY", "libc++"),
        ("CLANG_ENABLE_MODULES", "YES"),
        ("CLANG_ENABLE_OBJC_ARC", "YES"),
        ("CLANG_ENABLE_OBJC_WEAK", "YES"),
        ("CLANG_WARN_BLOCK_CAPTURE_AUTORELEASING", "YES"),
        ("CLANG_WARN_BOOL_CONVERSION", "YES"),
        ("CLANG_WARN_COMMA", "YES"),
        ("CLANG_WARN_CONSTANT_CONVERSION", "YES"),
        ("CLANG_WARN_DEPRECATED_OBJC_IMPLEMENTATIONS", "YES"),
        ("CLANG_WARN_DIRECT_OBJC_ISA_USAGE", "YES_ERROR"),
        ("CLANG_WARN_DOCUMENTATION_COMMENTS", "YES"),
        ("CLANG_WARN_EMPTY_BODY", "YES"),
        ("CLANG_WARN_ENUM_CONVERSION", "YES"),
        ("CLANG_WARN_INFINITE_RECURSION", "YES"),
        ("CLANG_WARN_INT_CONVERSION", "YES"),
        ("CLANG_WARN_NON_LITERAL_NULL_CONVERSION", "YES"),
        ("CLANG_WARN_OBJC_IMPLICIT_RETAIN_SELF", "YES"),
        ("CLANG_WARN_OBJC_LITERAL_CONVERSION", "YES"),
        ("CLANG_WARN_OBJC_ROOT_CLASS", "YES_ERROR"),
        ("CLANG_WARN_QUOTED_INCLUDE_IN_FRAMEWORK_HEADER", "YES"),
        ("CLANG_WARN_RANGE_LOOP_ANALYSIS", "YES"),
        ("CLANG_WARN_STRICT_PROTOTYPES", "YES"),
        ("CLANG_WARN_SUSPICIOUS_MOVE", "YES"),
        ("CLANG_WARN_UNGUARDED_AVAILABILITY", "YES_AGGRESSIVE"),
        ("CLANG_WARN_UNREACHABLE_CODE", "YES"),
        ("CLANG_WARN__DUPLICATE_METHOD_MATCH", "YES"),
        ("COPY_PHASE_STRIP", "NO"),
        ("ENABLE_STRICT_OBJC_MSGSEND", "YES"),
        ("GCC_C_LANGUAGE_STANDARD", "gnu11"),
        ("GCC_NO_COMMON_BLOCKS", "YES"),
        ("GCC_WARN_64_TO_32_BIT_CONVERSION", "YES"),
        ("GCC_WARN_ABOUT_RETURN_TYPE", "YES_ERROR"),
        ("GCC_WARN_UNDECLARED_SELECTOR", "YES"),
        ("GCC_WARN_UNINITIALIZED_AUTOS", "YES_AGGRESSIVE"),
        ("GCC_WARN_UNUSED_FUNCTION", "YES"),
        ("GCC_WARN_UNUSED_VARIABLE", "YES"),
        ("MTL_FAST_MATH", "YES"),
        ("SWIFT_VERSION", "5.0"),
    ] {
        settings
            .entry(key.to_owned())
            .or_insert_with(|| PbxValue::String(value.to_owned()));
    }

    if is_debug_config_name(config) {
        settings.insert(
            "DEBUG_INFORMATION_FORMAT".to_owned(),
            PbxValue::String("dwarf".to_owned()),
        );
        settings.insert("ENABLE_TESTABILITY".to_owned(), pbx_bool(true));
        settings.insert("GCC_DYNAMIC_NO_PIC".to_owned(), pbx_bool(false));
        settings.insert(
            "GCC_OPTIMIZATION_LEVEL".to_owned(),
            PbxValue::String("0".to_owned()),
        );
        settings.insert(
            "GCC_PREPROCESSOR_DEFINITIONS".to_owned(),
            PbxValue::Array(vec![
                PbxValue::String("$(inherited)".to_owned()),
                PbxValue::String("DEBUG=1".to_owned()),
            ]),
        );
        settings.insert(
            "MTL_ENABLE_DEBUG_INFO".to_owned(),
            PbxValue::String("INCLUDE_SOURCE".to_owned()),
        );
        settings.insert("ONLY_ACTIVE_ARCH".to_owned(), pbx_bool(true));
        settings.insert(
            "SWIFT_ACTIVE_COMPILATION_CONDITIONS".to_owned(),
            PbxValue::String("DEBUG".to_owned()),
        );
        settings.insert(
            "SWIFT_OPTIMIZATION_LEVEL".to_owned(),
            PbxValue::String("-Onone".to_owned()),
        );
    } else {
        settings.insert(
            "DEBUG_INFORMATION_FORMAT".to_owned(),
            PbxValue::String("dwarf-with-dsym".to_owned()),
        );
        settings.insert("ENABLE_NS_ASSERTIONS".to_owned(), pbx_bool(false));
        settings.insert(
            "MTL_ENABLE_DEBUG_INFO".to_owned(),
            PbxValue::String("NO".to_owned()),
        );
        settings.insert(
            "SWIFT_COMPILATION_MODE".to_owned(),
            PbxValue::String("wholemodule".to_owned()),
        );
        settings.insert(
            "SWIFT_OPTIMIZATION_LEVEL".to_owned(),
            PbxValue::String("-O".to_owned()),
        );
    }
}

fn xcconfig_include_path(line: &str) -> Option<PathBuf> {
    let include = line.strip_prefix("#include")?.trim();
    let include = include
        .strip_prefix('"')
        .and_then(|value| value.split_once('"').map(|(path, _)| path))
        .or_else(|| {
            include
                .strip_prefix('<')
                .and_then(|value| value.split_once('>').map(|(path, _)| path))
        })?;
    Some(PathBuf::from(include))
}

fn insert_target_setting_presets(
    settings: &mut BTreeMap<String, PbxValue>,
    target: &Target,
    _config: &str,
    project: &Project,
) {
    if matches!(target.platform, Platform::Macos) {
        settings
            .entry("COMBINE_HIDPI_IMAGES".to_owned())
            .or_insert_with(|| pbx_bool(true));
    }
    if product_type_is_app(&target.target_type) {
        if target.target_type == ProductType::Application {
            settings
                .entry("ASSETCATALOG_COMPILER_APPICON_NAME".to_owned())
                .or_insert_with(|| PbxValue::String("AppIcon".to_owned()));
        }
        if matches!(target.platform, Platform::Ios)
            && target.target_type == ProductType::Application
        {
            settings
                .entry("CODE_SIGN_IDENTITY".to_owned())
                .or_insert_with(|| PbxValue::String("iPhone Developer".to_owned()));
        }
        if !matches!(target.platform, Platform::Watchos) {
            settings
                .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
                .or_insert_with(|| executable_runpath_search_paths_for_platform(&target.platform));
        }
        if is_watch_app_product(&target.target_type) {
            settings
                .entry("SKIP_INSTALL".to_owned())
                .or_insert_with(|| pbx_bool(true));
        }
    } else if target.target_type == ProductType::UnitTestBundle {
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(|| test_runpath_search_paths_for_platform(&target.platform));
        if let Some(test_host) = test_host_setting(project, target) {
            settings
                .entry("TEST_HOST".to_owned())
                .or_insert_with(|| PbxValue::String(test_host));
        }
        settings
            .entry("BUNDLE_LOADER".to_owned())
            .or_insert_with(|| PbxValue::String("$(TEST_HOST)".to_owned()));
    } else if target.target_type == ProductType::UiTestBundle {
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(|| test_runpath_search_paths_for_platform(&target.platform));
        if let Some(test_target_name) = test_target_name_setting(project, target) {
            settings
                .entry("BUNDLE_LOADER".to_owned())
                .or_insert_with(|| PbxValue::String("$(TEST_HOST)".to_owned()));
            settings
                .entry("TEST_TARGET_NAME".to_owned())
                .or_insert_with(|| PbxValue::String(test_target_name));
        }
    } else if matches!(target.target_type, ProductType::StaticLibrary) {
        if !matches!(target.platform, Platform::Watchos) {
            settings
                .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
                .or_insert_with(|| executable_runpath_search_paths_for_platform(&target.platform));
        }
        settings
            .entry("SKIP_INSTALL".to_owned())
            .or_insert_with(|| pbx_bool(true));
    } else if matches!(
        target.target_type,
        ProductType::Bundle | ProductType::Other(_)
    ) {
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(|| executable_runpath_search_paths_for_platform(&target.platform));
    } else if matches!(
        target.target_type,
        ProductType::WatchExtension | ProductType::Watch2Extension
    ) {
        settings
            .entry("ASSETCATALOG_COMPILER_COMPLICATION_NAME".to_owned())
            .or_insert_with(|| PbxValue::String("Complication".to_owned()));
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(watch_extension_runpath_search_paths);
        settings
            .entry("SKIP_INSTALL".to_owned())
            .or_insert_with(|| pbx_bool(true));
    } else if target.target_type == ProductType::MessagesExtension {
        settings
            .entry("ASSETCATALOG_COMPILER_APPICON_NAME".to_owned())
            .or_insert_with(|| PbxValue::String("iMessage App Icon".to_owned()));
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(watch_extension_runpath_search_paths);
    } else if target.target_type == ProductType::StickerPack {
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(executable_runpath_search_paths);
    } else if matches!(
        target.target_type,
        ProductType::SystemExtension
            | ProductType::DriverExtension
            | ProductType::XpcService
            | ProductType::CommandLineTool
    ) {
        settings
            .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
            .or_insert_with(|| executable_runpath_search_paths_for_platform(&target.platform));
    }
    if matches!(
        target.target_type,
        ProductType::Framework | ProductType::StaticFramework
    ) {
        insert_framework_target_setting_presets(settings, &target.platform);
    }
}

fn insert_framework_target_setting_presets(
    settings: &mut BTreeMap<String, PbxValue>,
    platform: &Platform,
) {
    settings
        .entry("CODE_SIGN_IDENTITY".to_owned())
        .or_insert_with(|| PbxValue::String(String::new()));
    settings
        .entry("CURRENT_PROJECT_VERSION".to_owned())
        .or_insert_with(|| PbxValue::Int(1));
    settings
        .entry("DEFINES_MODULE".to_owned())
        .or_insert_with(|| pbx_bool(true));
    settings
        .entry("DYLIB_COMPATIBILITY_VERSION".to_owned())
        .or_insert_with(|| PbxValue::Int(1));
    settings
        .entry("DYLIB_CURRENT_VERSION".to_owned())
        .or_insert_with(|| PbxValue::Int(1));
    settings
        .entry("DYLIB_INSTALL_NAME_BASE".to_owned())
        .or_insert_with(|| PbxValue::String("@rpath".to_owned()));
    settings
        .entry("INSTALL_PATH".to_owned())
        .or_insert_with(|| PbxValue::String("$(LOCAL_LIBRARY_DIR)/Frameworks".to_owned()));
    match platform {
        Platform::Macos => {
            settings
                .entry("COMBINE_HIDPI_IMAGES".to_owned())
                .or_insert_with(|| pbx_bool(true));
            settings
                .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
                .or_insert_with(macos_runpath_search_paths);
        }
        Platform::Watchos => {}
        _ => {
            settings
                .entry("LD_RUNPATH_SEARCH_PATHS".to_owned())
                .or_insert_with(executable_runpath_search_paths);
        }
    }
    settings
        .entry("SKIP_INSTALL".to_owned())
        .or_insert_with(|| pbx_bool(true));
    settings
        .entry("VERSIONING_SYSTEM".to_owned())
        .or_insert_with(|| PbxValue::String("apple-generic".to_owned()));
}

fn executable_runpath_search_paths() -> PbxValue {
    PbxValue::Array(vec![
        PbxValue::String("$(inherited)".to_owned()),
        PbxValue::String("@executable_path/Frameworks".to_owned()),
    ])
}

fn macos_runpath_search_paths() -> PbxValue {
    PbxValue::Array(vec![
        PbxValue::String("$(inherited)".to_owned()),
        PbxValue::String("@executable_path/../Frameworks".to_owned()),
    ])
}

fn executable_runpath_search_paths_for_platform(platform: &Platform) -> PbxValue {
    if matches!(platform, Platform::Macos) {
        macos_runpath_search_paths()
    } else {
        executable_runpath_search_paths()
    }
}

fn test_runpath_search_paths() -> PbxValue {
    PbxValue::Array(vec![
        PbxValue::String("$(inherited)".to_owned()),
        PbxValue::String("@executable_path/Frameworks".to_owned()),
        PbxValue::String("@loader_path/Frameworks".to_owned()),
    ])
}

fn watch_extension_runpath_search_paths() -> PbxValue {
    PbxValue::Array(vec![
        PbxValue::String("$(inherited)".to_owned()),
        PbxValue::String("@executable_path/Frameworks".to_owned()),
        PbxValue::String("@executable_path/../../Frameworks".to_owned()),
    ])
}

fn test_runpath_search_paths_for_platform(platform: &Platform) -> PbxValue {
    if matches!(platform, Platform::Macos) {
        PbxValue::Array(vec![
            PbxValue::String("$(inherited)".to_owned()),
            PbxValue::String("@executable_path/../Frameworks".to_owned()),
            PbxValue::String("@loader_path/../Frameworks".to_owned()),
        ])
    } else {
        test_runpath_search_paths()
    }
}

fn test_target_name_setting(project: &Project, target: &Target) -> Option<String> {
    test_target_reference_name(project, target)
        .and_then(|target_name| project.targets.get(target_name))
        .map(|target| target.product_name.clone())
}

fn test_target_reference_name<'a>(project: &'a Project, target: &'a Target) -> Option<&'a str> {
    target.dependencies.iter().find_map(|dependency| {
        if dependency.dependency_type != DependencyType::Target {
            return None;
        }
        let dependency_target = project.targets.get(&dependency.reference)?;
        if dependency_target.target_type != ProductType::Application
            && dependency_target.target_type != ProductType::OnDemandInstallCapableApplication
        {
            return None;
        }
        Some(dependency.reference.as_str())
    })
}

fn bundle_identifier_suffix(target_name: &str) -> String {
    target_name.replace('_', "-").replace(' ', "")
}

fn test_host_setting(project: &Project, target: &Target) -> Option<String> {
    target.dependencies.iter().find_map(|dependency| {
        if dependency.dependency_type != DependencyType::Target {
            return None;
        }
        let dependency_target = project.targets.get(&dependency.reference)?;
        if dependency_target.target_type != ProductType::Application
            && dependency_target.target_type != ProductType::OnDemandInstallCapableApplication
        {
            return None;
        }
        Some(format!(
            "$(BUILT_PRODUCTS_DIR)/{}{}",
            dependency_target.filename(),
            if dependency_target.platform == Platform::Macos {
                format!("/Contents/MacOS/{}", dependency_target.product_name)
            } else {
                format!("/{}", dependency_target.product_name)
            }
        ))
    })
}

fn insert_deployment_target(
    settings: &mut BTreeMap<String, PbxValue>,
    platform: &Platform,
    deployment_target: Option<&str>,
) {
    let Some(deployment_target) = deployment_target else {
        return;
    };
    let key = platform.deployment_target_setting();
    if !key.is_empty() {
        settings.insert(
            key.to_owned(),
            PbxValue::String(deployment_target.to_owned()),
        );
    }
}

fn insert_targeted_device_family(settings: &mut BTreeMap<String, PbxValue>, platform: &Platform) {
    let Some(family) = targeted_device_family(platform) else {
        return;
    };
    settings.insert(
        "TARGETED_DEVICE_FAMILY".to_owned(),
        PbxValue::String(family.to_owned()),
    );
}

fn targeted_device_family(platform: &Platform) -> Option<&'static str> {
    match platform {
        Platform::Ios => Some("1,2"),
        Platform::Tvos => Some("3"),
        Platform::Watchos => Some("4"),
        Platform::Visionos => Some("7"),
        Platform::Macos | Platform::Auto => None,
    }
}

fn supported_platforms_setting(target: &Target) -> Option<String> {
    if target.supported_destinations.is_empty() {
        return None;
    }
    let mut platforms = Vec::new();
    let has_duplicate_destinations = target
        .supported_destinations
        .iter()
        .collect::<BTreeSet<_>>()
        .len()
        != target.supported_destinations.len();
    let destinations: Vec<String> = if has_duplicate_destinations {
        ["iOS", "tvOS", "watchOS", "visionOS", "macOS"]
            .into_iter()
            .flat_map(|destination| {
                target
                    .supported_destinations
                    .iter()
                    .filter(move |supported| supported.as_str() == destination)
                    .map(move |_| destination.to_owned())
            })
            .collect()
    } else {
        ["iOS", "tvOS", "watchOS", "visionOS", "macOS"]
            .into_iter()
            .filter(|destination| has_supported_destination(target, destination))
            .map(str::to_owned)
            .collect()
    };
    for destination in destinations {
        match destination.as_str() {
            "iOS" => {
                platforms.push("iphoneos");
                platforms.push("iphonesimulator");
            }
            "tvOS" => {
                platforms.push("appletvos");
                platforms.push("appletvsimulator");
            }
            "watchOS" => {
                platforms.push("watchos");
                platforms.push("watchsimulator");
            }
            "visionOS" => {
                platforms.push("xros");
                platforms.push("xrsimulator");
            }
            "macOS" => platforms.push("macosx"),
            _ => {}
        }
    }
    if platforms.is_empty() {
        None
    } else {
        Some(platforms.join(" "))
    }
}

fn targeted_device_family_for_destinations(target: &Target) -> Option<String> {
    let mut families = Vec::new();
    let has_duplicate_destinations = target
        .supported_destinations
        .iter()
        .collect::<BTreeSet<_>>()
        .len()
        != target.supported_destinations.len();
    if has_duplicate_destinations {
        for destination in ["iOS", "tvOS", "watchOS", "visionOS"] {
            for supported_destination in &target.supported_destinations {
                if supported_destination != destination {
                    continue;
                }
                match destination {
                    "iOS" => {
                        families.push("1");
                        families.push("2");
                    }
                    "tvOS" => families.push("3"),
                    "watchOS" => families.push("4"),
                    "visionOS" => families.push("7"),
                    _ => {}
                }
            }
        }
    } else {
        for (destination, family) in [
            ("iOS", "1"),
            ("iOS", "2"),
            ("tvOS", "3"),
            ("watchOS", "4"),
            ("visionOS", "7"),
        ] {
            for supported_destination in &target.supported_destinations {
                if supported_destination == destination && !families.contains(&family) {
                    families.push(family);
                }
            }
        }
    }
    if families.is_empty() {
        None
    } else {
        Some(families.join(","))
    }
}

fn insert_supported_destination_presets(
    settings: &mut BTreeMap<String, PbxValue>,
    target: &Target,
) {
    let has_ios = has_supported_destination(target, "iOS");
    let has_tvos = has_supported_destination(target, "tvOS");
    let has_visionos = has_supported_destination(target, "visionOS");
    let has_macos = has_supported_destination(target, "macOS");
    let has_mac_catalyst = has_supported_destination(target, "macCatalyst");

    settings.insert(
        "SUPPORTS_MACCATALYST".to_owned(),
        pbx_bool(has_mac_catalyst),
    );
    if has_macos && !has_ios && !has_tvos && !has_visionos {
        settings.insert("COMBINE_HIDPI_IMAGES".to_owned(), pbx_bool(true));
        settings.insert(
            "SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD".to_owned(),
            pbx_bool(false),
        );
        settings.insert(
            "TARGETED_DEVICE_FAMILY".to_owned(),
            PbxValue::String(String::new()),
        );
    }
    if has_ios || has_tvos || has_visionos {
        settings.insert(
            "SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD".to_owned(),
            pbx_bool(has_ios && !has_macos && !has_mac_catalyst),
        );
    }
    if has_ios || has_visionos {
        settings.insert(
            "SUPPORTS_XR_DESIGNED_FOR_IPHONE_IPAD".to_owned(),
            pbx_bool(has_ios && !has_visionos),
        );
    }

    if target.target_type == ProductType::Application {
        if has_ios || has_tvos {
            settings.insert(
                "LD_RUNPATH_SEARCH_PATHS".to_owned(),
                PbxValue::Array(vec![
                    PbxValue::String("$(inherited)".to_owned()),
                    PbxValue::String("@executable_path/Frameworks".to_owned()),
                ]),
            );
        }
        if has_tvos && !has_ios {
            settings.insert(
                "ASSETCATALOG_COMPILER_APPICON_NAME".to_owned(),
                PbxValue::String("App Icon & Top Shelf Image".to_owned()),
            );
            settings.insert(
                "ASSETCATALOG_COMPILER_LAUNCHIMAGE_NAME".to_owned(),
                PbxValue::String("LaunchImage".to_owned()),
            );
        } else if has_ios || has_visionos {
            settings.insert(
                "ASSETCATALOG_COMPILER_APPICON_NAME".to_owned(),
                PbxValue::String("AppIcon".to_owned()),
            );
        }
        if has_ios {
            settings.insert(
                "CODE_SIGN_IDENTITY".to_owned(),
                PbxValue::String("iPhone Developer".to_owned()),
            );
        }
    }
}

fn pbx_bool(value: bool) -> PbxValue {
    PbxValue::String(if value { "YES" } else { "NO" }.to_owned())
}

fn carthage_framework_search_paths(options: &SpecOptions, target: &Target) -> Option<PbxValue> {
    let mut paths = Vec::<String>::new();
    for dependency in &target.dependencies {
        let DependencyType::Carthage {
            find_frameworks,
            link_type,
        } = &dependency.dependency_type
        else {
            continue;
        };
        if !find_frameworks.unwrap_or(options.find_carthage_frameworks) {
            continue;
        }
        let platform_dir = carthage_platform_dir(&target.platform);
        let suffix = if *link_type == crate::spec::CarthageLinkType::Static {
            format!("{platform_dir}/Static")
        } else {
            platform_dir.to_owned()
        };
        let base_path = options
            .carthage_build_path
            .as_deref()
            .unwrap_or("Carthage/Build");
        let path = format!("$(PROJECT_DIR)/{base_path}/{suffix}");
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    if paths.is_empty() {
        None
    } else {
        let mut values = vec![PbxValue::String("$(inherited)".to_owned())];
        values.extend(paths.into_iter().map(PbxValue::String));
        Some(PbxValue::Array(values))
    }
}

fn framework_dependency_search_paths(target: &Target) -> Option<PbxValue> {
    let mut paths = BTreeSet::new();
    for dependency in &target.dependencies {
        if dependency.dependency_type != DependencyType::Framework {
            continue;
        }
        let parent = Path::new(&dependency.reference)
            .parent()
            .and_then(|parent| parent.to_str())
            .unwrap_or(".");
        paths.insert(if parent.is_empty() { "." } else { parent }.to_owned());
    }
    if paths.is_empty() {
        return None;
    }
    let mut values = vec![PbxValue::String("$(inherited)".to_owned())];
    values.extend(paths.into_iter().map(|path| {
        if path == "." {
            PbxValue::String("\".\"".to_owned())
        } else if path.contains('/') {
            PbxValue::String(format!("\"{path}\""))
        } else {
            PbxValue::String(path)
        }
    }));
    Some(PbxValue::Array(values))
}

fn carthage_dependency_references(
    project: &Project,
    target: &Target,
    dependency: &Dependency,
) -> Vec<String> {
    carthage_dependency_references_for_platform(project, &target.platform, dependency)
}

fn carthage_dependency_references_for_platform(
    project: &Project,
    platform: &Platform,
    dependency: &Dependency,
) -> Vec<String> {
    let DependencyType::Carthage {
        find_frameworks, ..
    } = &dependency.dependency_type
    else {
        return Vec::new();
    };
    if !find_frameworks.unwrap_or(project.spec_options.find_carthage_frameworks) {
        return vec![dependency.reference.clone()];
    }

    let base_path = project
        .spec_options
        .carthage_build_path
        .as_deref()
        .unwrap_or("Carthage/Build");
    let version_path = project
        .base_path
        .join(base_path)
        .join(format!(".{}.version", dependency.reference));
    let Ok(data) = fs::read_to_string(version_path) else {
        return vec![dependency.reference.clone()];
    };
    let Ok(Value::Object(version_file)) = serde_json::from_str::<Value>(&data) else {
        return vec![dependency.reference.clone()];
    };
    let platform_dir = carthage_platform_dir(platform);
    let Some(Value::Array(references)) = version_file.get(platform_dir) else {
        return vec![dependency.reference.clone()];
    };

    let mut names = references
        .iter()
        .filter_map(|reference| reference.get("name").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    if names.is_empty() {
        vec![dependency.reference.clone()]
    } else {
        names
    }
}

#[derive(Debug, Clone)]
struct ResolvedCarthageDependency {
    dependency: Dependency,
}

enum ProjectTargetRef<'a> {
    Target(&'a Target),
    Aggregate(&'a AggregateTarget),
}

impl<'a> ProjectTargetRef<'a> {
    fn name(&self) -> &str {
        match self {
            Self::Target(target) => &target.name,
            Self::Aggregate(target) => &target.name,
        }
    }
}

fn resolved_carthage_dependencies(
    project: &Project,
    top_level_target: &Target,
) -> Vec<ResolvedCarthageDependency> {
    let mut visited_targets = HashSet::<String>::new();
    let mut seen_dependencies = HashSet::<String>::new();
    let mut resolved = Vec::<ResolvedCarthageDependency>::new();
    let mut queue = VecDeque::from([ProjectTargetRef::Target(top_level_target)]);

    while let Some(project_target) = queue.pop_front() {
        if !visited_targets.insert(project_target.name().to_owned()) {
            continue;
        }

        match project_target {
            ProjectTargetRef::Target(target) => {
                for dependency in &target.dependencies {
                    if dependency.link == Some(false)
                        && !dependency.embed.unwrap_or_else(|| {
                            target_should_embed_carthage_dependencies(top_level_target)
                        })
                    {
                        continue;
                    }

                    match &dependency.dependency_type {
                        DependencyType::Carthage {
                            find_frameworks, ..
                        } => {
                            let references = if find_frameworks
                                .unwrap_or(project.spec_options.find_carthage_frameworks)
                            {
                                carthage_dependency_references_for_platform(
                                    project,
                                    &target.platform,
                                    dependency,
                                )
                            } else {
                                vec![dependency.reference.clone()]
                            };
                            for reference in references {
                                let mut dependency = dependency.clone();
                                dependency.reference = reference;
                                let key = resolved_carthage_dependency_key(&dependency);
                                if seen_dependencies.insert(key) {
                                    resolved.push(ResolvedCarthageDependency { dependency });
                                }
                            }
                        }
                        DependencyType::Target => {
                            if let Some(target) = project.targets.get(&dependency.reference) {
                                if top_level_target.platform == target.platform {
                                    queue.push_back(ProjectTargetRef::Target(target));
                                }
                            } else if let Some(aggregate) =
                                project.aggregate_target_specs.get(&dependency.reference)
                            {
                                queue.push_back(ProjectTargetRef::Aggregate(aggregate));
                            }
                        }
                        DependencyType::Framework
                        | DependencyType::Sdk { .. }
                        | DependencyType::Package { .. }
                        | DependencyType::Bundle => {}
                    }
                }
            }
            ProjectTargetRef::Aggregate(aggregate) => {
                for dependency_name in &aggregate.targets {
                    if let Some(target) = project.targets.get(dependency_name) {
                        queue.push_back(ProjectTargetRef::Target(target));
                    } else if let Some(aggregate) =
                        project.aggregate_target_specs.get(dependency_name)
                    {
                        queue.push_back(ProjectTargetRef::Aggregate(aggregate));
                    }
                }
            }
        }
    }

    resolved.sort_by(|left, right| left.dependency.reference.cmp(&right.dependency.reference));
    resolved
}

fn resolved_carthage_dependency_key(dependency: &Dependency) -> String {
    let link_type = match &dependency.dependency_type {
        DependencyType::Carthage { link_type, .. } => format!("{link_type:?}"),
        _ => String::new(),
    };
    format!(
        "{}|{}|{:?}|{:?}|{:?}|{}|{}",
        dependency.reference,
        link_type,
        dependency.embed,
        dependency.code_sign,
        dependency.link,
        dependency.implicit,
        dependency.weak_link
    )
}

fn carthage_platform_dir(platform: &Platform) -> &'static str {
    match platform {
        Platform::Macos => "Mac",
        Platform::Tvos => "tvOS",
        Platform::Watchos => "watchOS",
        Platform::Visionos => "visionOS",
        Platform::Ios | Platform::Auto => "iOS",
    }
}

fn carthage_framework_name(reference: &str) -> String {
    if reference.ends_with(".framework") {
        display_name(reference)
    } else {
        format!("{reference}.framework")
    }
}

fn ordered_carthage_platform_groups(groups: &BTreeMap<Platform, PbxValue>) -> Vec<PbxValue> {
    [
        Platform::Ios,
        Platform::Macos,
        Platform::Tvos,
        Platform::Watchos,
        Platform::Visionos,
        Platform::Auto,
    ]
    .into_iter()
    .filter_map(|platform| groups.get(&platform).cloned())
    .collect()
}

fn has_supported_destination(target: &Target, destination: &str) -> bool {
    target
        .supported_destinations
        .iter()
        .any(|item| item == destination)
}

fn platform_filters_for_source(source: &TargetSource, path: &str) -> Vec<String> {
    let explicit = source
        .destination_filters
        .iter()
        .filter_map(|filter| supported_destination_filter(filter))
        .collect::<Vec<_>>();
    if !explicit.is_empty() {
        return explicit;
    }
    if source.infer_destination_filters_by_path != Some(true) {
        return Vec::new();
    }
    infer_platform_filter_from_path(path).into_iter().collect()
}

fn platform_filters_for_dependency(dependency: &crate::spec::Dependency) -> Vec<String> {
    dependency
        .destination_filters
        .iter()
        .filter_map(|filter| supported_destination_filter(filter))
        .collect()
}

fn target_dependency_platform_filter(dependency: Option<&Dependency>) -> Option<String> {
    match dependency?.platform_filter {
        PlatformFilter::Ios => Some("ios".to_owned()),
        PlatformFilter::Macos => Some("macos".to_owned()),
        PlatformFilter::All => None,
    }
}

fn dependency_platform_filter(dependency: &Dependency) -> Option<String> {
    target_dependency_platform_filter(Some(dependency))
}

fn copy_files_settings_for_target_dependency(
    dependency: &Dependency,
    product_type: &ProductType,
) -> Option<CopyFilesSettings> {
    if let Some(settings) = dependency
        .copy_phase
        .as_ref()
        .and_then(copy_files_settings_from_value)
    {
        return Some(settings);
    }

    match product_type {
        ProductType::Framework | ProductType::StaticFramework => Some(CopyFilesSettings {
            dst_subfolder_spec: 10,
            dst_path: String::new(),
            phase_name: "Embed Frameworks".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::WatchApp | ProductType::Watch2App => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(CONTENTS_FOLDER_PATH)/Watch".to_owned(),
            phase_name: "Embed Watch Content".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::AppExtension
        | ProductType::XcodeExtension
        | ProductType::IntentsServiceExtension
        | ProductType::WatchExtension
        | ProductType::Watch2Extension
        | ProductType::TvExtension
        | ProductType::MessagesExtension
        | ProductType::StickerPack => Some(CopyFilesSettings {
            dst_subfolder_spec: 13,
            dst_path: String::new(),
            phase_name: "Embed Foundation Extensions".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::ExtensionKitExtension => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(EXTENSIONS_FOLDER_PATH)".to_owned(),
            phase_name: "Embed Foundation Extensions".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::XpcService => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(CONTENTS_FOLDER_PATH)/XPCServices".to_owned(),
            phase_name: "CopyFiles".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::OnDemandInstallCapableApplication => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(CONTENTS_FOLDER_PATH)/AppClips".to_owned(),
            phase_name: "Embed App Clips".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        ProductType::SystemExtension | ProductType::DriverExtension => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(SYSTEM_EXTENSIONS_FOLDER_PATH)".to_owned(),
            phase_name: "Embed System Extensions".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        }),
        _ => None,
    }
}

fn copy_build_file_settings_for_target_dependency(
    dependency: &Dependency,
    target: &Target,
) -> Option<PbxValue> {
    let code_sign = dependency.code_sign.unwrap_or_else(|| {
        target.target_type.is_framework() || target.target_type == ProductType::XpcService
    });
    copy_build_file_settings(code_sign, dependency.remove_headers)
}

fn copy_build_file_settings_for_dependency(dependency: &Dependency) -> Option<PbxValue> {
    let code_sign = dependency.code_sign.unwrap_or(true);
    copy_build_file_settings(code_sign, dependency.remove_headers)
}

fn copy_build_file_settings(code_sign: bool, remove_headers: bool) -> Option<PbxValue> {
    let mut attributes = Vec::new();
    if code_sign {
        attributes.push(PbxValue::String("CodeSignOnCopy".to_owned()));
    }
    if remove_headers {
        attributes.push(PbxValue::String("RemoveHeadersOnCopy".to_owned()));
    }
    if attributes.is_empty() {
        return None;
    }
    let mut settings = BTreeMap::new();
    settings.insert("ATTRIBUTES".to_owned(), PbxValue::Array(attributes));
    Some(PbxValue::Dict(settings))
}

fn should_embed_target_dependency(
    parent_target: &Target,
    dependency: &Dependency,
    dependency_target: &Target,
) -> bool {
    if let Some(value) = dependency.embed {
        return value;
    }
    if dependency.copy_phase.is_some() {
        return true;
    }
    if !default_embed_target_product(dependency_target) {
        return false;
    }
    if product_type_is_test(&parent_target.target_type) {
        return dependency_target.target_type.is_framework();
    }
    product_type_is_app(&parent_target.target_type)
        || matches!(
            parent_target.target_type,
            ProductType::AppExtension
                | ProductType::XcodeExtension
                | ProductType::IntentsServiceExtension
                | ProductType::WatchExtension
                | ProductType::Watch2Extension
                | ProductType::TvExtension
                | ProductType::MessagesExtension
                | ProductType::StickerPack
                | ProductType::ExtensionKitExtension
        )
}

fn default_embed_target_product(target: &Target) -> bool {
    if target.target_type == ProductType::StaticFramework || framework_builds_static_mach_o(target)
    {
        return false;
    }
    matches!(
        target.target_type,
        ProductType::Framework
            | ProductType::AppExtension
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
            | ProductType::MessagesExtension
            | ProductType::StickerPack
            | ProductType::ExtensionKitExtension
            | ProductType::XpcService
            | ProductType::SystemExtension
            | ProductType::DriverExtension
            | ProductType::OnDemandInstallCapableApplication
            | ProductType::WatchApp
            | ProductType::Watch2App
    )
}

fn framework_builds_static_mach_o(target: &Target) -> bool {
    target
        .settings_spec
        .build_settings
        .get("MACH_O_TYPE")
        .and_then(json_string_value)
        .is_some_and(|value| value == "staticlib")
}

fn is_watch_app_product(product_type: &ProductType) -> bool {
    matches!(product_type, ProductType::WatchApp | ProductType::Watch2App)
}

fn product_type_is_app(product_type: &ProductType) -> bool {
    matches!(
        product_type,
        ProductType::Application
            | ProductType::OnDemandInstallCapableApplication
            | ProductType::WatchApp
            | ProductType::Watch2App
            | ProductType::MessagesApplication
    )
}

fn product_type_is_test(product_type: &ProductType) -> bool {
    matches!(
        product_type,
        ProductType::UnitTestBundle | ProductType::UiTestBundle | ProductType::OcUnitTestBundle
    )
}

fn platform_requires_simulator_stripping(platform: &Platform) -> bool {
    matches!(
        platform,
        Platform::Auto | Platform::Ios | Platform::Tvos | Platform::Watchos | Platform::Visionos
    )
}

fn target_should_embed_carthage_dependencies(target: &Target) -> bool {
    (product_type_is_app(&target.target_type) && target.platform != Platform::Watchos)
        || target.target_type == ProductType::Watch2Extension
        || product_type_is_test(&target.target_type)
}

fn product_type_is_linkable(product_type: &ProductType) -> bool {
    product_type.is_framework() || product_type.is_library()
}

fn target_dependency_should_link(
    target: &Target,
    dependency: &Dependency,
    dependency_target: &Target,
) -> bool {
    if target.target_type.is_framework()
        && dependency_target.target_type == ProductType::StaticLibrary
    {
        return dependency.link.unwrap_or(false);
    }
    dependency
        .link
        .unwrap_or_else(|| product_type_is_linkable(&dependency_target.target_type))
        && target.target_type != ProductType::StaticLibrary
}

fn target_uses_transitive_dependencies(project: &Project, target: &Target) -> bool {
    target
        .transitively_link_dependencies
        .unwrap_or(project.spec_options.transitively_link_dependencies)
}

fn native_target_dependency_names(project: &Project, target: &Target) -> Vec<String> {
    let mut names = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();
    let mut queue = VecDeque::<String>::new();

    for dependency in &target.dependencies {
        if dependency.dependency_type != DependencyType::Target {
            continue;
        }
        if seen.insert(dependency.reference.clone()) {
            names.push(dependency.reference.clone());
            queue.push_back(dependency.reference.clone());
        }
    }

    if !target_uses_transitive_dependencies(project, target) {
        return names;
    }

    while let Some(dependency_name) = queue.pop_front() {
        let Some(dependency_target) = project.targets.get(&dependency_name) else {
            continue;
        };
        for dependency in &dependency_target.dependencies {
            if dependency.dependency_type != DependencyType::Target {
                continue;
            }
            if seen.insert(dependency.reference.clone()) {
                names.push(dependency.reference.clone());
                queue.push_back(dependency.reference.clone());
            }
        }
    }

    names
}

fn target_directly_embeds_carthage_dependencies(target: &Target) -> bool {
    target
        .directly_embed_carthage_dependencies
        .unwrap_or_else(|| {
            !(platform_requires_simulator_stripping(&target.platform)
                && (product_type_is_app(&target.target_type)
                    || target.target_type == ProductType::Watch2Extension))
        })
}

fn should_embed_carthage_dependency(target: &Target, dependency: &Dependency) -> bool {
    let DependencyType::Carthage { link_type, .. } = &dependency.dependency_type else {
        return false;
    };
    *link_type == crate::spec::CarthageLinkType::Dynamic
        && dependency
            .embed
            .unwrap_or_else(|| target_should_embed_carthage_dependencies(target))
}

fn ordered_group_children(
    options: &SpecOptions,
    group_name: &str,
    children: Vec<PbxValue>,
    is_group_or_folder: impl Fn(&PbxValue) -> bool,
) -> Vec<PbxValue> {
    let Some(ordering) = options
        .group_ordering
        .iter()
        .find(|ordering| {
            ordering.pattern.is_some()
                && group_ordering_matches(ordering.pattern.as_deref(), group_name)
        })
        .or_else(|| {
            if !group_name.is_empty() {
                return None;
            }
            options
                .group_ordering
                .iter()
                .find(|ordering| group_ordering_matches(ordering.pattern.as_deref(), group_name))
        })
    else {
        return children;
    };
    if ordering.order.is_empty() {
        return children;
    }

    let files = children
        .iter()
        .filter(|child| !is_group_or_folder(child))
        .cloned()
        .collect::<Vec<_>>();
    let mut groups = children
        .iter()
        .filter(|child| is_group_or_folder(child))
        .cloned()
        .collect::<Vec<_>>();

    let mut ordered_groups = Vec::new();
    for ordered_name in &ordering.order {
        let Some(child) = groups
            .iter()
            .find(|child| pbx_value_comment(child) == *ordered_name)
            .cloned()
        else {
            continue;
        };
        ordered_groups.push(child.clone());
        groups.retain(|group| group != &child);
    }
    let has_packages_group = groups
        .iter()
        .any(|group| pbx_value_comment(group) == "Packages");
    let has_products_group = groups
        .iter()
        .any(|group| pbx_value_comment(group) == "Products");
    let late_names: &[&str] = if group_name.is_empty() && has_packages_group {
        &["Packages", "Frameworks", "Products"]
    } else if has_products_group {
        &["Frameworks"]
    } else {
        &[]
    };
    let late_main_groups = if !late_names.is_empty() {
        groups
            .iter()
            .filter(|group| late_names.contains(&pbx_value_comment(group).as_str()))
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    groups.retain(|group| {
        late_names.is_empty() || !late_names.contains(&pbx_value_comment(group).as_str())
    });
    let products = groups
        .iter()
        .filter(|group| pbx_value_comment(group) == "Products")
        .cloned()
        .collect::<Vec<_>>();
    groups.retain(|group| pbx_value_comment(group) != "Products");
    ordered_groups.extend(groups);

    match effective_group_sort_position(options) {
        GroupSortPosition::Top => ordered_groups
            .into_iter()
            .chain(files)
            .chain(late_main_groups)
            .chain(products)
            .collect(),
        GroupSortPosition::Bottom => files
            .into_iter()
            .chain(ordered_groups)
            .chain(late_main_groups)
            .chain(products)
            .collect(),
    }
}

fn effective_group_sort_position(options: &SpecOptions) -> &GroupSortPosition {
    if !options.group_sort_position_explicit && options.group_ordering.is_empty() {
        &GroupSortPosition::Bottom
    } else {
        &options.group_sort_position
    }
}

fn group_ordering_matches(pattern: Option<&str>, group_name: &str) -> bool {
    let Some(pattern) = pattern else {
        return true;
    };
    if pattern == group_name {
        return true;
    }
    if let Some(suffix) = pattern
        .strip_prefix("^.*")
        .and_then(|value| value.strip_suffix('$'))
    {
        return group_name.ends_with(suffix);
    }
    group_name.contains(pattern)
}

fn pbx_value_comment(value: &PbxValue) -> String {
    match value {
        PbxValue::Ref { comment, .. } => comment.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn pbx_value_id(value: &PbxValue) -> Option<&str> {
    match value {
        PbxValue::Ref { id, .. } => Some(id),
        _ => None,
    }
}

fn pbx_string(value: &PbxValue) -> Option<&str> {
    match value {
        PbxValue::String(value) => Some(value),
        _ => None,
    }
}

fn project_xcode_version_last_upgrade_check(project: &Project) -> Option<String> {
    let value = project
        .options
        .get("xcodeVersion")
        .and_then(json_string_value)?;
    let mut parts = value.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    if major > 16 {
        return None;
    }
    Some(format!("{major}{minor:02}"))
}

fn should_embed_external_dependency(target: &Target, dependency: &Dependency) -> bool {
    if matches!(
        target.target_type,
        ProductType::Framework | ProductType::StaticFramework | ProductType::StaticLibrary
    ) && dependency.embed != Some(true)
    {
        return false;
    }
    match &dependency.dependency_type {
        DependencyType::Framework => dependency.embed.unwrap_or(true),
        DependencyType::Sdk { .. } => dependency.embed.unwrap_or(false),
        DependencyType::Package { .. } => dependency.embed.unwrap_or(false),
        DependencyType::Carthage { link_type, .. } => dependency
            .embed
            .unwrap_or(*link_type == crate::spec::CarthageLinkType::Dynamic),
        DependencyType::Target | DependencyType::Bundle => false,
    }
}

fn framework_dependency_reference_key(
    target_name: &str,
    dependency: &Dependency,
    path: &str,
) -> String {
    if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
        format!("sdkRef:{path}")
    } else {
        let _ = target_name;
        format!("navigatorFrameworkRef:{path}")
    }
}

fn sdk_reference_path(reference: &str, source_tree: &str) -> String {
    if source_tree != "SDKROOT" || reference.contains('/') {
        return reference.to_owned();
    }
    if reference.ends_with(".framework") {
        format!("System/Library/Frameworks/{reference}")
    } else if reference.ends_with(".tbd") || reference.ends_with(".dylib") {
        format!("usr/lib/{reference}")
    } else {
        reference.to_owned()
    }
}

fn copy_files_settings_for_embedded_dependency(
    dependency: &Dependency,
) -> Option<CopyFilesSettings> {
    match &dependency.dependency_type {
        DependencyType::Framework
        | DependencyType::Sdk { .. }
        | DependencyType::Carthage { .. }
        | DependencyType::Package { .. } => dependency
            .copy_phase
            .as_ref()
            .and_then(copy_files_settings_from_value)
            .or_else(|| {
                Some(CopyFilesSettings {
                    dst_subfolder_spec: 10,
                    dst_path: String::new(),
                    phase_name: "Embed Frameworks".to_owned(),
                    phase_order: CopyFilesPhaseOrder::PostCompile,
                })
            }),
        DependencyType::Target | DependencyType::Bundle => None,
    }
}

fn copy_files_settings_key(
    settings: &CopyFilesSettings,
) -> (i64, String, String, CopyFilesPhaseOrder) {
    (
        settings.dst_subfolder_spec,
        settings.dst_path.clone(),
        settings.phase_name.clone(),
        settings.phase_order,
    )
}

fn copy_files_destination_key(settings: &CopyFilesSettings) -> (i64, String, String) {
    (
        settings.dst_subfolder_spec,
        settings.dst_path.clone(),
        settings.phase_name.clone(),
    )
}

fn copy_files_phase_output_key(
    dst_subfolder_spec: i64,
    dst_path: &str,
    phase_name: &str,
) -> (i64, i64, String, String) {
    (
        copy_files_phase_name_order(phase_name),
        dst_subfolder_spec,
        dst_path.to_owned(),
        phase_name.to_owned(),
    )
}

fn copy_files_phase_name_order(phase_name: &str) -> i64 {
    match phase_name {
        "Embed Foundation Extensions" => 0,
        "Embed Frameworks" => 1,
        _ => 2,
    }
}

fn target_xcconfig_preserves_default(key: &str) -> bool {
    matches!(
        key,
        "CURRENT_PROJECT_VERSION"
            | "DYLIB_COMPATIBILITY_VERSION"
            | "DYLIB_CURRENT_VERSION"
            | "DYLIB_INSTALL_NAME_BASE"
            | "INSTALL_PATH"
            | "VERSIONING_SYSTEM"
    )
}

fn default_source_copy_files_settings() -> CopyFilesSettings {
    CopyFilesSettings {
        dst_subfolder_spec: 10,
        dst_path: String::new(),
        phase_name: "Embed Frameworks".to_owned(),
        phase_order: CopyFilesPhaseOrder::PostCompile,
    }
}

fn source_copy_files_settings(
    target: &Target,
    source: &TargetSource,
    path: &str,
) -> Option<CopyFilesSettings> {
    if let Some(settings) = source
        .build_phase
        .as_ref()
        .and_then(copy_files_settings_from_source_build_phase)
    {
        return Some(settings);
    }
    if is_module_copy_file(path) && target.target_type == ProductType::StaticLibrary {
        return Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "include/$(PRODUCT_NAME)".to_owned(),
            phase_name: "CopyFiles".to_owned(),
            phase_order: CopyFilesPhaseOrder::PreCompile,
        });
    }
    if is_module_copy_file(path) {
        return Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(PRODUCT_NAME).framework/Modules".to_owned(),
            phase_name: "CopyFiles".to_owned(),
            phase_order: CopyFilesPhaseOrder::PreCompile,
        });
    }
    if path.ends_with(".xpc") {
        return Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(CONTENTS_FOLDER_PATH)/XPCServices".to_owned(),
            phase_name: "CopyFiles".to_owned(),
            phase_order: CopyFilesPhaseOrder::PostCompile,
        });
    }
    Some(default_source_copy_files_settings())
}

fn copy_files_settings_from_source_build_phase(value: &Value) -> Option<CopyFilesSettings> {
    let map = value.as_object()?;
    map.get("copyFiles")
        .and_then(copy_files_settings_from_value)
        .map(|mut settings| {
            if settings.phase_name == "Copy Files" {
                settings.phase_name = "CopyFiles".to_owned();
            }
            settings
        })
}

fn copy_files_settings_from_value(value: &Value) -> Option<CopyFilesSettings> {
    let map = value.as_object()?;
    let destination = map
        .get("destination")
        .and_then(json_string_value)
        .unwrap_or_else(|| "frameworks".to_owned());
    let dst_subfolder_spec = match destination.as_str() {
        "absolute" => 0,
        "wrapper" => 1,
        "executables" => 6,
        "resources" => 7,
        "frameworks" => 10,
        "sharedFrameworks" => 11,
        "sharedSupport" => 12,
        "plugins" => 13,
        "javaResources" => 15,
        "productsDirectory" => 16,
        _ => return None,
    };
    Some(CopyFilesSettings {
        dst_subfolder_spec,
        dst_path: map
            .get("subpath")
            .and_then(json_string_value)
            .unwrap_or_default(),
        phase_name: match destination.as_str() {
            "frameworks" => "Embed Frameworks",
            "plugins" => "Embed Dependencies",
            _ => "Copy Files",
        }
        .to_owned(),
        phase_order: CopyFilesPhaseOrder::PostCompile,
    })
}

fn supported_destination_filter(filter: &str) -> Option<String> {
    Some(
        match filter {
            "iOS" | "ios" => "ios",
            "tvOS" | "tvos" => "tvos",
            "macOS" | "macos" => "macos",
            "macCatalyst" | "maccatalyst" => "maccatalyst",
            "watchOS" | "watchos" => "watchos",
            "visionOS" | "xros" => "xros",
            _ => return None,
        }
        .to_owned(),
    )
}

fn infer_platform_filter_from_path(path: &str) -> Option<String> {
    let path = path.to_lowercase();
    for (raw, filter) in [
        ("ios", "ios"),
        ("tvos", "tvos"),
        ("macos", "macos"),
        ("maccatalyst", "maccatalyst"),
        ("watchos", "watchos"),
        ("visionos", "xros"),
    ] {
        if path.contains(&format!("/{raw}/")) || path.ends_with(&format!("_{raw}.swift")) {
            return Some(filter.to_owned());
        }
    }
    None
}

fn target_sources_require_objc_linking(base_path: &Path, target: &Target) -> bool {
    target
        .sources
        .iter()
        .any(|source| source_path_requires_objc_linking(&base_path.join(&source.path)))
}

fn source_path_requires_objc_linking(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "m" | "mm"))
    {
        return true;
    }
    if !path.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return false;
    };
    entries
        .flatten()
        .any(|entry| source_path_requires_objc_linking(&entry.path()))
}

fn localized_variant_group_path(base_path: &Path, path: &Path) -> Option<String> {
    let group_name = localized_variant_group_name(path)?;
    let variant_parent = path.parent()?.parent()?;
    let mut relative = pathdiff(variant_parent, base_path);
    relative.push(group_name);
    Some(relative.to_string_lossy().into_owned())
}

fn localized_variant_group_is_direct_child(source_root: &Path, path: &Path) -> bool {
    path.parent().and_then(Path::parent) == Some(source_root)
}

fn localized_variant_group_matches_source(
    source_root: &Path,
    source: &TargetSource,
    path: &Path,
) -> bool {
    if is_localized_interface_file(path) {
        return true;
    }
    if path.extension().and_then(|extension| extension.to_str()) == Some("strings") {
        return source_root.is_dir()
            && path.starts_with(source_root)
            && !source
                .excludes
                .iter()
                .any(|pattern| pattern.contains(".lproj/**"));
    }
    localized_variant_group_is_direct_child(source_root, path)
}

fn is_localized_file(path: &Path) -> bool {
    path.parent()
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        == Some("lproj")
}

fn is_localized_interface_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "storyboard" | "xib"))
}

fn localized_variant_group_name(path: &Path) -> Option<String> {
    let lproj_dir = path.parent()?;
    if lproj_dir
        .extension()
        .and_then(|extension| extension.to_str())
        != Some("lproj")
    {
        return None;
    }
    let file_name = path.file_name()?.to_str()?;
    if path.extension().and_then(|extension| extension.to_str()) == Some("strings") {
        let stem = path.file_stem()?.to_str()?;
        for extension in ["storyboard", "xib"] {
            let parent = lproj_dir.parent()?;
            let has_base_file = fs::read_dir(parent).ok()?.flatten().any(|entry| {
                let candidate = entry.path().join(format!("{stem}.{extension}"));
                candidate.exists()
            });
            if has_base_file {
                return Some(format!("{stem}.{extension}"));
            }
        }
    }
    Some(file_name.to_owned())
}

fn localized_variant_files(parent: &Path, group_name: &str) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(parent) else {
        return files;
    };
    let stem = Path::new(group_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(group_name);
    for entry in entries.flatten() {
        let lproj = entry.path();
        if lproj.extension().and_then(|extension| extension.to_str()) != Some("lproj") {
            continue;
        }
        let primary = lproj.join(group_name);
        if primary.exists() {
            files.push(primary);
            continue;
        }
        let strings = lproj.join(format!("{stem}.strings"));
        if strings.exists() {
            files.push(strings);
        }
    }
    files
}

fn localized_file_locale(path: &Path) -> Option<String> {
    Some(path.parent()?.file_stem()?.to_str()?.to_owned())
}

fn bool_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
}

fn boolish_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.as_str() {
            "true" | "True" | "TRUE" | "yes" | "YES" => Some(true),
            "false" | "False" | "FALSE" | "no" | "NO" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn is_legacy_target(target: &Target) -> bool {
    target.target_type == ProductType::Other(String::new())
}

fn product_uses_explicit_file_type(target: &Target) -> bool {
    matches!(
        target.target_type,
        ProductType::Framework
            | ProductType::StaticFramework
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
            | ProductType::XpcService
            | ProductType::SystemExtension
            | ProductType::DriverExtension
            | ProductType::ExtensionKitExtension
    ) || matches!(
        target.target_type,
        ProductType::Application
            | ProductType::UnitTestBundle
            | ProductType::UiTestBundle
            | ProductType::OcUnitTestBundle
            | ProductType::StaticLibrary
    ) && matches!(target.platform, Platform::Macos | Platform::Watchos)
        || matches!(
            target.target_type,
            ProductType::WatchApp | ProductType::Watch2App
        )
}

fn build_script_key(script: &BuildScript) -> String {
    let script_text = match &script.script {
        BuildScriptKind::Script(script) => script.as_str(),
        BuildScriptKind::Path(path) => path.as_str(),
    };
    format!(
        "{}:{}:{}:{}",
        script.name.as_deref().unwrap_or("Run Script"),
        script_text,
        script.input_files.join(","),
        script.output_files.join(",")
    )
}

fn json_string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn json_bool_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.as_str() {
            "YES" | "yes" | "true" | "TRUE" | "1" => Some(true),
            "NO" | "no" | "false" | "FALSE" | "0" => Some(false),
            _ => None,
        },
        Value::Number(number) => number.as_i64().map(|number| number != 0),
        _ => None,
    }
}

fn normalize_build_setting_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            path.find("$(")
                .filter(|index| *index > 0 && path[..*index].ends_with('/'))
                .map(|index| path[index..].to_owned())
                .unwrap_or_else(|| path.clone())
        })
        .collect()
}

fn config_matches_variant(
    config: &str,
    variant: &str,
    configs: &indexmap::IndexMap<String, String>,
) -> bool {
    if !config.to_lowercase().contains(&variant.to_lowercase()) {
        return false;
    }
    !configs.contains_key(variant) || config == variant
}

fn is_static_plist_path(path: &str) -> bool {
    !path.contains("$(") && !path.contains("${")
}

fn default_info_plist_from_sources(base_path: &Path, target: &Target) -> Option<String> {
    target.sources.iter().find_map(|source| {
        let source_path = base_path.join(&source.path);
        if source_path.is_file() {
            (source_path.file_name().and_then(|name| name.to_str()) == Some("Info.plist"))
                .then(|| source.path.clone())
        } else if source_path.is_dir() {
            find_info_plist(&source_path)
                .map(|path| pathdiff(&path, base_path).to_string_lossy().into_owned())
        } else {
            None
        }
    })
}

fn find_info_plist(path: &Path) -> Option<PathBuf> {
    let mut entries = fs::read_dir(path)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    entries.sort();
    for entry in entries {
        if entry.file_name().and_then(|name| name.to_str()) == Some("Info.plist") {
            return Some(entry);
        }
        if entry.is_dir() && !is_wrapper_path(&entry) {
            if let Some(path) = find_info_plist(&entry) {
                return Some(path);
            }
        }
    }
    None
}


fn expand_source_path(
    base_path: &Path,
    source: &TargetSource,
    file_types: &indexmap::IndexMap<String, FileType>,
) -> Vec<PathBuf> {
    let path = base_path.join(&source.path);
    if source.source_type == Some(SourceType::Folder) {
        return if source_matches_filters(&path, &path, source) {
            vec![path]
        } else {
            Vec::new()
        };
    }
    if path.is_file() {
        return if source_matches_filters(&path, &path, source) {
            vec![path]
        } else {
            Vec::new()
        };
    }
    if !path.is_dir() {
        return if source_matches_filters(&path, &path, source) {
            vec![path]
        } else {
            Vec::new()
        };
    }
    if is_wrapper_path(&path) {
        return if source_matches_filters(&path, &path, source) {
            vec![path]
        } else {
            Vec::new()
        };
    }
    let mut files = Vec::new();
    collect_files(&path, &mut files, file_types);
    files.retain(|file| source_matches_filters(&path, file, source));
    files
}

fn explicit_folders_for_synced_source(base_path: &Path, source: &TargetSource) -> Vec<String> {
    if source.explicit_folders.is_empty() {
        return Vec::new();
    }
    let source_root = base_path.join(&source.path);
    let mut folders = Vec::new();
    collect_directories(&source_root, &mut folders);
    let mut explicit = Vec::new();
    for folder in folders {
        let relative = folder
            .strip_prefix(&source_root)
            .unwrap_or(&folder)
            .to_string_lossy()
            .into_owned();
        if source
            .explicit_folders
            .iter()
            .any(|pattern| explicit_folder_pattern_matches(pattern, &relative))
        {
            explicit.push(relative);
        }
    }
    for folder in &source.explicit_folders {
        if !folder.contains('*') && !explicit.contains(folder) {
            explicit.push(folder.clone());
        }
    }
    explicit.sort();
    explicit
}

fn explicit_folder_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    let path = path.trim_start_matches("./");
    if pattern.is_empty() {
        return false;
    }
    if path == pattern {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return wildcard_match(suffix, path)
            || Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| wildcard_match(suffix, name));
    }
    wildcard_match(pattern, path)
}

fn synced_folder_membership_exceptions(
    base_path: &Path,
    source: &TargetSource,
    info_plist_files: &BTreeSet<String>,
) -> Vec<String> {
    let source_root = base_path.join(&source.path);
    if !source_root.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    collect_files(&source_root, &mut files, &indexmap::IndexMap::new());
    let mut exceptions = files
        .into_iter()
        .filter_map(|file| {
            let relative_to_source = file
                .strip_prefix(&source_root)
                .unwrap_or(&file)
                .to_string_lossy()
                .into_owned();
            let relative_to_project = Path::new(&source.path)
                .join(&relative_to_source)
                .to_string_lossy()
                .into_owned();
            (!source_matches_filters(&source_root, &file, source)
                || info_plist_files.contains(&relative_to_project))
            .then_some(relative_to_source)
        })
        .collect::<Vec<_>>();
    exceptions.sort();
    exceptions
}

fn model_current_version_name(model_group: &Path) -> Option<String> {
    let contents = fs::read_to_string(model_group.join(".xccurrentversion")).ok()?;
    let key_index = contents.find("<key>_XCCurrentVersionName</key>")?;
    let after_key = &contents[key_index..];
    let start_tag = after_key.find("<string>")? + "<string>".len();
    let after_start = &after_key[start_tag..];
    let end_tag = after_start.find("</string>")?;
    Some(after_start[..end_tag].to_owned())
}

fn collect_directories(path: &Path, directories: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            directories.push(path.clone());
            collect_directories(&path, directories);
        }
    }
}

fn source_matches_filters(source_root: &Path, file: &Path, source: &TargetSource) -> bool {
    let relative_to_source = file
        .strip_prefix(source_root)
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned();
    let relative_to_declared = Path::new(&source.path)
        .join(&relative_to_source)
        .to_string_lossy()
        .into_owned();
    if source.excludes.iter().any(|pattern| {
        source_pattern_matches(pattern, &relative_to_source)
            || source_pattern_matches(pattern, &relative_to_declared)
    }) {
        return false;
    }
    source.includes.is_empty()
        || source.includes.iter().any(|pattern| {
            source_pattern_matches(pattern, &relative_to_source)
                || source_pattern_matches(pattern, &relative_to_declared)
        })
}

fn source_excludes_directory(source_root: &Path, directory: &Path, source: &TargetSource) -> bool {
    let relative_to_source = directory
        .strip_prefix(source_root)
        .unwrap_or(directory)
        .to_string_lossy()
        .into_owned();
    let relative_to_declared = Path::new(&source.path)
        .join(&relative_to_source)
        .to_string_lossy()
        .into_owned();
    source.excludes.iter().any(|pattern| {
        source_pattern_matches(pattern, &relative_to_source)
            || source_pattern_matches(pattern, &relative_to_declared)
    })
}

fn source_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    let path = path.trim_start_matches("./");
    if pattern.is_empty() {
        return false;
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return path == suffix
            || path.starts_with(&format!("{suffix}/"))
            || path
                .split('/')
                .scan(String::new(), |prefix, part| {
                    if prefix.is_empty() {
                        prefix.push_str(part);
                    } else {
                        prefix.push('/');
                        prefix.push_str(part);
                    }
                    Some(path.strip_prefix(&format!("{prefix}/")).unwrap_or(""))
                })
                .any(|tail| !tail.is_empty() && source_pattern_matches(suffix, tail))
            || wildcard_match(suffix, path)
            || path.split('/').any(|part| wildcard_match(suffix, part));
    }
    if let Some(prefix) = pattern.strip_suffix("/**") {
        if prefix.contains('*') {
            return path
                .split('/')
                .scan(String::new(), |ancestor, part| {
                    if ancestor.is_empty() {
                        ancestor.push_str(part);
                    } else {
                        ancestor.push('/');
                        ancestor.push_str(part);
                    }
                    Some(ancestor.clone())
                })
                .any(|ancestor| wildcard_match(prefix, &ancestor));
        }
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    if path == pattern || path.starts_with(&format!("{pattern}/")) {
        return true;
    }
    wildcard_match(pattern, path)
        || Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| wildcard_match(pattern, name))
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn wildcard_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard_match_inner(&pattern[1..], text)
                || (!text.is_empty() && wildcard_match_inner(pattern, &text[1..]))
        }
        b'?' => !text.is_empty() && wildcard_match_inner(&pattern[1..], &text[1..]),
        b'[' => {
            if text.is_empty() {
                return false;
            }
            let Some(end) = pattern.iter().position(|byte| *byte == b']') else {
                return pattern[0] == text[0] && wildcard_match_inner(&pattern[1..], &text[1..]);
            };
            wildcard_class_matches(&pattern[1..end], text[0])
                && wildcard_match_inner(&pattern[end + 1..], &text[1..])
        }
        _ => {
            !text.is_empty()
                && pattern[0] == text[0]
                && wildcard_match_inner(&pattern[1..], &text[1..])
        }
    }
}

fn wildcard_class_matches(class: &[u8], value: u8) -> bool {
    let mut index = 0;
    while index < class.len() {
        if index + 2 < class.len() && class[index + 1] == b'-' {
            if class[index] <= value && value <= class[index + 2] {
                return true;
            }
            index += 3;
        } else {
            if class[index] == value {
                return true;
            }
            index += 1;
        }
    }
    false
}

fn collect_files(
    path: &Path,
    files: &mut Vec<PathBuf>,
    file_types: &indexmap::IndexMap<String, FileType>,
) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if (name.starts_with('.') && name != ".swiftlint.yml")
            || name == "Carthage"
            || name.ends_with(".xcodeproj")
            || path.extension().and_then(|extension| extension.to_str()) == Some("orig")
        {
            continue;
        }
        let custom_file_type = path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| file_types.get(extension));
        if path.is_dir()
            && !is_wrapper_path(&path)
            && !custom_file_type.is_some_and(|file_type| file_type.file)
        {
            collect_files(&path, files, file_types);
        } else {
            files.push(path);
        }
    }
}

fn collect_known_regions_for_source(
    source_root: &Path,
    path: &Path,
    source: &TargetSource,
    regions: &mut BTreeSet<String>,
) {
    if path.is_file() {
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.extension().and_then(|extension| extension.to_str()) == Some("lproj")
            && source_matches_filters(source_root, &path, source)
        {
            if let Some(region) = path.file_stem().and_then(|stem| stem.to_str()) {
                regions.insert(region.to_owned());
            }
        }
        collect_known_regions_for_source(source_root, &path, source, regions);
    }
}

fn is_wrapper_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension,
                "app"
                    | "bundle"
                    | "docc"
                    | "icon"
                    | "playground"
                    | "scnassets"
                    | "swiftcrossimport"
                    | "swiftoverlay"
                    | "framework"
                    | "xpc"
                    | "xcassets"
                    | "xcdatamodeld"
                    | "xcmappingmodel"
                    | "xctestplan"
            )
        })
}

fn pathdiff(path: &Path, base: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}

fn display_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_owned()
}

fn natural_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left.len() && right_index < right.len() {
        let left_is_digit = left[left_index].is_ascii_digit();
        let right_is_digit = right[right_index].is_ascii_digit();

        if left_is_digit && right_is_digit {
            let left_start = left_index;
            let right_start = right_index;
            while left_index < left.len() && left[left_index].is_ascii_digit() {
                left_index += 1;
            }
            while right_index < right.len() && right[right_index].is_ascii_digit() {
                right_index += 1;
            }
            let left_number = std::str::from_utf8(&left[left_start..left_index])
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let right_number = std::str::from_utf8(&right[right_start..right_index])
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            match left_number.cmp(&right_number) {
                std::cmp::Ordering::Equal => {}
                ordering => return ordering,
            }
            continue;
        }

        let left_byte = left[left_index];
        let right_byte = right[right_index];
        match left_byte.cmp(&right_byte) {
            std::cmp::Ordering::Equal => {
                left_index += 1;
                right_index += 1;
            }
            ordering => return ordering,
        }
    }

    left.len().cmp(&right.len())
}

fn natural_group_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut left_index = 0;
    let mut right_index = 0;

    while left_index < left.len() && right_index < right.len() {
        let left_is_digit = left[left_index].is_ascii_digit();
        let right_is_digit = right[right_index].is_ascii_digit();

        if left_is_digit && right_is_digit {
            let left_start = left_index;
            let right_start = right_index;
            while left_index < left.len() && left[left_index].is_ascii_digit() {
                left_index += 1;
            }
            while right_index < right.len() && right[right_index].is_ascii_digit() {
                right_index += 1;
            }
            let left_number = std::str::from_utf8(&left[left_start..left_index])
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            let right_number = std::str::from_utf8(&right[right_start..right_index])
                .ok()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0);
            match left_number.cmp(&right_number) {
                std::cmp::Ordering::Equal => {}
                ordering => return ordering,
            }
            continue;
        }

        let left_byte = natural_sort_byte(left[left_index].to_ascii_lowercase());
        let right_byte = natural_sort_byte(right[right_index].to_ascii_lowercase());
        match left_byte.cmp(&right_byte) {
            std::cmp::Ordering::Equal => {
                left_index += 1;
                right_index += 1;
            }
            ordering => return ordering,
        }
    }

    left.len().cmp(&right.len())
}

fn natural_sort_byte(byte: u8) -> u8 {
    if byte == b'_' {
        0
    } else if byte == b'-' {
        b')'
    } else if byte == b'.' {
        b'*'
    } else {
        byte
    }
}

fn build_phase_for_source(path: &str) -> Option<&'static str> {
    if is_copy_files_source(path) {
        Some("CopyFiles")
    } else if is_header_file(path) {
        Some("Headers")
    } else if is_source_file(path) {
        Some("Sources")
    } else if is_unphased_file(path) {
        None
    } else if has_extension(path) {
        Some("Resources")
    } else {
        None
    }
}

fn build_phase_for_file_type(file_type: &FileType) -> Option<&'static str> {
    match file_type.build_phase.as_ref() {
        Some(FileBuildPhase::Sources) => Some("Sources"),
        Some(FileBuildPhase::Resources) => Some("Resources"),
        Some(FileBuildPhase::Headers) => Some("Headers"),
        Some(FileBuildPhase::None) => None,
        Some(FileBuildPhase::Other(_)) | None => None,
    }
}

fn file_type_options<'a>(
    path: &str,
    file_types: &'a indexmap::IndexMap<String, FileType>,
) -> Option<&'a FileType> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .and_then(|extension| file_types.get(extension))
}

fn target_supports_headers_phase(target: &Target) -> bool {
    matches!(
        target.target_type,
        ProductType::Framework | ProductType::StaticFramework | ProductType::DynamicLibrary
    )
}

fn is_public_header_source(source: &TargetSource) -> bool {
    source
        .header_visibility
        .as_deref()
        .unwrap_or("public")
        .eq_ignore_ascii_case("public")
}

fn is_source_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some(
            "swift"
                | "gyb"
                | "m"
                | "mm"
                | "c"
                | "cc"
                | "cpp"
                | "cp"
                | "cxx"
                | "S"
                | "xcdatamodeld"
                | "xcmappingmodel"
                | "intentdefinition"
                | "metal"
                | "mlmodel"
                | "mlpackage"
                | "rcproject"
                | "iig"
                | "docc",
        )
    )
}

fn is_header_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("h" | "hh" | "hpp" | "ipp" | "tpp" | "hxx" | "def")
    )
}

fn is_copy_files_source(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some(
            "framework" | "modulemap" | "swiftcrossimport" | "swiftoverlay" | "xcframework" | "xpc",
        )
    )
}

fn is_module_copy_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("modulemap" | "swiftcrossimport" | "swiftoverlay")
    )
}

fn is_unphased_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some(
            "xcconfig"
                | "entitlements"
                | "gpx"
                | "lproj"
                | "xcfilelist"
                | "apns"
                | "pch"
                | "xctestplan"
        )
    )
}

fn has_extension(path: &str) -> bool {
    Path::new(path).extension().is_some()
}

fn file_type_for_path(path: &str, product_type: Option<&ProductType>) -> String {
    if let Some(product_type) = product_type {
        return match product_type {
            ProductType::Application
            | ProductType::OnDemandInstallCapableApplication
            | ProductType::WatchApp
            | ProductType::Watch2App
            | ProductType::MessagesApplication => "wrapper.application",
            ProductType::Framework | ProductType::StaticFramework => "wrapper.framework",
            ProductType::StaticLibrary => "archive.ar",
            ProductType::DynamicLibrary => "compiled.mach-o.dylib",
            ProductType::Bundle => "wrapper.cfbundle",
            ProductType::UnitTestBundle
            | ProductType::UiTestBundle
            | ProductType::OcUnitTestBundle => "wrapper.cfbundle",
            ProductType::CommandLineTool => "compiled.mach-o.executable",
            ProductType::AppExtension
            | ProductType::XcodeExtension
            | ProductType::IntentsServiceExtension
            | ProductType::MessagesExtension
            | ProductType::StickerPack
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension => "wrapper.app-extension",
            ProductType::XpcService => "wrapper.xpc-service",
            ProductType::InstrumentsPackage => "wrapper.cfbundle",
            ProductType::MetalLibrary => "archive.metal-library",
            ProductType::SystemExtension => "wrapper.system-extension",
            ProductType::DriverExtension => "wrapper.driver-extension",
            _ => "wrapper.cfbundle",
        }
        .to_owned();
    }

    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
    {
        Some("swift") => "sourcecode.swift",
        Some("d") => "sourcecode.dtrace",
        Some("m") => "sourcecode.c.objc",
        Some("mm") => "sourcecode.cpp.objcpp",
        Some("cpp") | Some("cc") | Some("cxx") | Some("cp") => "sourcecode.cpp.cpp",
        Some("h") | Some("hh") | Some("hpp") | Some("ipp") | Some("tpp") | Some("hxx")
        | Some("def") => "sourcecode.c.h",
        Some("plist") => "text.plist",
        Some("txt") => "text",
        Some("yml") | Some("yaml") => "text.yaml",
        Some("md") | Some("markdown") => "net.daringfireball.markdown",
        Some("json") => "text.json",
        Some("html") => "text.html",
        Some("sh") => "text.script.sh",
        Some("png") => "image.png",
        Some("gif") => "image.gif",
        Some("jpg") | Some("jpeg") => "image.jpeg",
        Some("mp3") => "audio.mp3",
        Some("dae") => "text.xml.dae",
        Some("js") => "sourcecode.javascript",
        Some("metal") => "sourcecode.metal",
        Some("strings") => "text.plist.strings",
        Some("stringsdict") => "text.plist.stringsdict",
        Some("entitlements") => "text.plist.entitlements",
        Some("xcconfig") => "text.xcconfig",
        Some("xcfilelist") => "text.xcfilelist",
        Some("xcstrings") => "text.json.xcstrings",
        Some("apns") => "text",
        Some("playground") => "file.playground",
        Some("xcassets") => "folder.assetcatalog",
        Some("xcdatamodel") => "wrapper.xcdatamodel",
        Some("xcmappingmodel") => "wrapper.xcmappingmodel",
        Some("docc") => "folder.documentationcatalog",
        Some("icon") => "wrapper.icon",
        Some("iig") => "sourcecode.iig",
        Some("modulemap") => "sourcecode.module-map",
        Some("scnassets") => "wrapper.scnassets",
        Some("swiftcrossimport") => "wrapper.swiftcrossimport",
        Some("swiftoverlay") => "file",
        Some("storyboard") => "file.storyboard",
        Some("xib") => "file.xib",
        Some("framework") => "wrapper.framework",
        Some("xcframework") => "wrapper.xcframework",
        Some("xpc") => "wrapper.xpc-service",
        Some("a") => "archive.ar",
        Some("bin") => "archive.macbinary",
        Some("zip") => "archive.zip",
        Some("dylib") => "compiled.mach-o.dylib",
        Some("tbd") => "sourcecode.text-based-dylib-definition",
        Some("bundle") => "wrapper.cfbundle",
        Some("xctestplan") => "text",
        _ => "file",
    }
    .to_owned()
}

fn product_type_raw(product_type: &ProductType) -> &'static str {
    match product_type {
        ProductType::Application => "com.apple.product-type.application",
        ProductType::OnDemandInstallCapableApplication => {
            "com.apple.product-type.application.on-demand-install-capable"
        }
        ProductType::Framework => "com.apple.product-type.framework",
        ProductType::StaticFramework => "com.apple.product-type.framework.static",
        ProductType::DynamicLibrary => "com.apple.product-type.library.dynamic",
        ProductType::StaticLibrary => "com.apple.product-type.library.static",
        ProductType::Bundle => "com.apple.product-type.bundle",
        ProductType::UnitTestBundle => "com.apple.product-type.bundle.unit-test",
        ProductType::UiTestBundle => "com.apple.product-type.bundle.ui-testing",
        ProductType::OcUnitTestBundle => "com.apple.product-type.bundle.ocunit-test",
        ProductType::AppExtension => "com.apple.product-type.app-extension",
        ProductType::XcodeExtension => "com.apple.product-type.xcode-extension",
        ProductType::IntentsServiceExtension => {
            "com.apple.product-type.app-extension.intents-service"
        }
        ProductType::CommandLineTool => "com.apple.product-type.tool",
        ProductType::WatchApp => "com.apple.product-type.application.watchapp",
        ProductType::Watch2App => "com.apple.product-type.application.watchapp2",
        ProductType::WatchExtension => "com.apple.product-type.watchkit-extension",
        ProductType::Watch2Extension => "com.apple.product-type.watchkit2-extension",
        ProductType::TvExtension => "com.apple.product-type.tv-app-extension",
        ProductType::MessagesApplication => "com.apple.product-type.application.messages",
        ProductType::MessagesExtension => "com.apple.product-type.app-extension.messages",
        ProductType::StickerPack => "com.apple.product-type.app-extension.messages-sticker-pack",
        ProductType::XpcService => "com.apple.product-type.xpc-service",
        ProductType::InstrumentsPackage => "com.apple.product-type.instruments-package",
        ProductType::MetalLibrary => "com.apple.product-type.metal-library",
        ProductType::SystemExtension => "com.apple.product-type.system-extension",
        ProductType::ExtensionKitExtension => "com.apple.product-type.extensionkit-extension",
        ProductType::DriverExtension => "com.apple.product-type.driver-extension",
        ProductType::Other(_) => "com.apple.product-type",
    }
}

fn package_url(package: &serde_json::Value) -> Option<String> {
    package
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .or_else(|| {
            package
                .get("github")
                .and_then(|value| value.as_str())
                .map(|repo| format!("https://github.com/{repo}"))
        })
}

fn package_reference_comment(package_name: &str, url: &str) -> String {
    let trimmed = url.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .and_then(|component| component.strip_suffix(".git").or(Some(component)))
        .filter(|component| !component.is_empty())
        .unwrap_or(package_name)
        .to_owned()
}

fn package_requirement(package: &serde_json::Value) -> BTreeMap<String, PbxValue> {
    let mut requirement = BTreeMap::new();
    if let Some(version) = package.get("exactVersion").and_then(json_string_value) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("exactVersion".to_owned()),
        );
        requirement.insert("version".to_owned(), PbxValue::String(version));
    } else if let Some(version) = package.get("version").and_then(json_string_value) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("exactVersion".to_owned()),
        );
        requirement.insert("version".to_owned(), PbxValue::String(version));
    } else if let Some(version) = package
        .get("majorVersion")
        .or_else(|| package.get("from"))
        .and_then(json_string_value)
    {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("upToNextMajorVersion".to_owned()),
        );
        requirement.insert("minimumVersion".to_owned(), PbxValue::String(version));
    } else if let Some(version) = package.get("minorVersion").and_then(json_string_value) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("upToNextMinorVersion".to_owned()),
        );
        requirement.insert("minimumVersion".to_owned(), PbxValue::String(version));
    } else if let Some(branch) = package.get("branch").and_then(json_string_value) {
        requirement.insert("kind".to_owned(), PbxValue::String("branch".to_owned()));
        requirement.insert("branch".to_owned(), PbxValue::String(branch));
    } else if let Some(revision) = package.get("revision").and_then(json_string_value) {
        requirement.insert("kind".to_owned(), PbxValue::String("revision".to_owned()));
        requirement.insert("revision".to_owned(), PbxValue::String(revision));
    } else if let Some(min) = package.get("minVersion").and_then(json_string_value) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("versionRange".to_owned()),
        );
        requirement.insert("minimumVersion".to_owned(), PbxValue::String(min));
        if let Some(max) = package.get("maxVersion").and_then(json_string_value) {
            requirement.insert("maximumVersion".to_owned(), PbxValue::String(max));
        }
    } else {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("upToNextMajorVersion".to_owned()),
        );
        requirement.insert(
            "minimumVersion".to_owned(),
            PbxValue::String("0.0.0".to_owned()),
        );
    }
    requirement
}
