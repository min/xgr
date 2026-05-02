use crate::spec::{
    AggregateTarget, BuildRule, BuildRuleAction, BuildRuleFileType, BuildScript, BuildScriptKind,
    Dependency, DependencyType, FileBuildPhase, FileType, Platform, Plist, ProductType, Project,
    Settings, SourceType, SpecError, SpecOptions, Target, TargetSource,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProjectWriteError {
    #[error(transparent)]
    Spec(#[from] SpecError),
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct GeneratedProject {
    pub project_path: PathBuf,
    pub pbxproj: String,
    pub workspace_data: String,
}

#[derive(Debug, Default)]
pub struct ProjectWriter;

#[derive(Debug, Clone)]
struct PbxObject {
    isa: &'static str,
    comment: Option<String>,
    fields: BTreeMap<String, PbxValue>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PbxValue {
    Int(i64),
    String(String),
    Ref { id: String, comment: Option<String> },
    Array(Vec<PbxValue>),
    Dict(BTreeMap<String, PbxValue>),
}

#[derive(Debug, Default)]
struct PbxGraph {
    objects: BTreeMap<String, PbxObject>,
    comments: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct TargetBuildRefs {
    target_id: String,
    product_ref_id: String,
}

#[derive(Debug, Clone)]
struct FileBuildRefs {
    build_file_id: Option<String>,
    name: String,
    build_phase: Option<&'static str>,
}

#[derive(Debug, Clone)]
struct CopyFilesSettings {
    dst_subfolder_spec: i64,
    dst_path: String,
    phase_name: String,
}

struct SchemeManagementState {
    name: String,
    shared: bool,
    is_shown: Option<bool>,
    order_hint: Option<i64>,
}

impl ProjectWriter {
    pub fn generate(project: &Project) -> GeneratedProject {
        let project_path = project.default_project_path();
        let mut generator = PbxGenerator::new(project);
        let pbxproj = generator.generate();
        let workspace_data = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Workspace version=\"1.0\">\n   <FileRef location=\"self:\"></FileRef>\n</Workspace>\n"
        );
        GeneratedProject {
            project_path,
            pbxproj,
            workspace_data,
        }
    }

    pub fn write(
        project: &Project,
        output: Option<&Path>,
    ) -> Result<GeneratedProject, ProjectWriteError> {
        let mut generated = Self::generate(project);
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
        write_plists(project)?;
        write_schemes(project, &generated.project_path)?;
        write_scheme_management(project, &generated.project_path)?;
        write_breakpoints(project, &generated.project_path)?;
        Ok(generated)
    }
}

struct PbxGenerator<'a> {
    project: &'a Project,
    graph: PbxGraph,
    target_refs: HashMap<String, TargetBuildRefs>,
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
            package_refs: HashMap::new(),
            project_package_refs: Vec::new(),
            product_ref_ids: Vec::new(),
        }
    }

    fn generate(&mut self) -> String {
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
            let file_ref = self.add_file_reference(
                &format!("fileGroup:{group}"),
                display_name(group),
                Some(group.clone()),
                Some(file_type_for_path(group, None)),
                None,
                "SOURCE_ROOT",
                true,
            );
            main_children.push(PbxValue::reference(file_ref, display_name(group)));
        }

        main_children.extend(self.add_source_navigator_groups());
        main_children.extend(self.add_package_references());
        if let Some(frameworks_group) = self.add_frameworks_navigator_group() {
            main_children.push(frameworks_group);
        }

        for target in self.project.targets.values() {
            let product_ref = self.add_product_reference(target);
            let refs = TargetBuildRefs {
                target_id: String::new(),
                product_ref_id: product_ref.clone(),
            };
            self.target_refs.insert(target.name.clone(), refs);
            self.product_ref_ids.push(product_ref);
        }

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
        main_children.push(PbxValue::reference(
            product_group_id.clone(),
            "Products".to_owned(),
        ));

        let main_group_id = self.add_group("mainGroup", None, None, main_children);

        let mut target_ids = Vec::new();
        for target in self.project.targets.values() {
            let target_id = self.add_native_target(target);
            if let Some(target_refs) = self.target_refs.get_mut(&target.name) {
                target_refs.target_id = target_id.clone();
            }
            target_ids.push(PbxValue::reference(target_id, target.name.clone()));
        }

        let mut aggregate_target_ids = Vec::new();
        for aggregate in self.project.aggregate_target_specs.values() {
            let aggregate_id = self.add_aggregate_target(aggregate);
            aggregate_target_ids.push(PbxValue::reference(aggregate_id, aggregate.name.clone()));
        }
        target_ids.extend(aggregate_target_ids);

        let project_id = self.graph.add(
            "project",
            PbxObject::new("PBXProject", "Project object")
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
                    "compatibilityVersion",
                    PbxValue::String("Xcode 16.0".to_owned()),
                )
                .field(
                    "developmentRegion",
                    PbxValue::String(self.development_language().to_owned()),
                )
                .field("hasScannedForEncodings", PbxValue::Int(0))
                .field("knownRegions", string_array(&self.known_regions()))
                .field("mainGroup", PbxValue::reference(main_group_id, ""))
                .field("minimizedProjectReferenceProxies", PbxValue::Int(1))
                .field(
                    "packageReferences",
                    PbxValue::Array(self.project_package_refs.clone()),
                )
                .field("preferredProjectObjectVersion", PbxValue::Int(77))
                .field(
                    "productRefGroup",
                    PbxValue::reference(product_group_id, "Products"),
                )
                .field("projectDirPath", PbxValue::String(String::new()))
                .field("projectRoot", PbxValue::String(String::new()))
                .field("targets", PbxValue::Array(target_ids)),
        );

        self.serialize(&project_id)
    }

    fn project_attributes(&self) -> BTreeMap<String, PbxValue> {
        let mut attributes = BTreeMap::new();
        let last_upgrade_check = self
            .project
            .attributes
            .get("LastUpgradeCheck")
            .and_then(|value| value.as_str())
            .unwrap_or("1600");
        attributes.insert(
            "LastUpgradeCheck".to_owned(),
            PbxValue::String(last_upgrade_check.to_owned()),
        );
        let target_attributes = self.target_attributes();
        if !target_attributes.is_empty() {
            attributes.insert(
                "TargetAttributes".to_owned(),
                PbxValue::Dict(target_attributes),
            );
        }
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
        regions.insert("Base".to_owned());
        for target in self.project.targets.values() {
            for source in &target.sources {
                collect_known_regions(&self.project.base_path.join(&source.path), &mut regions);
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
            {
                attributes.insert("DevelopmentTeam".to_owned(), PbxValue::String(team));
            }
            if let Some(style) = build_settings
                .get("CODE_SIGN_STYLE")
                .and_then(json_string_value)
            {
                attributes.insert("ProvisioningStyle".to_owned(), PbxValue::String(style));
            }
            if matches!(
                target.target_type,
                ProductType::UnitTestBundle | ProductType::UiTestBundle
            ) {
                if let Some((test_target_id, test_target_name)) = self.test_target_reference(target)
                {
                    attributes.insert(
                        "TestTargetID".to_owned(),
                        PbxValue::reference(test_target_id, test_target_name),
                    );
                }
            }
            if !attributes.is_empty() {
                if let Some(target_refs) = self.target_refs.get(&target.name) {
                    all_attributes
                        .insert(target_refs.target_id.clone(), PbxValue::Dict(attributes));
                }
            }
        }
        all_attributes
    }

    fn test_target_reference(&self, target: &Target) -> Option<(String, String)> {
        target
            .dependencies
            .iter()
            .find_map(|dependency| {
                (dependency.dependency_type == DependencyType::Target).then(|| {
                    self.target_refs
                        .get(&dependency.reference)
                        .map(|refs| (refs.target_id.clone(), dependency.reference.clone()))
                })?
            })
            .or_else(|| {
                self.project.targets.values().find_map(|candidate| {
                    (candidate.target_type == ProductType::Application).then(|| {
                        self.target_refs
                            .get(&candidate.name)
                            .map(|refs| (refs.target_id.clone(), candidate.name.clone()))
                    })?
                })
            })
    }

    fn add_native_target(&mut self, target: &Target) -> String {
        let files = self.collect_target_files(target);
        let source_files = files
            .iter()
            .filter(|file| file.build_phase == Some("Sources"))
            .map(|file| {
                PbxValue::reference(
                    file.build_file_id.clone().unwrap(),
                    format!("{} in Sources", file.name),
                )
            })
            .collect::<Vec<_>>();
        let resource_files = files
            .iter()
            .filter(|file| file.build_phase == Some("Resources"))
            .map(|file| {
                PbxValue::reference(
                    file.build_file_id.clone().unwrap(),
                    format!("{} in Resources", file.name),
                )
            })
            .collect::<Vec<_>>();
        let header_files = files
            .iter()
            .filter(|file| file.build_phase == Some("Headers"))
            .map(|file| {
                PbxValue::reference(
                    file.build_file_id.clone().unwrap(),
                    format!("{} in Headers", file.name),
                )
            })
            .collect::<Vec<_>>();

        let sources_phase = self.add_build_phase(
            "PBXSourcesBuildPhase",
            &format!("{}:Sources", target.name),
            "Sources",
            source_files,
        );
        let resources_phase = self.add_build_phase(
            "PBXResourcesBuildPhase",
            &format!("{}:Resources", target.name),
            "Resources",
            resource_files,
        );
        let headers_phase = (!header_files.is_empty()).then(|| {
            self.add_build_phase(
                "PBXHeadersBuildPhase",
                &format!("{}:Headers", target.name),
                "Headers",
                header_files,
            )
        });
        let copy_headers = files
            .iter()
            .filter(|file| file.build_phase == Some("CopyHeaders"))
            .map(|file| {
                PbxValue::reference(
                    file.build_file_id.clone().unwrap(),
                    format!("{} in CopyHeaders", file.name),
                )
            })
            .collect::<Vec<_>>();
        let copy_headers_phase = (!copy_headers.is_empty()).then(|| {
            self.add_copy_files_build_phase(
                &format!("{}:CopyHeaders", target.name),
                "Copy Headers",
                16,
                "include/$(PRODUCT_NAME)",
                target.only_copy_files_on_install,
                copy_headers,
            )
        });
        let copy_files = files
            .iter()
            .filter(|file| file.build_phase == Some("CopyFiles"))
            .map(|file| {
                PbxValue::reference(
                    file.build_file_id.clone().unwrap(),
                    format!("{} in CopyFiles", file.name),
                )
            })
            .collect::<Vec<_>>();
        let copy_files_phase = (!copy_files.is_empty()).then(|| {
            self.add_copy_files_build_phase(
                &format!("{}:CopyFiles:Frameworks", target.name),
                "Embed Frameworks",
                10,
                "",
                target.only_copy_files_on_install,
                copy_files,
            )
        });
        let mut package_product_dependencies = Vec::new();
        let framework_build_files =
            self.framework_build_files(target, &mut package_product_dependencies);
        let frameworks_phase = self.add_build_phase(
            "PBXFrameworksBuildPhase",
            &format!("{}:Frameworks", target.name),
            "Frameworks",
            framework_build_files,
        );
        let target_dependency_copy_phase = self.target_dependency_copy_files_phase(target);
        let dependency_copy_phase = self.dependency_copy_files_phase(target);
        let bundle_copy_phase = self.bundle_copy_files_phase(target);
        let carthage_copy_phase = self.carthage_copy_frameworks_phase(target);
        let mut phases = Vec::new();
        phases.extend(target.pre_build_scripts.iter().map(|script| {
            PbxValue::reference(
                self.add_shell_script_build_phase(&target.name, script),
                script
                    .name
                    .clone()
                    .unwrap_or_else(|| "Run Script".to_owned()),
            )
        }));
        if let Some(copy_headers_phase) = copy_headers_phase {
            phases.push(PbxValue::reference(copy_headers_phase, "Copy Headers"));
        }
        if let Some(headers_phase) = headers_phase {
            phases.push(PbxValue::reference(headers_phase, "Headers"));
        }
        if target.target_type == ProductType::StaticLibrary {
            phases.push(PbxValue::reference(sources_phase, "Sources"));
            phases.extend(target.post_compile_scripts.iter().map(|script| {
                PbxValue::reference(
                    self.add_shell_script_build_phase(&target.name, script),
                    script
                        .name
                        .clone()
                        .unwrap_or_else(|| "Run Script".to_owned()),
                )
            }));
            if let Some(header_phase) = self.add_swift_objc_header_phase(target, &files) {
                phases.push(PbxValue::reference(
                    header_phase,
                    "Copy Swift Objective-C Interface Header",
                ));
            }
        } else if target.put_resources_before_sources_build_phase {
            phases.push(PbxValue::reference(resources_phase, "Resources"));
            phases.push(PbxValue::reference(sources_phase, "Sources"));
            phases.extend(target.post_compile_scripts.iter().map(|script| {
                PbxValue::reference(
                    self.add_shell_script_build_phase(&target.name, script),
                    script
                        .name
                        .clone()
                        .unwrap_or_else(|| "Run Script".to_owned()),
                )
            }));
            if let Some(carthage_copy_phase) = &carthage_copy_phase {
                phases.push(PbxValue::reference(carthage_copy_phase.clone(), "Carthage"));
            }
            phases.push(PbxValue::reference(frameworks_phase, "Frameworks"));
        } else {
            phases.push(PbxValue::reference(sources_phase, "Sources"));
            phases.extend(target.post_compile_scripts.iter().map(|script| {
                PbxValue::reference(
                    self.add_shell_script_build_phase(&target.name, script),
                    script
                        .name
                        .clone()
                        .unwrap_or_else(|| "Run Script".to_owned()),
                )
            }));
            phases.push(PbxValue::reference(resources_phase, "Resources"));
            if let Some(carthage_copy_phase) = &carthage_copy_phase {
                phases.push(PbxValue::reference(carthage_copy_phase.clone(), "Carthage"));
            }
            phases.push(PbxValue::reference(frameworks_phase, "Frameworks"));
        }
        if let Some((target_dependency_copy_phase, phase_name)) = target_dependency_copy_phase {
            phases.push(PbxValue::reference(
                target_dependency_copy_phase,
                phase_name,
            ));
        }
        if let Some((dependency_copy_phase, phase_name)) = dependency_copy_phase {
            phases.push(PbxValue::reference(dependency_copy_phase, phase_name));
        }
        if let Some(bundle_copy_phase) = bundle_copy_phase {
            phases.push(PbxValue::reference(
                bundle_copy_phase,
                "Copy Bundle Resources",
            ));
        }
        if let Some(copy_files_phase) = copy_files_phase {
            phases.push(PbxValue::reference(copy_files_phase, "Embed Frameworks"));
        }
        phases.extend(target.post_build_scripts.iter().map(|script| {
            PbxValue::reference(
                self.add_shell_script_build_phase(&target.name, script),
                script
                    .name
                    .clone()
                    .unwrap_or_else(|| "Run Script".to_owned()),
            )
        }));

        let dependency_refs = target
            .dependencies
            .iter()
            .filter_map(|dependency| {
                (dependency.dependency_type == DependencyType::Target).then(|| {
                    PbxValue::reference(
                        self.add_target_dependency(&target.name, &dependency.reference),
                        "PBXTargetDependency",
                    )
                })
            })
            .collect();

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
            .field("productName", PbxValue::String(target.product_name.clone()))
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
        self.graph
            .add(&format!("nativeTarget:{}", target.name), object)
    }

    fn add_aggregate_target(&mut self, aggregate: &AggregateTarget) -> String {
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
        let build_phases = aggregate
            .build_scripts
            .iter()
            .map(|script| {
                PbxValue::reference(
                    self.add_shell_script_build_phase(&aggregate.name, script),
                    script
                        .name
                        .clone()
                        .unwrap_or_else(|| "Run Script".to_owned()),
                )
            })
            .collect();
        let dependencies = aggregate
            .targets
            .iter()
            .map(|dependency| {
                PbxValue::reference(
                    self.add_target_dependency(&aggregate.name, dependency),
                    "PBXTargetDependency",
                )
            })
            .collect();

        self.graph.add(
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
        )
    }

    fn add_target_dependency(&mut self, target_name: &str, dependency_name: &str) -> String {
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
        self.graph.add(
            &format!("targetDependency:{target_name}:{dependency_name}"),
            PbxObject::new("PBXTargetDependency", "PBXTargetDependency")
                .field(
                    "target",
                    PbxValue::reference(dependency_target_id, dependency_name.to_owned()),
                )
                .field(
                    "targetProxy",
                    PbxValue::reference(proxy_id, "PBXContainerItemProxy"),
                ),
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
        let mut object = PbxObject::new(
            "PBXFileSystemSynchronizedRootGroup",
            display_name(&source.path),
        )
        .field("path", PbxValue::String(source.path.clone()))
        .field("sourceTree", PbxValue::String("<group>".to_owned()));
        if let Some(name) = &source.name {
            object = object.field("name", PbxValue::String(name.clone()));
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
        if !explicit_folders.is_empty() {
            object = object.field("explicitFolders", string_array(&explicit_folders));
        }
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
        let mut groups = Vec::<(String, PbxValue)>::new();
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
                let key = format!("{:?}:{}", source_type, source.path);
                if !seen.insert(key) {
                    continue;
                }
                let Some((id, comment)) = self.add_source_navigator_group(target, source) else {
                    continue;
                };
                let sort_path = source.group.clone().unwrap_or_else(|| source.path.clone());
                groups.push((sort_path, PbxValue::reference(id, comment)));
            }
        }
        groups.sort_by(|(left, _), (right, _)| natural_cmp(left, right));

        let mut top_level_seen = BTreeSet::new();
        groups
            .into_iter()
            .filter_map(|(_, value)| match value {
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
                Some((
                    id,
                    source
                        .name
                        .clone()
                        .unwrap_or_else(|| display_name(&source.path)),
                ))
            }
            SourceType::Folder => {
                let comment = source
                    .name
                    .clone()
                    .unwrap_or_else(|| display_name(&source.path));
                let id = self.add_file_reference(
                    &format!("navigatorFolder:{}", source.path),
                    comment.clone(),
                    Some(source.path.clone()),
                    Some("folder".to_owned()),
                    source.name.clone(),
                    "<group>",
                    false,
                );
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
                let parent = path
                    .parent()
                    .unwrap_or_else(|| self.project.base_path.as_path());
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
                    let id =
                        self.add_source_file_reference(file_parent, &path, source.name.as_deref());
                    return Some(self.add_nested_navigator_groups(
                        &format!("navigatorCustomGroup:{group}"),
                        group,
                        PbxValue::reference(id, name),
                    ));
                }
                if parent_relative.is_empty() {
                    let id = self.add_source_file_reference(parent, &path, source.name.as_deref());
                    Some((id, name))
                } else if self.should_create_intermediate_groups(source) {
                    let child =
                        self.add_source_file_reference(parent, &path, source.name.as_deref());
                    Some(self.add_nested_navigator_groups(
                        &format!("navigatorIntermediate:{parent_relative}"),
                        &parent_relative,
                        PbxValue::reference(child, name),
                    ))
                } else {
                    let child =
                        self.add_source_file_reference(parent, &path, source.name.as_deref());
                    let group_name = display_name(&parent_relative);
                    let id = self.add_group(
                        &format!("navigatorGroup:{parent_relative}"),
                        Some(group_name.clone()),
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
                let group_parent = if self.should_create_intermediate_groups(source) {
                    path.parent()
                        .unwrap_or_else(|| self.project.base_path.as_path())
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
            let group_key = format!("{key_prefix}:{}", parts[..=index].join("/"));
            let name = parts[index].to_owned();
            let id = self.add_group(&group_key, None, Some(name.clone()), vec![child]);
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
                !name.starts_with('.')
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
        for entry in entries {
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
            ) {
                let file_id = self.add_source_file_reference(directory, &entry, None);
                children.push(PbxValue::reference(
                    file_id,
                    display_name(&entry.to_string_lossy()),
                ));
            }
        }

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
        Some(self.add_group(
            &format!("navigatorGroup:{relative}"),
            (root && source.name.is_some()).then_some(name.clone()),
            Some(group_path),
            children,
        ))
    }

    fn add_source_file_reference(
        &mut self,
        parent: &Path,
        path: &Path,
        name: Option<&str>,
    ) -> String {
        let relative = pathdiff(path, &self.project.base_path)
            .to_string_lossy()
            .into_owned();
        let parent_relative = pathdiff(parent, &self.project.base_path);
        let file_path = pathdiff(path, parent).to_string_lossy().into_owned();
        let comment = name
            .map(str::to_owned)
            .unwrap_or_else(|| display_name(&relative));
        self.add_file_reference(
            &format!("navigatorFileRef:{relative}"),
            comment,
            Some(file_path),
            Some(file_type_for_path(&relative, None)),
            name.map(str::to_owned)
                .filter(|value| *value != display_name(&relative)),
            if parent_relative.as_os_str().is_empty() {
                "<group>"
            } else {
                "<group>"
            },
            false,
        )
    }

    fn add_frameworks_navigator_group(&mut self) -> Option<PbxValue> {
        let mut references = BTreeMap::<String, PbxValue>::new();
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
                        (name, dependency.reference.clone(), "SDKROOT")
                    }
                    DependencyType::Carthage { .. } => {
                        let name = carthage_framework_name(&dependency.reference);
                        (name.clone(), name, "<group>")
                    }
                    _ => continue,
                };
                let id = self.add_file_reference(
                    &format!("navigatorFrameworkRef:{path}"),
                    name.clone(),
                    Some(path),
                    Some(file_type_for_path(&name, None)),
                    None,
                    source_tree,
                    false,
                );
                references.insert(name.clone(), PbxValue::reference(id, name));
            }
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
            let custom_name_applies =
                source.name.is_some() && (!source_path.is_dir() || expanded_paths.len() == 1);
            for path in expanded_paths {
                let relative = pathdiff(&path, &self.project.base_path);
                let relative_string = relative.to_string_lossy().into_owned();
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
                let file_type = if effective_source_type == SourceType::Folder {
                    "folder".to_owned()
                } else {
                    file_type_for_path(&relative_string, None)
                };
                let file_ref_id = self.add_file_reference(
                    &format!("fileRef:{}:{relative_string}", target.name),
                    name.clone(),
                    Some(relative_string.clone()),
                    Some(file_type),
                    custom_name_applies.then(|| name.clone()),
                    "<group>",
                    false,
                );
                let mut build_phase =
                    if let Some(override_phase) = source_build_phase_override(source) {
                        override_phase
                    } else if effective_source_type == SourceType::Folder {
                        Some("Resources")
                    } else if info_plist_files.contains(&relative_string) {
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
                } else if build_phase == Some("Headers") && !target_supports_headers_phase(target) {
                    build_phase = None;
                }
                let build_file_id = if let Some(phase) = build_phase {
                    let mut object = PbxObject::new("PBXBuildFile", format!("{name} in {phase}"))
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
    ) -> Vec<PbxValue> {
        let mut files = Vec::new();
        for dependency in &target.dependencies {
            match &dependency.dependency_type {
                DependencyType::Framework | DependencyType::Sdk { .. } => {
                    let reference = &dependency.reference;
                    let name = display_name(reference);
                    let file_ref = self.add_file_reference(
                        &format!("frameworkRef:{}:{reference}", target.name),
                        name.clone(),
                        Some(reference.clone()),
                        Some(file_type_for_path(&name, None)),
                        None,
                        if matches!(dependency.dependency_type, DependencyType::Sdk { .. }) {
                            "SDKROOT"
                        } else {
                            "<group>"
                        },
                        false,
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
                    for product_name in product_names {
                        let product_dependency = self
                            .add_package_product_dependency(&dependency.reference, &product_name);
                        package_product_dependencies.push(PbxValue::reference(
                            product_dependency.clone(),
                            product_name.clone(),
                        ));
                        files.push(self.package_product_build_file(
                            &target.name,
                            &product_name,
                            &product_dependency,
                            dependency.weak_link,
                            &platform_filters_for_dependency(dependency),
                        ));
                    }
                }
                DependencyType::Carthage { .. } => {
                    if dependency.link == Some(false) {
                        continue;
                    }
                    let name = carthage_framework_name(&dependency.reference);
                    let file_ref = self.add_file_reference(
                        &format!("carthageRef:{}:{}", target.name, dependency.reference),
                        name.clone(),
                        Some(name.clone()),
                        Some(file_type_for_path(&name, None)),
                        None,
                        "<group>",
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
                        &format!("carthageBuildFile:{}:{}", target.name, dependency.reference),
                        object,
                    );
                    files.push(PbxValue::reference(
                        build_file,
                        format!("{name} in Frameworks"),
                    ));
                }
                DependencyType::Bundle => {}
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
                    &format!("bundleRef:{}:{reference}", target.name),
                    name.clone(),
                    Some(reference.clone()),
                    Some(file_type_for_path(reference, None)),
                    None,
                    "<group>",
                    false,
                );
                let mut object = PbxObject::new("PBXBuildFile", format!("{name} in CopyFiles"))
                    .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                let platform_filters = platform_filters_for_dependency(dependency);
                if !platform_filters.is_empty() {
                    object = object.field("platformFilters", string_array(&platform_filters));
                }
                let build_file = self.graph.add(
                    &format!("bundleBuildFile:{}:{reference}", target.name),
                    object,
                );
                PbxValue::reference(build_file, format!("{name} in CopyFiles"))
            })
            .collect::<Vec<_>>();

        (!files.is_empty()).then(|| {
            self.add_copy_files_build_phase(
                &format!("{}:CopyBundles", target.name),
                "Copy Bundle Resources",
                7,
                "",
                target.only_copy_files_on_install,
                files,
            )
        })
    }

    fn target_dependency_copy_files_phase(&mut self, target: &Target) -> Option<(String, String)> {
        let mut phase_settings = None::<CopyFilesSettings>;
        let mut files = Vec::new();

        for dependency in &target.dependencies {
            if dependency.dependency_type != DependencyType::Target {
                continue;
            }
            let Some(dependency_target) = self.project.targets.get(&dependency.reference) else {
                continue;
            };
            if !should_embed_target_dependency(dependency, dependency_target) {
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
            phase_settings.get_or_insert_with(|| settings.clone());

            let mut object = PbxObject::new("PBXBuildFile", format!("{filename} in CopyFiles"))
                .field(
                    "fileRef",
                    PbxValue::reference(product_ref_id, filename.clone()),
                );
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
            files.push(PbxValue::reference(
                build_file,
                format!("{filename} in CopyFiles"),
            ));
        }

        if files.is_empty() {
            return None;
        }
        let settings = phase_settings.expect("copy files should have settings when files exist");
        let phase_id = self.add_copy_files_build_phase(
            &format!("{}:CopyTargetDependencies", target.name),
            &settings.phase_name,
            settings.dst_subfolder_spec,
            &settings.dst_path,
            target.only_copy_files_on_install,
            files,
        );
        Some((phase_id, settings.phase_name))
    }

    fn dependency_copy_files_phase(&mut self, target: &Target) -> Option<(String, String)> {
        let mut phase_settings = None::<CopyFilesSettings>;
        let mut files = Vec::new();

        for dependency in &target.dependencies {
            if !should_embed_external_dependency(dependency) {
                continue;
            }
            let Some(settings) = copy_files_settings_for_embedded_dependency(dependency) else {
                continue;
            };
            phase_settings.get_or_insert_with(|| settings.clone());
            let platform_filters = platform_filters_for_dependency(dependency);

            match &dependency.dependency_type {
                DependencyType::Framework | DependencyType::Sdk { .. } => {
                    let reference = &dependency.reference;
                    let name = display_name(reference);
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
                    let mut object = PbxObject::new("PBXBuildFile", format!("{name} in CopyFiles"))
                        .field("fileRef", PbxValue::reference(file_ref, name.clone()));
                    if !platform_filters.is_empty() {
                        object = object.field("platformFilters", string_array(&platform_filters));
                    }
                    let build_file = self.graph.add(
                        &format!("dependencyCopyBuildFile:{}:{reference}", target.name),
                        object,
                    );
                    files.push(PbxValue::reference(
                        build_file,
                        format!("{name} in CopyFiles"),
                    ));
                }
                DependencyType::Carthage { .. } => {}
                DependencyType::Package { products } => {
                    let product_names = if products.is_empty() {
                        vec![dependency.reference.clone()]
                    } else {
                        products.clone()
                    };
                    for product_name in product_names {
                        let product_dependency = self
                            .add_package_product_dependency(&dependency.reference, &product_name);
                        let mut object =
                            PbxObject::new("PBXBuildFile", format!("{product_name} in CopyFiles"))
                                .field(
                                    "productRef",
                                    PbxValue::reference(product_dependency, product_name.clone()),
                                );
                        if !platform_filters.is_empty() {
                            object =
                                object.field("platformFilters", string_array(&platform_filters));
                        }
                        let build_file = self.graph.add(
                            &format!("packageProductCopyBuildFile:{}:{product_name}", target.name),
                            object,
                        );
                        files.push(PbxValue::reference(
                            build_file,
                            format!("{product_name} in CopyFiles"),
                        ));
                    }
                }
                DependencyType::Target | DependencyType::Bundle => {}
            }
        }

        if files.is_empty() {
            return None;
        }
        let settings = phase_settings.expect("copy files should have settings when files exist");
        let phase_id = self.add_copy_files_build_phase(
            &format!("{}:CopyDependencies", target.name),
            &settings.phase_name,
            settings.dst_subfolder_spec,
            &settings.dst_path,
            target.only_copy_files_on_install,
            files,
        );
        Some((phase_id, settings.phase_name))
    }

    fn carthage_copy_frameworks_phase(&mut self, target: &Target) -> Option<String> {
        let platform_dir = carthage_platform_dir(&target.platform);
        let base_path = self
            .project
            .spec_options
            .carthage_build_path
            .as_deref()
            .unwrap_or("Carthage/Build");
        let mut input_paths = Vec::new();
        let mut output_paths = Vec::new();

        for dependency in &target.dependencies {
            let DependencyType::Carthage { link_type, .. } = &dependency.dependency_type else {
                continue;
            };
            if *link_type != crate::spec::CarthageLinkType::Dynamic
                || !should_embed_external_dependency(dependency)
            {
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
        for (name, package) in &self.project.packages {
            if let Some(path) = package.get("path").and_then(|value| value.as_str()) {
                if package
                    .get("excludeFromProject")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    continue;
                }
                let package_ref = self.graph.add(
                    &format!("xcLocalPackage:{name}:{path}"),
                    PbxObject::new(
                        "XCLocalSwiftPackageReference",
                        format!("XCLocalSwiftPackageReference \"{path}\""),
                    )
                    .field("relativePath", PbxValue::String(path.to_owned())),
                );
                self.package_refs.insert(name.clone(), package_ref.clone());
                self.project_package_refs.push(PbxValue::reference(
                    package_ref,
                    format!("XCLocalSwiftPackageReference \"{path}\""),
                ));

                let file_ref = self.add_file_reference(
                    &format!("localPackageFile:{name}:{path}"),
                    name.clone(),
                    Some(path.to_owned()),
                    Some("folder".to_owned()),
                    Some(name.clone()),
                    "SOURCE_ROOT",
                    true,
                );
                let package_group = package
                    .get("group")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| self.project.spec_options.local_packages_group.clone())
                    .unwrap_or_else(|| "Packages".to_owned());
                if package_group.is_empty() {
                    main_group_children.push(PbxValue::reference(file_ref, name.clone()));
                } else {
                    grouped_package_children
                        .entry(package_group)
                        .or_default()
                        .push(PbxValue::reference(file_ref, name.clone()));
                }
            } else if let Some(url) = package_url(package) {
                let package_ref = self.graph.add(
                    &format!("xcRemotePackage:{name}:{url}"),
                    PbxObject::new(
                        "XCRemoteSwiftPackageReference",
                        format!("XCRemoteSwiftPackageReference \"{name}\""),
                    )
                    .field("repositoryURL", PbxValue::String(url))
                    .field("requirement", PbxValue::Dict(package_requirement(package))),
                );
                self.package_refs.insert(name.clone(), package_ref.clone());
                self.project_package_refs.push(PbxValue::reference(
                    package_ref,
                    format!("XCRemoteSwiftPackageReference \"{name}\""),
                ));
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
            let key = format!("localPackageGroup:{}", parts[..=index].join("/"));
            current_id = self.add_group(&key, Some(name.clone()), None, children);
            children = vec![PbxValue::reference(current_id.clone(), name)];
        }
        (
            current_id,
            parts.first().copied().unwrap_or(group_path).to_owned(),
        )
    }

    fn add_package_product_dependency(&mut self, package_name: &str, product_name: &str) -> String {
        let mut object = PbxObject::new("XCSwiftPackageProductDependency", product_name.to_owned())
            .field("productName", PbxValue::String(product_name.to_owned()));
        if let Some(package_ref) = self.package_refs.get(package_name) {
            let comment = self
                .graph
                .comments
                .get(package_ref)
                .cloned()
                .unwrap_or_else(|| package_name.to_owned());
            object = object.field("package", PbxValue::reference(package_ref.clone(), comment));
        }
        self.graph.add(
            &format!("packageProduct:{package_name}:{product_name}"),
            object,
        )
    }

    fn package_product_build_file(
        &mut self,
        target_name: &str,
        product_name: &str,
        product_dependency: &str,
        weak_link: bool,
        platform_filters: &[String],
    ) -> PbxValue {
        let mut object = PbxObject::new("PBXBuildFile", format!("{product_name} in Frameworks"))
            .field(
                "productRef",
                PbxValue::reference(product_dependency.to_owned(), product_name.to_owned()),
            );
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
        self.add_file_reference(
            &format!("productRef:{}", target.name),
            target.filename(),
            Some(target.filename()),
            Some(file_type_for_path(
                &target.filename(),
                Some(&target.target_type),
            )),
            None,
            "BUILT_PRODUCTS_DIR",
            true,
        )
    }

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
        let mut object =
            PbxObject::new("PBXGroup", comment).field("children", PbxValue::Array(children));
        if let Some(name) = name {
            object = object.field("name", PbxValue::String(name));
        }
        if let Some(path) = path {
            object = object.field("path", PbxValue::String(path));
        }
        object = object.field("sourceTree", PbxValue::String("<group>".to_owned()));
        self.graph
            .add_or_merge_group(&format!("group:{key}"), object)
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

    fn add_copy_files_build_phase(
        &mut self,
        key: &str,
        name: &str,
        dst_subfolder_spec: i64,
        dst_path: &str,
        run_only_for_deployment_postprocessing: bool,
        files: Vec<PbxValue>,
    ) -> String {
        self.graph.add(
            &format!("buildPhase:{key}"),
            PbxObject::new("PBXCopyFilesBuildPhase", name.to_owned())
                .field("buildActionMask", PbxValue::Int(2147483647))
                .field("dstPath", PbxValue::String(dst_path.to_owned()))
                .field("dstSubfolderSpec", PbxValue::Int(dst_subfolder_spec))
                .field("files", PbxValue::Array(files))
                .field("name", PbxValue::String(name.to_owned()))
                .field(
                    "runOnlyForDeploymentPostprocessing",
                    PbxValue::Int(if run_only_for_deployment_postprocessing {
                        1
                    } else {
                        0
                    }),
                ),
        )
    }

    fn add_shell_script_build_phase(&mut self, target_name: &str, script: &BuildScript) -> String {
        let name = script
            .name
            .clone()
            .unwrap_or_else(|| "Run Script".to_owned());
        let script_text = match &script.script {
            BuildScriptKind::Script(script) => script.clone(),
            BuildScriptKind::Path(path) => {
                fs::read_to_string(self.project.base_path.join(path)).unwrap_or_default()
            }
        };
        let mut object = PbxObject::new("PBXShellScriptBuildPhase", name.clone())
            .field("buildActionMask", PbxValue::Int(2147483647))
            .field("files", PbxValue::Array(Vec::new()))
            .field("inputFileListPaths", string_array(&script.input_file_lists))
            .field("inputPaths", string_array(&script.input_files))
            .field(
                "outputFileListPaths",
                string_array(&script.output_file_lists),
            )
            .field("outputPaths", string_array(&script.output_files))
            .field(
                "runOnlyForDeploymentPostprocessing",
                PbxValue::Int(bool_int(script.run_only_when_installing)),
            )
            .field(
                "shellPath",
                PbxValue::String(script.shell.clone().unwrap_or_else(|| "/bin/sh".to_owned())),
            )
            .field("shellScript", PbxValue::String(script_text))
            .field(
                "showEnvVarsInLog",
                PbxValue::Int(bool_int(script.show_env_vars)),
            );
        if !script.based_on_dependency_analysis {
            object = object.field("alwaysOutOfDate", PbxValue::Int(1));
        }
        if let Some(dependency_file) = &script.discovered_dependency_file {
            object = object.field("dependencyFile", PbxValue::String(dependency_file.clone()));
        }
        self.graph.add(
            &format!("shellScript:{target_name}:{}", build_script_key(script)),
            object,
        )
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
                .field("inputFileListPaths", PbxValue::Array(Vec::new()))
                .field("inputPaths", string_array(&input_paths))
                .field("outputFileListPaths", PbxValue::Array(Vec::new()))
                .field("outputPaths", string_array(&output_paths))
                .field("runOnlyForDeploymentPostprocessing", PbxValue::Int(0))
                .field("shellPath", PbxValue::String("/bin/sh".to_owned()))
                .field(
                    "shellScript",
                    PbxValue::String(
                        "ditto \"${SCRIPT_INPUT_FILE_0}\" \"${SCRIPT_OUTPUT_FILE_0}\"\n".to_owned(),
                    ),
                )
                .field("showEnvVarsInLog", PbxValue::Int(1)),
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
        let configs = if self.project.configs.is_empty() {
            vec!["Debug".to_owned(), "Release".to_owned()]
        } else {
            self.project.configs.keys().cloned().collect()
        };
        for config_name in configs {
            let mut build_settings = BTreeMap::new();
            build_settings.insert(
                "PRODUCT_NAME".to_owned(),
                PbxValue::String("$(TARGET_NAME)".to_owned()),
            );
            if let Some(extra_settings) = build_settings_by_config.get(&config_name) {
                build_settings.extend(extra_settings.clone());
            }
            let mut object = PbxObject::new("XCBuildConfiguration", config_name.clone())
                .field("buildSettings", PbxValue::Dict(build_settings))
                .field("name", PbxValue::String(config_name.clone()));
            if let Some(Some(path)) = config_files.get(&config_name) {
                let config_ref = self.add_file_reference(
                    &format!("configFile:{owner_type}:{owner_name}:{config_name}:{path}"),
                    display_name(path),
                    Some(path.clone()),
                    Some("text.xcconfig".to_owned()),
                    None,
                    "<group>",
                    true,
                );
                object = object.field(
                    "baseConfigurationReference",
                    PbxValue::reference(config_ref, display_name(path)),
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
                let mut build_settings = self.project_default_build_settings();
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
            let settings = settings_by_config.entry(config).or_default();
            for (key, value) in self.target_default_build_settings(target) {
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
            if self.target_dependencies_require_objc_linking(target)
                && !settings.contains_key("OTHER_LDFLAGS")
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
            None => target_sources_require_objc_linking(&self.project.base_path, target),
        }
    }

    fn default_configuration_name(&self) -> String {
        self.project
            .spec_options
            .default_config
            .clone()
            .unwrap_or_else(|| "Release".to_owned())
    }

    fn development_language(&self) -> &str {
        self.project
            .spec_options
            .development_language
            .as_deref()
            .unwrap_or("en")
    }

    fn project_default_build_settings(&self) -> BTreeMap<String, PbxValue> {
        let mut settings = BTreeMap::new();
        if let Some(platform) = self.single_project_platform() {
            settings.insert(
                "SDKROOT".to_owned(),
                PbxValue::String(platform.sdk_root().to_owned()),
            );
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

    fn target_default_build_settings(&self, target: &Target) -> BTreeMap<String, PbxValue> {
        let mut settings = BTreeMap::new();
        if let Some(prefix) = &self.project.spec_options.bundle_id_prefix {
            settings.insert(
                "PRODUCT_BUNDLE_IDENTIFIER".to_owned(),
                PbxValue::String(format!("{prefix}.{}", target.name)),
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
        }
        settings
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
            self.project.configs.keys().cloned().collect()
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

    fn serialize(&self, root_id: &str) -> String {
        let mut output = String::new();
        output.push_str("// !$*UTF8*$!\n{\n");
        output.push_str(
            "\tarchiveVersion = 1;\n\tclasses = {\n\t};\n\tobjectVersion = 77;\n\tobjects = {\n\n",
        );

        let mut sections: BTreeMap<&str, Vec<(&String, &PbxObject)>> = BTreeMap::new();
        for (id, object) in &self.graph.objects {
            sections.entry(object.isa).or_default().push((id, object));
        }

        for (isa, objects) in sections {
            let _ = writeln!(output, "/* Begin {isa} section */");
            for (id, object) in objects {
                let _ = write!(output, "\t\t{id}");
                if let Some(comment) = &object.comment {
                    if !comment.is_empty() {
                        let _ = write!(output, " /* {comment} */");
                    }
                }
                output.push_str(" = {\n");
                let _ = writeln!(output, "\t\t\tisa = {isa};");
                for (key, value) in &object.fields {
                    let _ = write!(output, "\t\t\t{key} = ");
                    value.write(&mut output, 3, &self.graph.comments);
                    output.push_str(";\n");
                }
                output.push_str("\t\t};\n");
            }
            let _ = writeln!(output, "/* End {isa} section */\n");
        }

        output.push_str("\t};\n");
        let _ = writeln!(
            output,
            "\trootObject = {} /* Project object */;\n}}",
            root_id
        );
        output
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

fn write_schemes(project: &Project, project_path: &Path) -> Result<(), ProjectWriteError> {
    let schemes_dir = project_path.join("xcshareddata/xcschemes");
    let mut schemes = Vec::new();
    for scheme in project.scheme_specs.values() {
        if scheme.management.shared {
            schemes.push((scheme.name.clone(), scheme_xml(project, scheme)));
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
                target_scheme_xml(project, target, target_scheme, variant),
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
    let schemes_dir = project_path.join("xcuserdata/oxidegen.xcuserdatad/xcschemes");
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

fn scheme_xml(project: &Project, scheme: &crate::spec::Scheme) -> String {
    let debug_config = default_config_for(project, "debug");
    let release_config = default_config_for(project, "release");
    let runnable = first_runnable_scheme_target(project, &scheme.build.targets).or_else(|| {
        scheme
            .build
            .targets
            .first()
            .map(|target| target.target.as_str())
    });
    let run_macro_expansion = scheme
        .run
        .as_ref()
        .and_then(|run| run.macro_expansion.as_deref());
    let testing_macro_expansion =
        first_testing_runnable_scheme_target(project, &scheme.build.targets);
    let test_macro_expansion = scheme
        .test
        .as_ref()
        .and_then(|test| test.macro_expansion.as_deref())
        .or(testing_macro_expansion)
        .or(run_macro_expansion);
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        output,
        "<Scheme LastUpgradeVersion=\"{}\" version=\"1.7\">",
        xml_escape(&scheme_last_upgrade_version(project))
    );
    write_build_action(&mut output, project, &scheme.build, 1);
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
            .and_then(|test| test.custom_lldb_init.as_deref()),
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
        1,
    );
    let empty_command_line_arguments = indexmap::IndexMap::new();
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
            .map(|run| &run.command_line_arguments)
            .unwrap_or(&empty_command_line_arguments),
        scheme.run.as_ref().and_then(|run| run.language.as_deref()),
        scheme.run.as_ref().and_then(|run| run.region.as_deref()),
        scheme
            .run
            .as_ref()
            .map(|run| run.environment_variables.as_slice())
            .unwrap_or(&[]),
        &debug_config,
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
        &release_config,
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
    let test_targets = scheme
        .test_targets
        .iter()
        .map(|name| crate::spec::SchemeTestTarget {
            target_reference: name.clone(),
            random_execution_order: false,
            parallelizable: false,
            location: None,
            skipped: false,
            skipped_tests: Vec::new(),
            selected_tests: Vec::new(),
        })
        .collect::<Vec<_>>();
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        output,
        "<Scheme LastUpgradeVersion=\"{}\" version=\"1.7\">",
        xml_escape(&scheme_last_upgrade_version(project))
    );
    write_build_action(&mut output, project, &build, 1);
    write_test_action(
        &mut output,
        project,
        Some(&debug_config),
        scheme.gather_coverage_data,
        true,
        scheme.disable_main_thread_checker,
        None,
        None,
        &test_targets,
        &scheme.coverage_targets,
        &scheme.test_plans,
        &scheme.environment_variables,
        None,
        None,
        None,
        &debug_config,
        1,
    );
    write_launch_action(
        &mut output,
        project,
        Some(&target.name),
        Some(&debug_config),
        true,
        None,
        false,
        None,
        None,
        None,
        scheme.store_kit_configuration.as_deref(),
        None,
        None,
        scheme.disable_main_thread_checker,
        &scheme.command_line_arguments,
        scheme.language.as_deref(),
        scheme.region.as_deref(),
        &scheme.environment_variables,
        &debug_config,
        1,
    );
    write_profile_action(
        &mut output,
        project,
        Some(&target.name),
        Some(&release_config),
        &scheme.environment_variables,
        false,
        &release_config,
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
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<BuildAction parallelizeBuildables=\"{}\" buildImplicitDependencies=\"{}\" runPostActionsOnFailure=\"{}\">",
        bool_xml(build.parallelize_build),
        bool_xml(build.build_implicit_dependencies),
        bool_xml(build.run_post_actions_on_failure)
    );
    for action in &build.pre_actions {
        write_execution_action(output, project, "PreActions", action, indent + 1);
    }
    write_indent(output, indent + 1);
    output.push_str("<BuildActionEntries>\n");
    for target in &build.targets {
        if let Some(project_target) = project.targets.get(&target.target) {
            write_indent(output, indent + 2);
            output.push_str("<BuildActionEntry");
            write_build_for_attributes(output, &target.build_types);
            output.push_str(">\n");
            write_buildable_reference(output, project, project_target, indent + 3);
            write_indent(output, indent + 2);
            output.push_str("</BuildActionEntry>\n");
        } else if let Some((target_name, container)) =
            project_reference_target(project, &target.target)
        {
            write_indent(output, indent + 2);
            output.push_str("<BuildActionEntry");
            write_build_for_attributes(output, &target.build_types);
            output.push_str(">\n");
            write_external_buildable_reference(output, target_name, container, indent + 3);
            write_indent(output, indent + 2);
            output.push_str("</BuildActionEntry>\n");
        }
    }
    write_indent(output, indent + 1);
    output.push_str("</BuildActionEntries>\n");
    for action in &build.post_actions {
        write_execution_action(output, project, "PostActions", action, indent + 1);
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
    custom_lldb_init: Option<&str>,
    macro_expansion: Option<&str>,
    test_targets: &[crate::spec::SchemeTestTarget],
    coverage_targets: &[String],
    test_plans: &[crate::spec::TestPlan],
    environment_variables: &[crate::spec::EnvironmentVariable],
    capture_screenshots_automatically: Option<bool>,
    delete_screenshots_when_each_test_succeeds: Option<bool>,
    preferred_screen_capture_format: Option<&str>,
    default_config: &str,
    indent: usize,
) {
    write_indent(output, indent);
    let system_attachment_lifetime = match (
        capture_screenshots_automatically,
        delete_screenshots_when_each_test_succeeds,
    ) {
        (Some(false), _) => Some("keepNever"),
        (Some(true), Some(false)) => Some("keepAlways"),
        _ => None,
    };
    let _ = writeln!(
        output,
        "<TestAction buildConfiguration=\"{}\" selectedDebuggerIdentifier=\"{}\" selectedLauncherIdentifier=\"{}\" shouldUseLaunchSchemeArgsEnv=\"YES\" codeCoverageEnabled=\"{}\" disableMainThreadChecker=\"{}\"{}{}{}>",
        xml_escape(config.unwrap_or(default_config)),
        if debug_enabled { "Xcode.DebuggerFoundation.Debugger.LLDB" } else { "" },
        if debug_enabled { "Xcode.DebuggerFoundation.Launcher.LLDB" } else { "Xcode.IDEFoundation.Launcher.PosixSpawn" },
        bool_xml(gather_coverage_data),
        bool_xml(disable_main_thread_checker),
        system_attachment_lifetime
            .map(|value| format!(" systemAttachmentLifetime=\"{value}\""))
            .unwrap_or_default(),
        preferred_screen_capture_format
            .map(|value| format!(" preferredScreenCaptureFormat=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        custom_lldb_init
            .map(|value| format!(" customLLDBInitFile=\"{}\"", xml_escape(value)))
            .unwrap_or_default()
    );
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
            write_indent(output, indent + 2);
            let _ = writeln!(
                output,
                "<TestableReference skipped=\"{}\" parallelizable=\"{}\" testExecutionOrdering=\"{}\">",
                bool_xml(test_target.skipped),
                bool_xml(test_target.parallelizable),
                if test_target.random_execution_order { "random" } else { "" }
            );
            if let Some(target) = project.targets.get(target_name) {
                write_buildable_reference(output, project, target, indent + 3);
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
    if !coverage_targets.is_empty() {
        write_indent(output, indent + 1);
        output.push_str("<CodeCoverageTargets>\n");
        for target_name in coverage_targets {
            if let Some(target) = project.targets.get(target_name) {
                write_buildable_reference(output, project, target, indent + 2);
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
    write_macro_expansion(output, project, macro_expansion, indent + 1);
    write_environment_variables(output, environment_variables, indent + 1);
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
    command_line_arguments: &indexmap::IndexMap<String, bool>,
    language: Option<&str>,
    region: Option<&str>,
    environment_variables: &[crate::spec::EnvironmentVariable],
    default_config: &str,
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<LaunchAction buildConfiguration=\"{}\" selectedDebuggerIdentifier=\"{}\" selectedLauncherIdentifier=\"{}\" launchStyle=\"0\" useCustomWorkingDirectory=\"{}\"{}{}{}{}{}{} ignoresPersistentStateOnLaunch=\"NO\" debugDocumentVersioning=\"YES\" debugServiceExtension=\"internal\" allowLocationSimulation=\"YES\"{} disableMainThreadChecker=\"{}\">",
        xml_escape(config.unwrap_or(default_config)),
        if debug_enabled { "Xcode.DebuggerFoundation.Debugger.LLDB" } else { "" },
        if debug_enabled { "Xcode.DebuggerFoundation.Launcher.LLDB" } else { "Xcode.IDEFoundation.Launcher.PosixSpawn" },
        bool_xml(custom_working_directory.is_some()),
        custom_working_directory
            .map(|value| format!(" customWorkingDirectory=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        if ask_for_app_to_launch { " askForAppToLaunch=\"YES\"" } else { "" },
        custom_lldb_init
            .map(|value| format!(" customLLDBInitFile=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        enable_gpu_frame_capture_mode
            .map(|value| format!(" enableGPUFrameCaptureMode=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        language
            .map(|value| format!(" language=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        region
            .map(|value| format!(" region=\"{}\"", xml_escape(value)))
            .unwrap_or_default(),
        launch_automatically_substyle.map(|value| format!(" launchAutomaticallySubstyle=\"{}\"", xml_escape(value))).unwrap_or_default(),
        bool_xml(disable_main_thread_checker)
    );
    if let Some(target_name) = runnable.and_then(|name| project.targets.get(name)) {
        write_indent(output, indent + 1);
        if is_watch_app_product(&target_name.target_type) {
            output.push_str("<RemoteRunnable runnableDebuggingMode=\"2\">\n");
        } else {
            output.push_str("<BuildableProductRunnable runnableDebuggingMode=\"0\">\n");
        }
        write_buildable_reference(output, project, target_name, indent + 2);
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
    write_macro_expansion(output, project, macro_expansion, indent + 1);
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
    write_command_line_arguments(output, command_line_arguments, indent + 1);
    write_environment_variables(output, environment_variables, indent + 1);
    write_indent(output, indent);
    output.push_str("</LaunchAction>\n");
}

fn write_profile_action(
    output: &mut String,
    project: &Project,
    runnable: Option<&str>,
    config: Option<&str>,
    environment_variables: &[crate::spec::EnvironmentVariable],
    ask_for_app_to_launch: bool,
    default_config: &str,
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<ProfileAction buildConfiguration=\"{}\" shouldUseLaunchSchemeArgsEnv=\"YES\" savedToolIdentifier=\"\" useCustomWorkingDirectory=\"NO\" debugDocumentVersioning=\"YES\"{}>",
        xml_escape(config.unwrap_or(default_config)),
        if ask_for_app_to_launch { " askForAppToLaunch=\"YES\"" } else { "" }
    );
    if let Some(target) = runnable.and_then(|name| project.targets.get(name)) {
        write_indent(output, indent + 1);
        output.push_str("<BuildableProductRunnable runnableDebuggingMode=\"0\">\n");
        write_buildable_reference(output, project, target, indent + 2);
        write_indent(output, indent + 1);
        output.push_str("</BuildableProductRunnable>\n");
    }
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
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<{element} {attribute}=\"{}\"/>",
        xml_escape(config)
    );
}

fn write_execution_action(
    output: &mut String,
    project: &Project,
    container: &str,
    action: &crate::spec::SchemeAction,
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(output, "<{container}>");
    write_indent(output, indent + 1);
    let _ = writeln!(
        output,
        "<ExecutionAction ActionType=\"Xcode.IDEStandardExecutionActionsCore.ExecutionActionType.ShellScriptAction\">"
    );
    write_indent(output, indent + 2);
    let _ = writeln!(
        output,
        "<ActionContent title=\"{}\" scriptText=\"{}\">",
        xml_escape(&action.name),
        xml_escape(&action.script)
    );
    if let Some(target) = action
        .settings_target
        .as_ref()
        .and_then(|target| project.targets.get(target))
    {
        write_buildable_reference(output, project, target, indent + 3);
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
    indent: usize,
) {
    write_indent(output, indent);
    let _ = writeln!(
        output,
        "<BuildableReference BuildableIdentifier=\"primary\" BlueprintIdentifier=\"{}\" BuildableName=\"{}\" BlueprintName=\"{}\" ReferencedContainer=\"container:{}.xcodeproj\"/>",
        object_id(&format!("nativeTarget:{}", target.name), 0),
        xml_escape(&target.filename()),
        xml_escape(&target.name),
        xml_escape(&project.name)
    );
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
    indent: usize,
) {
    let Some(target) = target_name.and_then(|name| project.targets.get(name)) else {
        return;
    };
    write_indent(output, indent);
    output.push_str("<MacroExpansion>\n");
    write_buildable_reference(output, project, target, indent + 1);
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
        write_indent(output, indent + 1);
        let _ = writeln!(
            output,
            "<EnvironmentVariable key=\"{}\" value=\"{}\" isEnabled=\"{}\"/>",
            xml_escape(&variable.variable),
            xml_escape(&variable.value),
            bool_xml(variable.enabled)
        );
    }
    write_indent(output, indent);
    output.push_str("</EnvironmentVariables>\n");
}

fn write_command_line_arguments(
    output: &mut String,
    arguments: &indexmap::IndexMap<String, bool>,
    indent: usize,
) {
    if arguments.is_empty() {
        return;
    }
    write_indent(output, indent);
    output.push_str("<CommandLineArguments>\n");
    for (argument, enabled) in arguments {
        write_indent(output, indent + 1);
        let _ = writeln!(
            output,
            "<CommandLineArgument argument=\"{}\" isEnabled=\"{}\"/>",
            xml_escape(argument),
            bool_xml(*enabled)
        );
    }
    write_indent(output, indent);
    output.push_str("</CommandLineArguments>\n");
}

fn write_build_for_attributes(output: &mut String, build_types: &[crate::spec::BuildType]) {
    let all = build_types.is_empty();
    let has = |kind| all || build_types.contains(&kind);
    let _ = write!(
        output,
        " buildForTesting=\"{}\" buildForRunning=\"{}\" buildForProfiling=\"{}\" buildForArchiving=\"{}\" buildForAnalyzing=\"{}\"",
        bool_xml(has(crate::spec::BuildType::Testing)),
        bool_xml(has(crate::spec::BuildType::Running)),
        bool_xml(has(crate::spec::BuildType::Profiling)),
        bool_xml(has(crate::spec::BuildType::Archiving)),
        bool_xml(has(crate::spec::BuildType::Analyzing))
    );
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
        .unwrap_or("1600")
        .to_owned()
}

fn write_indent(output: &mut String, indent: usize) {
    for _ in 0..indent {
        output.push_str("   ");
    }
}

fn bool_xml(value: bool) -> &'static str {
    if value {
        "YES"
    } else {
        "NO"
    }
}

fn info_plist_properties(target: &Target, plist: &Plist) -> indexmap::IndexMap<String, Value> {
    let mut properties = indexmap::IndexMap::new();
    properties.insert(
        "CFBundleIdentifier".to_owned(),
        Value::String("$(PRODUCT_BUNDLE_IDENTIFIER)".to_owned()),
    );
    properties.insert(
        "CFBundleInfoDictionaryVersion".to_owned(),
        Value::String("6.0".to_owned()),
    );
    properties.insert(
        "CFBundleName".to_owned(),
        Value::String("$(PRODUCT_NAME)".to_owned()),
    );
    properties.insert(
        "CFBundleDevelopmentRegion".to_owned(),
        Value::String("$(DEVELOPMENT_LANGUAGE)".to_owned()),
    );
    properties.insert(
        "CFBundleShortVersionString".to_owned(),
        Value::String("1.0".to_owned()),
    );
    properties.insert("CFBundleVersion".to_owned(), Value::String("1".to_owned()));
    if target.target_type != ProductType::Bundle {
        properties.insert(
            "CFBundleExecutable".to_owned(),
            Value::String("$(EXECUTABLE_NAME)".to_owned()),
        );
    }
    if let Some(package_type) = bundle_package_type(&target.target_type) {
        properties.insert(
            "CFBundlePackageType".to_owned(),
            Value::String(package_type.to_owned()),
        );
    }
    for (key, value) in &plist.attributes {
        properties.insert(key.clone(), value.clone());
    }
    properties
}

fn bundle_package_type(product_type: &ProductType) -> Option<&'static str> {
    match product_type {
        ProductType::UnitTestBundle | ProductType::UiTestBundle | ProductType::Bundle => {
            Some("BNDL")
        }
        ProductType::Application | ProductType::Watch2App => Some("APPL"),
        ProductType::Framework => Some("FMWK"),
        ProductType::XpcService | ProductType::AppExtension => Some("XPC!"),
        _ => None,
    }
}

fn plist_xml(properties: &indexmap::IndexMap<String, Value>) -> String {
    let mut output = String::new();
    output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    output.push_str("<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" ");
    output.push_str("\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n");
    output.push_str("<plist version=\"1.0\">\n");
    write_plist_dict(&mut output, properties, 0);
    output.push_str("</plist>\n");
    output
}

fn write_plist_dict(
    output: &mut String,
    values: &indexmap::IndexMap<String, Value>,
    indent: usize,
) {
    output.push_str(&"\t".repeat(indent));
    output.push_str("<dict>\n");
    for (key, value) in values {
        output.push_str(&"\t".repeat(indent + 1));
        let _ = writeln!(output, "<key>{}</key>", xml_escape(key));
        write_plist_value(output, value, indent + 1);
    }
    output.push_str(&"\t".repeat(indent));
    output.push_str("</dict>\n");
}

fn write_plist_value(output: &mut String, value: &Value, indent: usize) {
    output.push_str(&"\t".repeat(indent));
    match value {
        Value::Bool(true) => output.push_str("<true/>\n"),
        Value::Bool(false) => output.push_str("<false/>\n"),
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            let _ = writeln!(output, "<integer>{number}</integer>");
        }
        Value::Number(number) => {
            let _ = writeln!(output, "<real>{number}</real>");
        }
        Value::Array(items) => {
            output.push_str("<array>\n");
            for item in items {
                write_plist_value(output, item, indent + 1);
            }
            output.push_str(&"\t".repeat(indent));
            output.push_str("</array>\n");
        }
        Value::Object(map) => {
            output.push_str("<dict>\n");
            for (key, value) in map {
                output.push_str(&"\t".repeat(indent + 1));
                let _ = writeln!(output, "<key>{}</key>", xml_escape(key));
                write_plist_value(output, value, indent + 1);
            }
            output.push_str(&"\t".repeat(indent));
            output.push_str("</dict>\n");
        }
        Value::Null => output.push_str("<string></string>\n"),
        Value::String(value) => {
            let _ = writeln!(output, "<string>{}</string>", xml_escape(value));
        }
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn pbx_value_from_json(value: &Value) -> PbxValue {
    match value {
        Value::Bool(true) => PbxValue::String("YES".to_owned()),
        Value::Bool(false) => PbxValue::String("NO".to_owned()),
        Value::Number(number) => PbxValue::String(number.to_string()),
        Value::String(value) => PbxValue::String(value.clone()),
        Value::Array(values) => PbxValue::Array(values.iter().map(pbx_value_from_json).collect()),
        Value::Object(map) => PbxValue::Dict(
            map.iter()
                .map(|(key, value)| (key.clone(), pbx_value_from_json(value)))
                .collect(),
        ),
        Value::Null => PbxValue::String(String::new()),
    }
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
        _ => None,
    }
}

fn string_array(values: &[String]) -> PbxValue {
    PbxValue::Array(values.iter().cloned().map(PbxValue::String).collect())
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
    for destination in ["iOS", "tvOS", "watchOS", "visionOS", "macOS"] {
        if !has_supported_destination(target, destination) {
            continue;
        }
        match destination {
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
    for (destination, family) in [
        ("iOS", "1"),
        ("iOS", "2"),
        ("tvOS", "3"),
        ("watchOS", "4"),
        ("visionOS", "7"),
    ] {
        if has_supported_destination(target, destination) && !families.contains(&family) {
            families.push(family);
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

fn carthage_platform_dir(platform: &Platform) -> &'static str {
    match platform {
        Platform::Macos => "Mac",
        Platform::Tvos => "tvOS",
        Platform::Watchos => "watchOS",
        Platform::Visionos => "VisionOS",
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
        }),
        ProductType::AppExtension
        | ProductType::WatchExtension
        | ProductType::Watch2Extension
        | ProductType::TvExtension
        | ProductType::MessagesExtension
        | ProductType::StickerPack => Some(CopyFilesSettings {
            dst_subfolder_spec: 13,
            dst_path: String::new(),
            phase_name: "Copy Files".to_owned(),
        }),
        ProductType::ExtensionKitExtension => Some(CopyFilesSettings {
            dst_subfolder_spec: 16,
            dst_path: "$(EXTENSIONS_FOLDER_PATH)".to_owned(),
            phase_name: "Copy Files".to_owned(),
        }),
        _ => None,
    }
}

fn should_embed_target_dependency(dependency: &Dependency, target: &Target) -> bool {
    match dependency.embed {
        Some(value) => value,
        None => default_embed_target_product(target),
    }
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
            | ProductType::WatchExtension
            | ProductType::Watch2Extension
            | ProductType::TvExtension
            | ProductType::MessagesExtension
            | ProductType::StickerPack
            | ProductType::ExtensionKitExtension
            | ProductType::XpcService
            | ProductType::SystemExtension
            | ProductType::DriverExtension
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

fn should_embed_external_dependency(dependency: &Dependency) -> bool {
    match &dependency.dependency_type {
        DependencyType::Framework | DependencyType::Sdk { .. } | DependencyType::Package { .. } => {
            dependency.embed.unwrap_or(true)
        }
        DependencyType::Carthage { link_type, .. } => dependency
            .embed
            .unwrap_or(*link_type == crate::spec::CarthageLinkType::Dynamic),
        DependencyType::Target | DependencyType::Bundle => false,
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
                })
            }),
        DependencyType::Target | DependencyType::Bundle => None,
    }
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
        phase_name: if destination == "frameworks" {
            "Embed Frameworks".to_owned()
        } else {
            "Copy Files".to_owned()
        },
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

fn bool_int(value: bool) -> i64 {
    if value {
        1
    } else {
        0
    }
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

impl PbxGraph {
    fn add(&mut self, key: &str, object: PbxObject) -> String {
        let id = self.id_for(key);
        if !self.objects.contains_key(&id) {
            if let Some(comment) = object.comment.clone() {
                self.comments.insert(id.clone(), comment);
            }
            self.objects.insert(id.clone(), object);
        }
        id
    }

    fn add_or_merge_group(&mut self, key: &str, object: PbxObject) -> String {
        let id = self.id_for(key);
        let Some(existing) = self.objects.get_mut(&id) else {
            if let Some(comment) = object.comment.clone() {
                self.comments.insert(id.clone(), comment);
            }
            self.objects.insert(id.clone(), object);
            return id;
        };

        if let Some(PbxValue::Array(existing_children)) = existing.fields.get_mut("children") {
            if let Some(PbxValue::Array(new_children)) = object.fields.get("children") {
                for child in new_children {
                    if !existing_children.contains(child) {
                        existing_children.push(child.clone());
                    }
                }
            }
        }

        if !existing.fields.contains_key("name") {
            if let Some(name) = object.fields.get("name") {
                existing.fields.insert("name".to_owned(), name.clone());
            }
        }
        if !existing.fields.contains_key("path") {
            if let Some(path) = object.fields.get("path") {
                existing.fields.insert("path".to_owned(), path.clone());
            }
        }

        id
    }

    fn id_for(&self, key: &str) -> String {
        object_id(key, 0)
    }
}

impl PbxObject {
    fn new(isa: &'static str, comment: impl Into<String>) -> Self {
        Self {
            isa,
            comment: Some(comment.into()),
            fields: BTreeMap::new(),
        }
    }

    fn field(mut self, key: &str, value: PbxValue) -> Self {
        self.fields.insert(key.to_owned(), value);
        self
    }
}

impl PbxValue {
    fn reference(id: String, comment: impl Into<String>) -> Self {
        Self::Ref {
            id,
            comment: Some(comment.into()),
        }
    }

    fn write(&self, output: &mut String, indent: usize, comments: &HashMap<String, String>) {
        match self {
            Self::Int(value) => {
                let _ = write!(output, "{value}");
            }
            Self::String(value) => output.push_str(&quote(value)),
            Self::Ref { id, comment } => {
                output.push_str(id);
                let comment = comment
                    .as_ref()
                    .filter(|comment| !comment.is_empty())
                    .cloned()
                    .or_else(|| comments.get(id).cloned());
                if let Some(comment) = comment {
                    let _ = write!(output, " /* {comment} */");
                }
            }
            Self::Array(values) => {
                if values.is_empty() {
                    output.push_str("(\n");
                    output.push_str(&"\t".repeat(indent));
                    output.push(')');
                } else {
                    output.push_str("(\n");
                    for value in values {
                        output.push_str(&"\t".repeat(indent + 1));
                        value.write(output, indent + 1, comments);
                        output.push_str(",\n");
                    }
                    output.push_str(&"\t".repeat(indent));
                    output.push(')');
                }
            }
            Self::Dict(values) => {
                if values.is_empty() {
                    output.push_str("{\n");
                    output.push_str(&"\t".repeat(indent));
                    output.push('}');
                } else {
                    output.push_str("{\n");
                    for (key, value) in values {
                        let _ = write!(output, "{}{} = ", "\t".repeat(indent + 1), key);
                        value.write(output, indent + 1, comments);
                        output.push_str(";\n");
                    }
                    output.push_str(&"\t".repeat(indent));
                    output.push('}');
                }
            }
        }
    }
}

fn object_id(key: &str, salt: u64) -> String {
    let mut hash = 0xcbf29ce484222325u64 ^ salt;
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let second = hash.rotate_left(17) ^ 0x9e3779b97f4a7c15;
    format!("{hash:016X}{:08X}", second as u32)
}

fn quote(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_owned();
    }
    let bare_ok = value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '$'));
    if bare_ok {
        value.to_owned()
    } else {
        format!(
            "\"{}\"",
            value
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', "\\n")
                .replace('\r', "\\r")
        )
    }
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
    let mut files = Vec::new();
    collect_files(&path, &mut files, file_types);
    files.retain(|file| source_matches_filters(&path, file, source));
    files.sort();
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
            .any(|pattern| source_pattern_matches(pattern, &relative))
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

fn source_pattern_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_end_matches('/');
    let path = path.trim_start_matches("./");
    if pattern.is_empty() {
        return false;
    }
    if path == pattern || path.starts_with(&format!("{pattern}/")) {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return wildcard_match(suffix, path)
            || path.split('/').any(|part| wildcard_match(suffix, part));
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
        if name.starts_with('.')
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

fn collect_known_regions(path: &Path, regions: &mut BTreeSet<String>) {
    if path.is_file() {
        collect_string_catalog_regions(path, regions);
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.extension().and_then(|extension| extension.to_str()) == Some("lproj") {
                if let Some(region) = path.file_stem().and_then(|stem| stem.to_str()) {
                    regions.insert(region.to_owned());
                }
            }
            collect_known_regions(&path, regions);
        } else {
            collect_string_catalog_regions(&path, regions);
        }
    }
}

fn collect_string_catalog_regions(path: &Path, regions: &mut BTreeSet<String>) {
    if path.extension().and_then(|extension| extension.to_str()) != Some("xcstrings") {
        return;
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return;
    };
    let Ok(json) = serde_json::from_str::<Value>(&contents) else {
        return;
    };
    let Some(strings) = json.get("strings").and_then(Value::as_object) else {
        return;
    };
    for entry in strings.values() {
        if let Some(localizations) = entry.get("localizations").and_then(Value::as_object) {
            regions.extend(localizations.keys().cloned());
        }
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
                    | "framework"
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

        let left_byte = left[left_index].to_ascii_lowercase();
        let right_byte = right[right_index].to_ascii_lowercase();
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

fn build_phase_for_source(path: &str) -> Option<&'static str> {
    if is_framework_file(path) {
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

fn is_framework_file(path: &str) -> bool {
    matches!(
        Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str()),
        Some("framework" | "xcframework")
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
            ProductType::UnitTestBundle | ProductType::UiTestBundle => "wrapper.cfbundle",
            ProductType::CommandLineTool => "compiled.mach-o.executable",
            ProductType::XpcService => "wrapper.xpc-service",
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
        Some("m") => "sourcecode.c.objc",
        Some("mm") => "sourcecode.cpp.objcpp",
        Some("h") | Some("hh") | Some("hpp") | Some("ipp") | Some("tpp") | Some("hxx")
        | Some("def") => "sourcecode.c.h",
        Some("plist") => "text.plist",
        Some("xcassets") => "folder.assetcatalog",
        Some("storyboard") => "file.storyboard",
        Some("xib") => "file.xib",
        Some("framework") => "wrapper.framework",
        Some("a") => "archive.ar",
        Some("dylib") | Some("tbd") => "compiled.mach-o.dylib",
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
        ProductType::AppExtension => "com.apple.product-type.app-extension",
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

fn package_requirement(package: &serde_json::Value) -> BTreeMap<String, PbxValue> {
    let mut requirement = BTreeMap::new();
    if let Some(version) = package.get("exactVersion").and_then(|value| value.as_str()) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("exactVersion".to_owned()),
        );
        requirement.insert("version".to_owned(), PbxValue::String(version.to_owned()));
    } else if let Some(version) = package.get("version").and_then(|value| value.as_str()) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("exactVersion".to_owned()),
        );
        requirement.insert("version".to_owned(), PbxValue::String(version.to_owned()));
    } else if let Some(version) = package.get("majorVersion").and_then(|value| value.as_str()) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("upToNextMajorVersion".to_owned()),
        );
        requirement.insert(
            "minimumVersion".to_owned(),
            PbxValue::String(version.to_owned()),
        );
    } else if let Some(version) = package.get("minorVersion").and_then(|value| value.as_str()) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("upToNextMinorVersion".to_owned()),
        );
        requirement.insert(
            "minimumVersion".to_owned(),
            PbxValue::String(version.to_owned()),
        );
    } else if let Some(branch) = package.get("branch").and_then(|value| value.as_str()) {
        requirement.insert("kind".to_owned(), PbxValue::String("branch".to_owned()));
        requirement.insert("branch".to_owned(), PbxValue::String(branch.to_owned()));
    } else if let Some(revision) = package.get("revision").and_then(|value| value.as_str()) {
        requirement.insert("kind".to_owned(), PbxValue::String("revision".to_owned()));
        requirement.insert("revision".to_owned(), PbxValue::String(revision.to_owned()));
    } else if let Some(min) = package.get("minVersion").and_then(|value| value.as_str()) {
        requirement.insert(
            "kind".to_owned(),
            PbxValue::String("versionRange".to_owned()),
        );
        requirement.insert(
            "minimumVersion".to_owned(),
            PbxValue::String(min.to_owned()),
        );
        if let Some(max) = package.get("maxVersion").and_then(|value| value.as_str()) {
            requirement.insert(
                "maximumVersion".to_owned(),
                PbxValue::String(max.to_owned()),
            );
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
