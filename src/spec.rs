use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub(crate) type JsonMap = Map<String, Value>;

#[allow(dead_code)]
pub(crate) fn remove_empty_arrays_dictionaries_and_nulls(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let filtered = map
                .into_iter()
                .filter_map(|(key, value)| {
                    let value = remove_empty_arrays_dictionaries_and_nulls(value);
                    match &value {
                        Value::Null => None,
                        Value::Array(items) if items.is_empty() => None,
                        Value::Object(map) if map.is_empty() => None,
                        _ => Some((key, value)),
                    }
                })
                .collect();
            Value::Object(filtered)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .filter_map(|value| {
                    let value = remove_empty_arrays_dictionaries_and_nulls(value);
                    match &value {
                        Value::Null => None,
                        Value::Array(items) if items.is_empty() => None,
                        Value::Object(map) if map.is_empty() => None,
                        _ => Some(value),
                    }
                })
                .collect(),
        ),
        value => value,
    }
}

pub(crate) fn format_deployment_target(value: impl ToString) -> Result<String, SpecError> {
    let value = value.to_string();
    let parts = value.split('.').collect::<Vec<_>>();
    if parts.is_empty()
        || parts.len() > 3
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()))
    {
        return Err(SpecError::InvalidVersion(value));
    }
    Ok(match parts.as_slice() {
        [major] => format!("{major}.0"),
        [major, minor] => format!("{major}.{minor}"),
        [major, minor, "0"] => format!("{major}.{minor}"),
        [major, minor, patch] => format!("{major}.{minor}.{patch}"),
        _ => unreachable!(),
    })
}

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse {path}: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("expected top-level mapping in {0}")]
    ExpectedMapping(PathBuf),
    #[error("project spec is missing required key `{0}`")]
    MissingRequired(&'static str),
    #[error("unknown target type `{0}`")]
    UnknownTargetType(String),
    #[error("unknown target platform `{0}`")]
    UnknownTargetPlatform(String),
    #[error("target cannot use platform array and supportedDestinations together")]
    InvalidTargetPlatformAsArray,
    #[error("invalid dependency {0}")]
    InvalidDependency(String),
    #[error("unknown breakpoint {kind} `{value}`")]
    UnknownBreakpoint {
        kind: BreakpointField,
        value: String,
    },
    #[error("invalid configs mapping format for keys: {0:?}")]
    InvalidConfigsMappingFormat(Vec<String>),
    #[error("invalid version `{0}`")]
    InvalidVersion(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakpointField {
    Type,
    Scope,
    StopOnStyle,
    ActionType,
    ActionConveyanceType,
    ActionSoundName,
}

impl std::fmt::Display for BreakpointField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Type => "type",
            Self::Scope => "scope",
            Self::StopOnStyle => "stop-on style",
            Self::ActionType => "action type",
            Self::ActionConveyanceType => "action conveyance type",
            Self::ActionSoundName => "action sound name",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("project spec validation failed: {errors:?}")]
pub struct SpecValidationError {
    pub errors: Vec<ValidationError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    InvalidXcodeGenVersion {
        minimum_version: String,
        version: String,
    },
    MissingDefaultConfig {
        config_name: String,
    },
    InvalidConfigFileConfig(String),
    InvalidBuildSettingConfig(String),
    InvalidSettingsGroup(String),
    InvalidConfigFile {
        config_file: String,
        config: String,
    },
    InvalidFileGroup(String),
    InvalidLocalPackage(String),
    InvalidProjectReferencePath {
        name: String,
        path: String,
    },
    DuplicateDependencies {
        target: String,
        dependency_reference: String,
    },
    EmptySourcePath {
        target: String,
    },
    InvalidTargetDependency {
        target: String,
        dependency: String,
    },
    InvalidSwiftPackage {
        name: String,
        target: String,
    },
    InvalidSdkDependency {
        target: String,
        dependency: String,
    },
    InvalidTargetSource {
        target: String,
        source: String,
    },
    InvalidBuildScriptPath {
        target: String,
        name: Option<String>,
        path: String,
    },
    InvalidPluginPackageReference {
        plugin: String,
        package: String,
    },
    InvalidTargetConfigFile {
        target: String,
        config_file: String,
        config: String,
    },
    InvalidTargetSchemeTest {
        target: String,
        test_target: String,
    },
    InvalidSchemeTarget {
        scheme: String,
        target: String,
        action: String,
    },
    InvalidSchemeConfig {
        scheme: String,
        config: String,
    },
    InvalidProjectReference {
        scheme: String,
        reference: String,
    },
    InvalidTestPlan(TestPlan),
    InvalidPerConfigSettings,
    MultipleDefaultTestPlans,
    UnexpectedTargetPlatformForSupportedDestinations {
        target: String,
        platform: Platform,
    },
    ContainsWatchOSDestinationForMultiplatformApp {
        target: String,
    },
    MultipleMacPlatformsInSupportedDestinations {
        target: String,
    },
    InvalidTargetPlatformForSupportedDestinations {
        target: String,
    },
    MissingTargetPlatformInSupportedDestinations {
        target: String,
        platform: Platform,
    },
}

#[derive(Debug, Clone)]
struct Include {
    path: PathBuf,
    relative_paths: bool,
    enable: bool,
}

impl Include {
    fn parse(value: Option<&Value>) -> Vec<Self> {
        match value {
            Some(Value::Array(items)) => items.iter().filter_map(Self::from_value).collect(),
            Some(value) => Self::from_value(value).into_iter().collect(),
            None => Vec::new(),
        }
    }

    fn from_value(value: &Value) -> Option<Self> {
        match value {
            Value::String(path) => Some(Self {
                path: PathBuf::from(path),
                relative_paths: true,
                enable: true,
            }),
            Value::Object(map) => {
                let path = map.get("path")?.as_str()?;
                Some(Self {
                    path: PathBuf::from(path),
                    relative_paths: boolish(map.get("relativePaths")).unwrap_or(true),
                    enable: boolish(map.get("enable")).unwrap_or(true),
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SpecFile {
    pub base_path: PathBuf,
    pub file_path: PathBuf,
    pub relative_path: PathBuf,
    pub dictionary: JsonMap,
    pub sub_specs: Vec<SpecFile>,
}

impl SpecFile {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SpecError> {
        Self::load_with_options(path, None, HashMap::new())
    }

    pub fn load_with_options(
        path: impl AsRef<Path>,
        project_root: Option<PathBuf>,
        variables: HashMap<String, String>,
    ) -> Result<Self, SpecError> {
        let path = path.as_ref();
        let absolute = absolute_path(path);
        let base_path = project_root
            .map(absolute_path)
            .unwrap_or_else(|| absolute.parent().unwrap_or(Path::new("")).to_path_buf());
        let file_path = pathdiff(&absolute, &base_path);
        let mut cache = HashMap::new();
        Self::load_inner(file_path, base_path, PathBuf::new(), &variables, &mut cache)
    }

    fn load_include(
        include: Include,
        base_path: PathBuf,
        relative_path: PathBuf,
        variables: &HashMap<String, String>,
        cache: &mut HashMap<PathBuf, SpecFile>,
    ) -> Result<Self, SpecError> {
        let include_base = if include.relative_paths {
            normalize_path(base_path.join(&relative_path))
        } else {
            base_path
        };
        let include_relative = if include.relative_paths {
            include.path.parent().unwrap_or(Path::new("")).to_path_buf()
        } else {
            PathBuf::new()
        };
        Self::load_inner(
            include.path,
            include_base,
            include_relative,
            variables,
            cache,
        )
    }

    fn load_inner(
        file_path: PathBuf,
        base_path: PathBuf,
        relative_path: PathBuf,
        variables: &HashMap<String, String>,
        cache: &mut HashMap<PathBuf, SpecFile>,
    ) -> Result<Self, SpecError> {
        let path = normalize_path(base_path.join(&file_path));
        if let Some(cached) = cache.get(&path) {
            return Ok(cached.clone());
        }

        let mut dictionary = load_dictionary(&path)?;
        expand_variables_in_map(&mut dictionary, variables);
        let includes = Include::parse(dictionary.get("include"));
        let sub_specs = includes
            .into_iter()
            .filter(|include| include.enable)
            .map(|include| {
                Self::load_include(
                    include,
                    base_path.clone(),
                    relative_path.clone(),
                    variables,
                    cache,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        let spec = Self {
            base_path: base_path.clone(),
            file_path,
            relative_path,
            dictionary,
            sub_specs,
        };
        cache.insert(path, spec.clone());
        Ok(spec)
    }

    pub fn resolved_dictionary(&self) -> JsonMap {
        let mut resolved_cache = HashMap::new();
        let resolved_spec = self.resolving_paths(&mut resolved_cache, Path::new(""));
        let mut seen = HashSet::new();
        resolved_spec.merge_unique(&mut seen)
    }

    fn resolving_paths(
        &self,
        cache: &mut HashMap<PathBuf, SpecFile>,
        relative_to: &Path,
    ) -> SpecFile {
        let path = normalize_path(relative_to.join(&self.file_path));
        if let Some(cached) = cache.get(&path) {
            return cached.clone();
        }

        let relative_path = normalize_path(relative_to.join(&self.relative_path));
        let mut dictionary = self.dictionary.clone();
        if !relative_path.as_os_str().is_empty() {
            resolve_project_paths(&mut dictionary, &relative_path);
        }
        let spec = SpecFile {
            base_path: self.base_path.clone(),
            file_path: self.file_path.clone(),
            relative_path: self.relative_path.clone(),
            dictionary,
            sub_specs: self
                .sub_specs
                .iter()
                .map(|sub_spec| sub_spec.resolving_paths(cache, &relative_path))
                .collect(),
        };
        cache.insert(path, spec.clone());
        spec
    }

    fn merge_unique(self, seen: &mut HashSet<PathBuf>) -> JsonMap {
        let path = normalize_path(self.base_path.join(&self.file_path));
        if !seen.insert(path) {
            return JsonMap::new();
        }

        let mut merged = JsonMap::new();
        for sub_spec in self.sub_specs {
            merged = merge_maps(sub_spec.merge_unique(seen), merged);
        }
        merge_maps(self.dictionary, merged)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub base_path: PathBuf,
    pub name: String,
    pub configs: IndexMap<String, String>,
    pub targets: IndexMap<String, Target>,
    pub aggregate_targets: IndexMap<String, Value>,
    pub aggregate_target_specs: IndexMap<String, AggregateTarget>,
    pub packages: IndexMap<String, Value>,
    pub package_specs: IndexMap<String, SwiftPackage>,
    pub settings: Value,
    pub settings_spec: Settings,
    pub setting_groups: IndexMap<String, Value>,
    pub setting_group_specs: IndexMap<String, Settings>,
    pub schemes: Value,
    pub scheme_specs: IndexMap<String, Scheme>,
    pub breakpoints: Vec<Breakpoint>,
    pub options: Value,
    pub spec_options: SpecOptions,
    pub attributes: Value,
    pub file_groups: Vec<String>,
    pub config_files: IndexMap<String, String>,
    pub include: Value,
    pub project_references: IndexMap<String, Value>,
    pub raw: JsonMap,
}

impl Project {
    pub fn from_spec(spec: &SpecFile) -> Result<Self, SpecError> {
        Self::from_dictionary(spec.base_path.clone(), spec.resolved_dictionary())
    }

    pub fn from_dictionary(base_path: PathBuf, dictionary: JsonMap) -> Result<Self, SpecError> {
        let dictionary = resolve_project(dictionary)?;
        validate_configs_mapping_format(&dictionary)?;
        validate_package_versions(dictionary.get("packages"))?;
        let name = dictionary
            .get("name")
            .and_then(Value::as_str)
            .ok_or(SpecError::MissingRequired("name"))?
            .to_owned();

        let mut packages = parse_value_map(dictionary.get("packages"));
        for local_package in parse_string_array(dictionary.get("localPackages")) {
            let package_name = normalize_path(base_path.join(&local_package))
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&local_package)
                .to_owned();
            packages.entry(package_name).or_insert_with(|| {
                serde_json::json!({
                    "path": local_package,
                    "group": null,
                    "excludeFromProject": false
                })
            });
        }
        let package_specs = parse_package_value_map(&packages);

        Ok(Self {
            base_path: base_path.clone(),
            name,
            configs: parse_string_map(dictionary.get("configs")).unwrap_or_else(default_configs),
            targets: parse_targets(dictionary.get("targets"))?,
            aggregate_targets: parse_value_map(dictionary.get("aggregateTargets")),
            aggregate_target_specs: parse_aggregate_targets(dictionary.get("aggregateTargets")),
            packages,
            package_specs,
            settings: dictionary.get("settings").cloned().unwrap_or(Value::Null),
            settings_spec: Settings::from_value(dictionary.get("settings")),
            setting_groups: parse_value_map(dictionary.get("settingGroups")),
            setting_group_specs: parse_setting_groups(dictionary.get("settingGroups")),
            schemes: dictionary
                .get("schemes")
                .cloned()
                .unwrap_or(Value::Array(Vec::new())),
            scheme_specs: parse_schemes(dictionary.get("schemes")),
            breakpoints: parse_breakpoints(dictionary.get("breakpoints"))?,
            options: dictionary
                .get("options")
                .cloned()
                .unwrap_or(Value::Object(JsonMap::new())),
            spec_options: SpecOptions::from_value(dictionary.get("options")),
            attributes: dictionary
                .get("attributes")
                .cloned()
                .unwrap_or(Value::Object(JsonMap::new())),
            file_groups: parse_string_array(dictionary.get("fileGroups")),
            config_files: parse_string_map(dictionary.get("configFiles")).unwrap_or_default(),
            include: dictionary.get("include").cloned().unwrap_or(Value::Null),
            project_references: parse_value_map(dictionary.get("projectReferences")),
            raw: dictionary,
        })
    }

    pub fn default_project_path(&self) -> PathBuf {
        self.base_path.join(format!("{}.xcodeproj", self.name))
    }

    pub fn validate(&self) -> Result<(), SpecValidationError> {
        let mut errors = Vec::new();
        if let Some(default_config) = &self.spec_options.default_config {
            if !self.configs.contains_key(default_config) {
                errors.push(ValidationError::MissingDefaultConfig {
                    config_name: default_config.clone(),
                });
            }
        }
        for (config, config_file) in &self.config_files {
            if !self.configs.contains_key(config) {
                errors.push(ValidationError::InvalidConfigFileConfig(config.clone()));
            }
            if !self.path_exists(config_file) {
                errors.push(ValidationError::InvalidConfigFile {
                    config_file: config_file.clone(),
                    config: config.clone(),
                });
            }
        }
        for file_group in &self.file_groups {
            if !self.path_exists(file_group) {
                errors.push(ValidationError::InvalidFileGroup(file_group.clone()));
            }
        }
        for (name, package) in &self.package_specs {
            if let SwiftPackage::Local { path, .. } = package {
                if !self.path_exists(path) {
                    errors.push(ValidationError::InvalidLocalPackage(name.clone()));
                }
            }
        }
        for (name, reference) in &self.project_references {
            if let Some(path) = reference.get("path").and_then(Value::as_str) {
                if !self.path_exists(path) {
                    errors.push(ValidationError::InvalidProjectReferencePath {
                        name: name.clone(),
                        path: path.to_owned(),
                    });
                }
            }
        }
        collect_per_config_settings_validation_errors(self, &self.settings_spec, &mut errors);
        collect_settings_validation_errors(self, &self.settings_spec, &mut errors);
        for settings in self.setting_group_specs.values() {
            collect_per_config_settings_validation_errors(self, settings, &mut errors);
            collect_settings_validation_errors(self, settings, &mut errors);
        }
        for target in self.targets.values() {
            collect_target_validation_errors(self, target, &mut errors);
        }
        for aggregate in self.aggregate_target_specs.values() {
            collect_settings_validation_errors(self, &aggregate.settings_spec, &mut errors);
            collect_aggregate_target_validation_errors(self, aggregate, &mut errors);
        }
        for scheme in self.scheme_specs.values() {
            collect_scheme_validation_errors(self, scheme, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(SpecValidationError { errors })
        }
    }

    pub fn validate_minimum_xcodegen_version(
        &self,
        version: &str,
    ) -> Result<(), SpecValidationError> {
        let Some(minimum_version) = &self.spec_options.minimum_xcodegen_version else {
            return Ok(());
        };
        if compare_versions(version, minimum_version).is_some_and(|ordering| ordering.is_lt()) {
            Err(SpecValidationError {
                errors: vec![ValidationError::InvalidXcodeGenVersion {
                    minimum_version: minimum_version.clone(),
                    version: version.to_owned(),
                }],
            })
        } else {
            Ok(())
        }
    }

    fn path_exists(&self, path: &str) -> bool {
        path_has_wildcards(path) || normalize_path(self.base_path.join(path)).exists()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecOptions {
    pub carthage_build_path: Option<String>,
    pub carthage_executable_path: Option<String>,
    pub create_intermediate_groups: bool,
    pub default_source_directory_type: Option<SourceType>,
    pub bundle_id_prefix: Option<String>,
    pub development_language: Option<String>,
    pub deployment_target: DeploymentTarget,
    pub file_types: IndexMap<String, FileType>,
    pub find_carthage_frameworks: bool,
    pub use_base_internationalization: bool,
    pub pre_gen_command: Option<String>,
    pub post_gen_command: Option<String>,
    pub scheme_path_prefix: Option<String>,
    pub local_packages_group: Option<String>,
    pub default_config: Option<String>,
    pub minimum_xcodegen_version: Option<String>,
    pub setting_presets_none: bool,
    pub transitively_link_dependencies: bool,
    pub group_sort_position: GroupSortPosition,
    pub group_sort_position_explicit: bool,
    pub group_ordering: Vec<GroupOrdering>,
    pub uses_tabs: Option<bool>,
    pub indent_width: Option<i64>,
    pub tab_width: Option<i64>,
}

impl Default for SpecOptions {
    fn default() -> Self {
        Self {
            carthage_build_path: None,
            carthage_executable_path: None,
            create_intermediate_groups: false,
            default_source_directory_type: None,
            bundle_id_prefix: None,
            development_language: None,
            deployment_target: DeploymentTarget::default(),
            file_types: IndexMap::new(),
            find_carthage_frameworks: false,
            use_base_internationalization: true,
            pre_gen_command: None,
            post_gen_command: None,
            scheme_path_prefix: None,
            local_packages_group: None,
            default_config: None,
            minimum_xcodegen_version: None,
            setting_presets_none: false,
            transitively_link_dependencies: false,
            group_sort_position: GroupSortPosition::Top,
            group_sort_position_explicit: false,
            group_ordering: Vec::new(),
            uses_tabs: None,
            indent_width: None,
            tab_width: None,
        }
    }
}

impl SpecOptions {
    fn from_value(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::default();
        };
        Self {
            carthage_build_path: string_at(map, "carthageBuildPath"),
            carthage_executable_path: string_at(map, "carthageExecutablePath"),
            create_intermediate_groups: boolish(map.get("createIntermediateGroups"))
                .unwrap_or(false),
            default_source_directory_type: string_at(map, "defaultSourceDirectoryType")
                .map(|value| SourceType::parse(&value)),
            bundle_id_prefix: string_at(map, "bundleIdPrefix"),
            development_language: string_at(map, "developmentLanguage"),
            deployment_target: DeploymentTarget::from_value(map.get("deploymentTarget")),
            file_types: parse_file_types(map.get("fileTypes")),
            find_carthage_frameworks: boolish(map.get("findCarthageFrameworks")).unwrap_or(false),
            use_base_internationalization: boolish(map.get("useBaseInternationalization"))
                .unwrap_or(true),
            pre_gen_command: string_at(map, "preGenCommand"),
            post_gen_command: string_at(map, "postGenCommand"),
            scheme_path_prefix: string_at(map, "schemePathPrefix"),
            local_packages_group: string_at(map, "localPackagesGroup"),
            default_config: string_at(map, "defaultConfig"),
            minimum_xcodegen_version: scalar_to_string(map.get("minimumXcodeGenVersion")),
            setting_presets_none: string_at(map, "settingPresets")
                .is_some_and(|value| value == "none"),
            transitively_link_dependencies: boolish(map.get("transitivelyLinkDependencies"))
                .unwrap_or(false),
            group_sort_position: GroupSortPosition::from_value(map.get("groupSortPosition")),
            group_sort_position_explicit: map.contains_key("groupSortPosition"),
            group_ordering: parse_group_ordering(map.get("groupOrdering")),
            uses_tabs: boolish(map.get("usesTabs")),
            indent_width: map.get("indentWidth").and_then(Value::as_i64),
            tab_width: map.get("tabWidth").and_then(Value::as_i64),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GroupSortPosition {
    #[default]
    Top,
    Bottom,
}

impl GroupSortPosition {
    fn from_value(value: Option<&Value>) -> Self {
        match scalar_to_string(value).as_deref() {
            Some("bottom") => Self::Bottom,
            _ => Self::Top,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupOrdering {
    pub pattern: Option<String>,
    pub order: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeploymentTarget {
    pub ios: Option<String>,
    pub tvos: Option<String>,
    pub watchos: Option<String>,
    pub macos: Option<String>,
    pub visionos: Option<String>,
}

impl DeploymentTarget {
    fn from_value(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::default();
        };
        Self {
            ios: scalar_to_string(map.get("iOS"))
                .and_then(|value| format_deployment_target(value).ok()),
            tvos: scalar_to_string(map.get("tvOS"))
                .and_then(|value| format_deployment_target(value).ok()),
            watchos: scalar_to_string(map.get("watchOS"))
                .and_then(|value| format_deployment_target(value).ok()),
            macos: scalar_to_string(map.get("macOS"))
                .and_then(|value| format_deployment_target(value).ok()),
            visionos: scalar_to_string(map.get("visionOS"))
                .and_then(|value| format_deployment_target(value).ok()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileType {
    pub file: bool,
    pub build_phase: Option<FileBuildPhase>,
    pub attributes: Vec<String>,
    pub resource_tags: Vec<String>,
    pub compiler_flags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileBuildPhase {
    Sources,
    Resources,
    Headers,
    None,
    Other(String),
}

impl FileBuildPhase {
    fn parse(value: &str) -> Self {
        match value {
            "sources" => Self::Sources,
            "resources" => Self::Resources,
            "headers" => Self::Headers,
            "none" => Self::None,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AggregateTarget {
    pub name: String,
    pub targets: Vec<String>,
    pub settings: Value,
    pub settings_spec: Settings,
    pub config_files: IndexMap<String, String>,
    pub build_scripts: Vec<BuildScript>,
    pub build_tool_plugins: Vec<BuildToolPlugin>,
    pub scheme: Option<TargetScheme>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SwiftPackage {
    Remote {
        url: String,
        version_requirement: PackageVersionRequirement,
    },
    Local {
        path: String,
        group: Option<String>,
        exclude_from_project: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PackageVersionRequirement {
    Exact(String),
    UpToNextMajorVersion(String),
    UpToNextMinorVersion(String),
    Branch(String),
    Revision(String),
    Range { from: String, to: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Settings {
    pub build_settings: IndexMap<String, Value>,
    pub config_settings: IndexMap<String, Settings>,
    pub groups: Vec<String>,
}

impl Settings {
    fn from_value(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::default();
        };
        let has_structured_keys =
            map.contains_key("base") || map.contains_key("configs") || map.contains_key("groups");
        let build_settings = if has_structured_keys {
            parse_value_map(map.get("base"))
        } else {
            map.iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        };
        let config_settings = if let Some(Value::Object(configs)) = map.get("configs") {
            configs
                .iter()
                .map(|(key, value)| (key.clone(), Settings::from_value(Some(value))))
                .collect()
        } else {
            IndexMap::new()
        };
        Self {
            build_settings,
            config_settings,
            groups: parse_string_array(map.get("groups")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plist {
    pub path: Option<String>,
    pub attributes: IndexMap<String, Value>,
}

impl Plist {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        match value? {
            Value::String(path) => Some(Self {
                path: Some(path.clone()),
                attributes: IndexMap::new(),
            }),
            Value::Object(map) => {
                let path = string_at(map, "path");
                let attributes = map
                    .get("properties")
                    .and_then(Value::as_object)
                    .map(|properties| {
                        properties
                            .iter()
                            .map(|(key, value)| (key.clone(), value.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                Some(Self { path, attributes })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TargetScheme {
    pub test_targets: Vec<String>,
    pub test_target_options: Vec<SchemeTestTarget>,
    pub test_plans: Vec<TestPlan>,
    pub config_variants: Vec<String>,
    pub gather_coverage_data: bool,
    pub coverage_targets: Vec<String>,
    pub store_kit_configuration: Option<String>,
    pub language: Option<String>,
    pub region: Option<String>,
    pub disable_main_thread_checker: bool,
    pub stop_on_every_main_thread_checker_issue: bool,
    pub disable_thread_performance_checker: bool,
    pub command_line_arguments: IndexMap<String, bool>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub pre_actions: Vec<SchemeAction>,
    pub post_actions: Vec<SchemeAction>,
    pub management: Option<SchemeManagement>,
}

impl TargetScheme {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let Value::Object(map) = value? else {
            return None;
        };
        Some(Self {
            test_targets: parse_target_names(map.get("testTargets")),
            test_target_options: parse_test_targets(map.get("testTargets")),
            test_plans: parse_test_plans(map.get("testPlans")),
            config_variants: parse_string_array(map.get("configVariants")),
            gather_coverage_data: boolish(map.get("gatherCoverageData")).unwrap_or(false),
            coverage_targets: parse_string_array(map.get("coverageTargets")),
            store_kit_configuration: string_at(map, "storeKitConfiguration"),
            language: string_at(map, "language"),
            region: string_at(map, "region"),
            disable_main_thread_checker: boolish(map.get("disableMainThreadChecker"))
                .unwrap_or(false),
            stop_on_every_main_thread_checker_issue: boolish(
                map.get("stopOnEveryMainThreadCheckerIssue"),
            )
            .unwrap_or(false),
            disable_thread_performance_checker: boolish(map.get("disableThreadPerformanceChecker"))
                .unwrap_or(false),
            command_line_arguments: parse_bool_map(map.get("commandLineArguments")),
            environment_variables: parse_environment_variables(map.get("environmentVariables")),
            pre_actions: parse_scheme_actions(map.get("preActions")),
            post_actions: parse_scheme_actions(map.get("postActions")),
            management: SchemeManagement::from_value(map.get("management")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scheme {
    pub name: String,
    pub build: SchemeBuild,
    pub run: Option<SchemeRun>,
    pub test: Option<SchemeTest>,
    pub profile: Option<SchemeExecutionAction>,
    pub analyze: Option<SchemeExecutionAction>,
    pub archive: Option<SchemeExecutionAction>,
    pub management: SchemeManagement,
    pub raw: Value,
}

impl Scheme {
    fn from_entry(name: &str, value: &Value) -> Self {
        let map = value.as_object().cloned().unwrap_or_default();
        Self {
            name: string_at(&map, "name").unwrap_or_else(|| name.to_owned()),
            build: SchemeBuild::from_value(map.get("build")),
            run: map.get("run").and_then(SchemeRun::from_value),
            test: map.get("test").and_then(SchemeTest::from_value),
            profile: map
                .get("profile")
                .and_then(SchemeExecutionAction::from_value),
            analyze: map
                .get("analyze")
                .and_then(SchemeExecutionAction::from_value),
            archive: map
                .get("archive")
                .and_then(SchemeExecutionAction::from_value),
            management: SchemeManagement::from_value(map.get("management")).unwrap_or_default(),
            raw: value.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeBuild {
    pub targets: Vec<SchemeBuildTarget>,
    pub parallelize_build: bool,
    pub build_implicit_dependencies: bool,
    pub run_post_actions_on_failure: bool,
    pub pre_actions: Vec<SchemeAction>,
    pub post_actions: Vec<SchemeAction>,
}

impl Default for SchemeBuild {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            parallelize_build: true,
            build_implicit_dependencies: true,
            run_post_actions_on_failure: false,
            pre_actions: Vec::new(),
            post_actions: Vec::new(),
        }
    }
}

impl SchemeBuild {
    fn from_value(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::default();
        };
        Self {
            targets: parse_build_targets(map.get("targets")),
            parallelize_build: boolish(map.get("parallelizeBuild")).unwrap_or(true),
            build_implicit_dependencies: boolish(map.get("buildImplicitDependencies"))
                .unwrap_or(true),
            run_post_actions_on_failure: boolish(map.get("runPostActionsOnFailure"))
                .unwrap_or(false),
            pre_actions: parse_scheme_actions(map.get("preActions")),
            post_actions: parse_scheme_actions(map.get("postActions")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeBuildTarget {
    pub target: String,
    pub build_types: Vec<BuildType>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildType {
    Running,
    Testing,
    Profiling,
    Analyzing,
    Archiving,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeRun {
    pub config: Option<String>,
    pub macro_expansion: Option<String>,
    pub enable_gpu_frame_capture_mode: Option<String>,
    pub ask_for_app_to_launch: bool,
    pub debug_enabled: bool,
    pub launch_automatically_substyle: Option<String>,
    pub simulate_location: Option<SchemeSimulateLocation>,
    pub store_kit_configuration: Option<String>,
    pub custom_lldb_init: Option<String>,
    pub custom_working_directory: Option<String>,
    pub disable_main_thread_checker: bool,
    pub stop_on_every_main_thread_checker_issue: bool,
    pub disable_thread_performance_checker: bool,
    pub language: Option<String>,
    pub region: Option<String>,
    pub command_line_arguments: IndexMap<String, bool>,
    pub environment_variables: Vec<EnvironmentVariable>,
}

impl SchemeRun {
    fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        Some(Self {
            config: string_at(map, "config"),
            macro_expansion: string_at(map, "macroExpansion"),
            enable_gpu_frame_capture_mode: string_at(map, "enableGPUFrameCaptureMode"),
            ask_for_app_to_launch: boolish(map.get("askForAppToLaunch")).unwrap_or(false),
            debug_enabled: boolish(map.get("debugEnabled")).unwrap_or(true),
            launch_automatically_substyle: scalar_to_string(map.get("launchAutomaticallySubstyle")),
            simulate_location: SchemeSimulateLocation::from_value(map.get("simulateLocation")),
            store_kit_configuration: string_at(map, "storeKitConfiguration"),
            custom_lldb_init: string_at(map, "customLLDBInit"),
            custom_working_directory: string_at(map, "customWorkingDirectory"),
            disable_main_thread_checker: boolish(map.get("disableMainThreadChecker"))
                .unwrap_or(false),
            stop_on_every_main_thread_checker_issue: boolish(
                map.get("stopOnEveryMainThreadCheckerIssue"),
            )
            .unwrap_or(false),
            disable_thread_performance_checker: boolish(map.get("disableThreadPerformanceChecker"))
                .unwrap_or(false),
            language: string_at(map, "language"),
            region: string_at(map, "region"),
            command_line_arguments: parse_bool_map(map.get("commandLineArguments")),
            environment_variables: parse_environment_variables(map.get("environmentVariables")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeSimulateLocation {
    pub allow: bool,
    pub default_location: Option<String>,
}

impl SchemeSimulateLocation {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let Value::Object(map) = value? else {
            return None;
        };
        Some(Self {
            allow: boolish(map.get("allow")).unwrap_or(true),
            default_location: string_at(map, "defaultLocation"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemeTest {
    pub config: Option<String>,
    pub macro_expansion: Option<String>,
    pub gather_coverage_data: bool,
    pub debug_enabled: bool,
    pub custom_lldb_init: Option<String>,
    pub disable_main_thread_checker: bool,
    pub stop_on_every_main_thread_checker_issue: bool,
    pub targets: Vec<SchemeTestTarget>,
    pub coverage_targets: Vec<String>,
    pub test_plans: Vec<TestPlan>,
    pub pre_actions: Vec<SchemeAction>,
    pub post_actions: Vec<SchemeAction>,
    pub command_line_arguments: IndexMap<String, bool>,
    pub environment_variables: Vec<EnvironmentVariable>,
    pub capture_screenshots_automatically: Option<bool>,
    pub delete_screenshots_when_each_test_succeeds: Option<bool>,
    pub preferred_screen_capture_format: Option<String>,
}

impl SchemeTest {
    fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        Some(Self {
            config: string_at(map, "config"),
            macro_expansion: string_at(map, "macroExpansion"),
            gather_coverage_data: boolish(map.get("gatherCoverageData")).unwrap_or(false),
            debug_enabled: boolish(map.get("debugEnabled")).unwrap_or(true),
            custom_lldb_init: string_at(map, "customLLDBInit"),
            disable_main_thread_checker: boolish(map.get("disableMainThreadChecker"))
                .unwrap_or(false),
            stop_on_every_main_thread_checker_issue: boolish(
                map.get("stopOnEveryMainThreadCheckerIssue"),
            )
            .unwrap_or(false),
            targets: parse_test_targets(map.get("targets")),
            coverage_targets: parse_string_array(map.get("coverageTargets")),
            test_plans: parse_test_plans(map.get("testPlans")),
            pre_actions: parse_scheme_actions(map.get("preActions")),
            post_actions: parse_scheme_actions(map.get("postActions")),
            command_line_arguments: parse_bool_map(map.get("commandLineArguments")),
            environment_variables: parse_environment_variables(map.get("environmentVariables")),
            capture_screenshots_automatically: boolish(map.get("captureScreenshotsAutomatically")),
            delete_screenshots_when_each_test_succeeds: boolish(
                map.get("deleteScreenshotsWhenEachTestSucceeds"),
            ),
            preferred_screen_capture_format: string_at(map, "preferredScreenCaptureFormat"),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeTestTarget {
    pub target_reference: String,
    pub random_execution_order: bool,
    pub parallelizable: bool,
    pub location: Option<String>,
    pub skipped: bool,
    pub skipped_tests: Vec<String>,
    pub selected_tests: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeExecutionAction {
    pub config: Option<String>,
    pub ask_for_app_to_launch: bool,
    pub environment_variables: Vec<EnvironmentVariable>,
}

impl SchemeExecutionAction {
    fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        Some(Self {
            config: string_at(map, "config"),
            ask_for_app_to_launch: boolish(map.get("askForAppToLaunch")).unwrap_or(false),
            environment_variables: parse_environment_variables(map.get("environmentVariables")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeAction {
    pub name: String,
    pub script: String,
    pub settings_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentVariable {
    pub variable: String,
    pub value: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPlan {
    pub path: String,
    pub default_plan: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemeManagement {
    pub shared: bool,
    pub order_hint: Option<i64>,
    pub is_shown: Option<bool>,
}

impl Default for SchemeManagement {
    fn default() -> Self {
        Self {
            shared: true,
            order_hint: None,
            is_shown: None,
        }
    }
}

impl SchemeManagement {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        let Value::Object(map) = value? else {
            return None;
        };
        Some(Self {
            shared: boolish(map.get("shared")).unwrap_or(true),
            order_hint: map.get("orderHint").and_then(Value::as_i64),
            is_shown: boolish(map.get("isShown")),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub name: String,
    pub product_name: String,
    pub target_type: ProductType,
    pub platform: Platform,
    pub supported_destinations: Vec<String>,
    pub deployment_target: Option<String>,
    pub deployment_targets: DeploymentTarget,
    pub settings: Value,
    pub settings_spec: Settings,
    pub config_files: IndexMap<String, String>,
    pub sources: Vec<TargetSource>,
    pub dependencies: Vec<Dependency>,
    pub info: Value,
    pub info_plist: Option<Plist>,
    pub entitlements: Value,
    pub entitlements_plist: Option<Plist>,
    pub pre_build_scripts: Vec<BuildScript>,
    pub build_tool_plugins: Vec<BuildToolPlugin>,
    pub post_compile_scripts: Vec<BuildScript>,
    pub post_build_scripts: Vec<BuildScript>,
    pub build_rules: Vec<BuildRule>,
    pub scheme: Value,
    pub target_scheme: Option<TargetScheme>,
    pub legacy: Value,
    pub attributes: Value,
    pub only_copy_files_on_install: bool,
    pub put_resources_before_sources_build_phase: bool,
    pub transitively_link_dependencies: Option<bool>,
    pub directly_embed_carthage_dependencies: Option<bool>,
    pub requires_objc_linking: Option<bool>,
    pub raw: JsonMap,
}

impl Target {
    fn from_entry(name: &str, value: &Value) -> Result<Self, SpecError> {
        let map = value.as_object().cloned().unwrap_or_default();
        let resolved_name = map
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(name)
            .to_owned();
        let product_name = map
            .get("productName")
            .and_then(Value::as_str)
            .unwrap_or(&resolved_name)
            .to_owned();
        let type_string = map.get("type").and_then(Value::as_str).unwrap_or("");
        let target_type = ProductType::parse(type_string)?;
        let mut supported_destinations = parse_string_array(map.get("supportedDestinations"));
        let is_multi_platform = boolish(map.get("isMultiPlatformTarget")).unwrap_or(false);
        if is_multi_platform && !supported_destinations.is_empty() {
            return Err(SpecError::InvalidTargetPlatformAsArray);
        }
        if supported_destinations
            .iter()
            .any(|item| item == "macCatalyst")
            && !supported_destinations.iter().any(|item| item == "iOS")
        {
            supported_destinations.push("iOS".to_owned());
        }
        let platform_string = map
            .get("platform")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(if supported_destinations.is_empty() {
                ""
            } else {
                "auto"
            });
        let platform = Platform::parse(platform_string)?;

        let deployment_target = scalar_to_string(map.get("deploymentTarget"))
            .map(format_deployment_target)
            .transpose()?;
        let deployment_targets = DeploymentTarget::from_value(map.get("deploymentTarget"));

        Ok(Self {
            name: resolved_name,
            product_name,
            target_type,
            platform: platform.clone(),
            supported_destinations,
            deployment_target,
            deployment_targets,
            settings: map.get("settings").cloned().unwrap_or(Value::Null),
            settings_spec: Settings::from_value(map.get("settings")),
            config_files: parse_string_map(map.get("configFiles")).unwrap_or_default(),
            sources: parse_sources(map.get("sources")),
            dependencies: parse_dependencies(map.get("dependencies"), &platform)?,
            info: map.get("info").cloned().unwrap_or(Value::Null),
            info_plist: Plist::from_value(map.get("info")),
            entitlements: map.get("entitlements").cloned().unwrap_or(Value::Null),
            entitlements_plist: Plist::from_value(map.get("entitlements")),
            pre_build_scripts: parse_build_scripts(
                map.get("preBuildScripts")
                    .or_else(|| map.get("prebuildScripts")),
            ),
            build_tool_plugins: parse_build_tool_plugins(map.get("buildToolPlugins"))?,
            post_compile_scripts: parse_build_scripts(map.get("postCompileScripts")),
            post_build_scripts: parse_build_scripts(
                map.get("postBuildScripts")
                    .or_else(|| map.get("postbuildScripts")),
            ),
            build_rules: parse_build_rules(map.get("buildRules")),
            scheme: map.get("scheme").cloned().unwrap_or(Value::Null),
            target_scheme: TargetScheme::from_value(map.get("scheme")),
            legacy: map.get("legacy").cloned().unwrap_or(Value::Null),
            attributes: map
                .get("attributes")
                .cloned()
                .unwrap_or(Value::Object(JsonMap::new())),
            only_copy_files_on_install: boolish(map.get("onlyCopyFilesOnInstall")).unwrap_or(false),
            put_resources_before_sources_build_phase: boolish(
                map.get("putResourcesBeforeSourcesBuildPhase"),
            )
            .unwrap_or(false),
            transitively_link_dependencies: boolish(map.get("transitivelyLinkDependencies")),
            directly_embed_carthage_dependencies: boolish(
                map.get("directlyEmbedCarthageDependencies"),
            ),
            requires_objc_linking: boolish(map.get("requiresObjCLinking")),
            raw: map,
        })
    }

    pub fn filename(&self) -> String {
        let mut filename = self.product_name.clone();
        if let Some(extension) = self.target_type.file_extension() {
            filename.push('.');
            filename.push_str(extension);
        }
        if self.target_type == ProductType::StaticLibrary {
            filename = format!("lib{filename}");
        }
        filename
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetSource {
    pub path: String,
    pub name: Option<String>,
    pub group: Option<String>,
    pub compiler_flags: Vec<String>,
    pub excludes: Vec<String>,
    pub includes: Vec<String>,
    pub explicit_folders: Vec<String>,
    pub source_type: Option<SourceType>,
    pub optional: bool,
    pub build_phase: Option<Value>,
    pub header_visibility: Option<String>,
    pub create_intermediate_groups: Option<bool>,
    pub attributes: Vec<String>,
    pub resource_tags: Vec<String>,
    pub infer_destination_filters_by_path: Option<bool>,
    pub destination_filters: Vec<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SourceType {
    Group,
    File,
    Folder,
    SyncedFolder,
    Other(String),
}

impl SourceType {
    fn parse(value: &str) -> Self {
        match value {
            "group" => Self::Group,
            "file" => Self::File,
            "folder" => Self::Folder,
            "syncedFolder" => Self::SyncedFolder,
            other => Self::Other(other.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dependency {
    pub dependency_type: DependencyType,
    pub reference: String,
    pub embed: Option<bool>,
    pub code_sign: Option<bool>,
    pub remove_headers: bool,
    pub link: Option<bool>,
    pub implicit: bool,
    pub weak_link: bool,
    pub platform_filter: PlatformFilter,
    pub destination_filters: Vec<String>,
    pub platforms: Vec<Platform>,
    pub copy_phase: Option<Value>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DependencyType {
    Target,
    Framework,
    Carthage {
        find_frameworks: Option<bool>,
        link_type: CarthageLinkType,
    },
    Sdk {
        root: Option<String>,
    },
    Package {
        products: Vec<String>,
    },
    Bundle,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CarthageLinkType {
    Dynamic,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformFilter {
    All,
    Ios,
    Macos,
}

impl Dependency {
    fn from_value(value: &Value) -> Result<Self, SpecError> {
        let map = value
            .as_object()
            .ok_or_else(|| SpecError::InvalidDependency(value.to_string()))?;
        let (dependency_type, reference) = if let Some(target) = string_at(map, "target") {
            (DependencyType::Target, target)
        } else if let Some(framework) = string_at(map, "framework") {
            (DependencyType::Framework, framework)
        } else if let Some(carthage) = string_at(map, "carthage") {
            let link_type = match string_at(map, "linkType").as_deref() {
                Some("static") => CarthageLinkType::Static,
                _ => CarthageLinkType::Dynamic,
            };
            (
                DependencyType::Carthage {
                    find_frameworks: boolish(map.get("findFrameworks")),
                    link_type,
                },
                carthage,
            )
        } else if let Some(sdk) = string_at(map, "sdk") {
            (
                DependencyType::Sdk {
                    root: string_at(map, "root"),
                },
                sdk,
            )
        } else if let Some(package) = string_at(map, "package") {
            let products = parse_string_array(map.get("products"))
                .into_iter()
                .chain(string_at(map, "product"))
                .collect();
            (DependencyType::Package { products }, package)
        } else if let Some(bundle) = string_at(map, "bundle") {
            (DependencyType::Bundle, bundle)
        } else {
            return Err(SpecError::InvalidDependency(value.to_string()));
        };

        Ok(Self {
            dependency_type,
            reference,
            embed: boolish(map.get("embed")),
            code_sign: boolish(map.get("codeSign")),
            remove_headers: boolish(map.get("removeHeaders")).unwrap_or(true),
            link: boolish(map.get("link")),
            implicit: boolish(map.get("implicit")).unwrap_or(false),
            weak_link: boolish(map.get("weak")).unwrap_or(false),
            platform_filter: match string_at(map, "platformFilter").as_deref() {
                Some("iOS") => PlatformFilter::Ios,
                Some("macOS") => PlatformFilter::Macos,
                _ => PlatformFilter::All,
            },
            destination_filters: parse_string_array(map.get("destinationFilters")),
            platforms: parse_string_array(map.get("platforms"))
                .into_iter()
                .filter_map(|platform| Platform::parse(&platform).ok())
                .collect(),
            copy_phase: map.get("copy").cloned(),
            raw: value.clone(),
        })
    }

    fn supports_platform(&self, platform: &Platform) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|item| item == platform)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    pub breakpoint_type: BreakpointType,
    pub enabled: bool,
    pub ignore_count: i64,
    pub continue_after_running_actions: bool,
    pub condition: Option<String>,
    pub actions: Vec<BreakpointAction>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointType {
    File {
        path: String,
        line: i64,
        column: Option<i64>,
    },
    Exception {
        scope: BreakpointScope,
        stop_on_style: BreakpointStopOnStyle,
    },
    SwiftError,
    OpenGLError,
    Symbolic {
        symbol: Option<String>,
        module: Option<String>,
    },
    IdeConstraintError,
    IdeTestFailure,
    RuntimeIssue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointScope {
    All,
    ObjectiveC,
    Cpp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointStopOnStyle {
    Throw,
    Catch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointAction {
    DebuggerCommand(Option<String>),
    Log {
        message: Option<String>,
        conveyance_type: BreakpointLogConveyanceType,
    },
    ShellCommand {
        path: Option<String>,
        arguments: Option<String>,
        wait_until_done: bool,
    },
    GraphicsTrace,
    AppleScript(Option<String>),
    Sound(BreakpointSound),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointLogConveyanceType {
    Console,
    Speak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreakpointSound {
    Basso,
    Blow,
    Bottle,
    Frog,
    Funk,
    Glass,
    Hero,
    Morse,
    Ping,
    Pop,
    Purr,
    Sosumi,
    Submarine,
    Tink,
}

impl Breakpoint {
    fn from_value(value: &Value) -> Result<Self, SpecError> {
        let map = value
            .as_object()
            .ok_or_else(|| SpecError::UnknownBreakpoint {
                kind: BreakpointField::Type,
                value: value.to_string(),
            })?;
        let id = string_at(map, "type").ok_or_else(|| SpecError::UnknownBreakpoint {
            kind: BreakpointField::Type,
            value: String::new(),
        })?;
        let breakpoint_type = match id.as_str() {
            "File" | "FileBreakpoint" => BreakpointType::File {
                path: string_at(map, "path").unwrap_or_default(),
                line: map.get("line").and_then(Value::as_i64).unwrap_or_default(),
                column: map.get("column").and_then(Value::as_i64),
            },
            "Exception" | "ExceptionBreakpoint" => BreakpointType::Exception {
                scope: parse_breakpoint_scope(
                    string_at(map, "scope").as_deref().unwrap_or("Objective-C"),
                )?,
                stop_on_style: parse_breakpoint_stop_on_style(
                    string_at(map, "stopOnStyle").as_deref().unwrap_or("throw"),
                )?,
            },
            "SwiftError" | "SwiftErrorBreakpoint" => BreakpointType::SwiftError,
            "OpenGLError" | "OpenGLErrorBreakpoint" => BreakpointType::OpenGLError,
            "Symbolic" | "SymbolicBreakpoint" => BreakpointType::Symbolic {
                symbol: string_at(map, "symbol"),
                module: string_at(map, "module"),
            },
            "IDEConstraintError" | "IDEConstraintErrorBreakpoint" => {
                BreakpointType::IdeConstraintError
            }
            "IDETestFailure" | "IDETestFailureBreakpoint" => BreakpointType::IdeTestFailure,
            "RuntimeIssue" | "RuntimeIssueBreakpoint" => BreakpointType::RuntimeIssue,
            other => {
                return Err(SpecError::UnknownBreakpoint {
                    kind: BreakpointField::Type,
                    value: other.to_owned(),
                })
            }
        };

        Ok(Self {
            breakpoint_type,
            enabled: boolish(map.get("enabled")).unwrap_or(true),
            ignore_count: map.get("ignoreCount").and_then(Value::as_i64).unwrap_or(0),
            continue_after_running_actions: boolish(map.get("continueAfterRunningActions"))
                .unwrap_or(false),
            condition: string_at(map, "condition"),
            actions: parse_breakpoint_actions(map.get("actions"))?,
            raw: value.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildScript {
    pub script: BuildScriptKind,
    pub name: Option<String>,
    pub shell: Option<String>,
    pub input_files: Vec<String>,
    pub output_files: Vec<String>,
    pub input_file_lists: Vec<String>,
    pub output_file_lists: Vec<String>,
    pub run_only_when_installing: bool,
    pub show_env_vars: bool,
    pub based_on_dependency_analysis: bool,
    pub discovered_dependency_file: Option<String>,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildScriptKind {
    Path(String),
    Script(String),
}

impl BuildScript {
    fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        let script = if let Some(script) = string_at(map, "script") {
            BuildScriptKind::Script(script)
        } else {
            BuildScriptKind::Path(string_at(map, "path")?)
        };
        Some(Self {
            script,
            name: string_at(map, "name"),
            shell: string_at(map, "shell"),
            input_files: parse_string_array(map.get("inputFiles")),
            output_files: parse_string_array(map.get("outputFiles")),
            input_file_lists: parse_string_array(map.get("inputFileLists")),
            output_file_lists: parse_string_array(map.get("outputFileLists")),
            run_only_when_installing: boolish(map.get("runOnlyWhenInstalling")).unwrap_or(false),
            show_env_vars: boolish(map.get("showEnvVars")).unwrap_or(true),
            based_on_dependency_analysis: boolish(map.get("basedOnDependencyAnalysis"))
                .unwrap_or(true),
            discovered_dependency_file: string_at(map, "discoveredDependencyFile"),
            raw: value.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildRule {
    pub file_type: BuildRuleFileType,
    pub action: BuildRuleAction,
    pub output_files: Vec<String>,
    pub output_files_compiler_flags: Vec<String>,
    pub name: Option<String>,
    pub run_once_per_architecture: bool,
    pub raw: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildToolPlugin {
    pub plugin: String,
    pub package: String,
    pub raw: Value,
}

impl BuildToolPlugin {
    fn from_value(value: &Value) -> Result<Self, SpecError> {
        let map = value
            .as_object()
            .ok_or_else(|| SpecError::InvalidDependency(value.to_string()))?;
        Ok(Self {
            plugin: string_at(map, "plugin")
                .ok_or_else(|| SpecError::InvalidDependency(value.to_string()))?,
            package: string_at(map, "package")
                .ok_or_else(|| SpecError::InvalidDependency(value.to_string()))?,
            raw: value.clone(),
        })
    }

    pub fn unique_id(&self) -> String {
        format!("{}/{}", self.plugin, self.package)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildRuleFileType {
    Type(String),
    Pattern(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildRuleAction {
    CompilerSpec(String),
    Script(String),
}

impl BuildRule {
    fn from_value(value: &Value) -> Option<Self> {
        let map = value.as_object()?;
        let file_type = if let Some(file_type) = string_at(map, "fileType") {
            BuildRuleFileType::Type(file_type)
        } else {
            BuildRuleFileType::Pattern(string_at(map, "filePattern")?)
        };
        let action = if let Some(compiler_spec) = string_at(map, "compilerSpec") {
            BuildRuleAction::CompilerSpec(compiler_spec)
        } else {
            BuildRuleAction::Script(string_at(map, "script")?)
        };
        Some(Self {
            file_type,
            action,
            output_files: parse_string_array(map.get("outputFiles")),
            output_files_compiler_flags: parse_string_array(map.get("outputFilesCompilerFlags")),
            name: string_at(map, "name"),
            run_once_per_architecture: boolish(map.get("runOncePerArchitecture")).unwrap_or(true),
            raw: value.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProductType {
    Application,
    OnDemandInstallCapableApplication,
    Framework,
    StaticFramework,
    DynamicLibrary,
    StaticLibrary,
    Bundle,
    UnitTestBundle,
    UiTestBundle,
    OcUnitTestBundle,
    AppExtension,
    XcodeExtension,
    IntentsServiceExtension,
    CommandLineTool,
    WatchApp,
    Watch2App,
    WatchExtension,
    Watch2Extension,
    TvExtension,
    MessagesApplication,
    MessagesExtension,
    StickerPack,
    XpcService,
    InstrumentsPackage,
    MetalLibrary,
    SystemExtension,
    ExtensionKitExtension,
    DriverExtension,
    Other(String),
}

impl ProductType {
    fn parse(value: &str) -> Result<Self, SpecError> {
        Ok(match value {
            "application" => Self::Application,
            "application.on-demand-install-capable" => Self::OnDemandInstallCapableApplication,
            "framework" => Self::Framework,
            "staticFramework" => Self::StaticFramework,
            "dynamicLibrary" => Self::DynamicLibrary,
            "staticLibrary" | "library.static" => Self::StaticLibrary,
            "bundle" => Self::Bundle,
            "unit-test" | "unitTestBundle" | "bundle.unit-test" => Self::UnitTestBundle,
            "ui-testing" | "uiTestBundle" | "bundle.ui-testing" => Self::UiTestBundle,
            "ocUnitTestBundle" | "bundle.ocunit-test" => Self::OcUnitTestBundle,
            "app-extension" | "appExtension" => Self::AppExtension,
            "xcodeExtension" | "xcode-extension" => Self::XcodeExtension,
            "intentsServiceExtension" | "intents-service-extension" => {
                Self::IntentsServiceExtension
            }
            "command-line" | "tool" | "commandLineTool" => Self::CommandLineTool,
            "watch-app" | "watchApp" => Self::WatchApp,
            "watch2App" | "watch2-app" | "application.watchapp2" => Self::Watch2App,
            "watch-extension" | "watchExtension" => Self::WatchExtension,
            "watch2Extension" | "watch2-extension" | "watchkit2-extension" => Self::Watch2Extension,
            "tv-extension" | "tvExtension" | "tv-app-extension" => Self::TvExtension,
            "messages-application" | "messagesApplication" | "application.messages" => {
                Self::MessagesApplication
            }
            "messages-extension" | "messagesExtension" | "app-extension.messages" => {
                Self::MessagesExtension
            }
            "sticker-pack" | "stickerPack" | "app-extension.messages-sticker-pack" => {
                Self::StickerPack
            }
            "xpc-service" | "xpcService" => Self::XpcService,
            "instrumentsPackage" | "instruments-package" => Self::InstrumentsPackage,
            "metalLibrary" | "metal-library" => Self::MetalLibrary,
            "system-extension" | "systemExtension" => Self::SystemExtension,
            "extensionkit-extension" | "extensionKitExtension" => Self::ExtensionKitExtension,
            "driver-extension" | "driverExtension" => Self::DriverExtension,
            "" => Self::Other(String::new()),
            other if other.starts_with("com.apple.product-type.") => Self::Other(other.to_owned()),
            other => return Err(SpecError::UnknownTargetType(other.to_owned())),
        })
    }

    pub fn file_extension(&self) -> Option<&'static str> {
        match self {
            Self::Application
            | Self::OnDemandInstallCapableApplication
            | Self::WatchApp
            | Self::Watch2App
            | Self::MessagesApplication => Some("app"),
            Self::Framework | Self::StaticFramework => Some("framework"),
            Self::DynamicLibrary => Some("dylib"),
            Self::StaticLibrary => Some("a"),
            Self::Bundle => Some("bundle"),
            Self::UnitTestBundle | Self::UiTestBundle => Some("xctest"),
            Self::OcUnitTestBundle => Some("octest"),
            Self::AppExtension
            | Self::XcodeExtension
            | Self::IntentsServiceExtension
            | Self::WatchExtension
            | Self::Watch2Extension
            | Self::TvExtension
            | Self::MessagesExtension
            | Self::StickerPack
            | Self::ExtensionKitExtension => Some("appex"),
            Self::XpcService => Some("xpc"),
            Self::InstrumentsPackage => Some("instrpkg"),
            Self::MetalLibrary => Some("metallib"),
            Self::SystemExtension => Some("systemextension"),
            Self::DriverExtension => Some("dext"),
            Self::CommandLineTool | Self::Other(_) => None,
        }
    }

    pub fn is_framework(&self) -> bool {
        matches!(self, Self::Framework | Self::StaticFramework)
    }

    pub fn is_library(&self) -> bool {
        matches!(self, Self::StaticLibrary | Self::DynamicLibrary)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Platform {
    Ios,
    Macos,
    Tvos,
    Watchos,
    Visionos,
    Auto,
}

impl Platform {
    fn parse(value: &str) -> Result<Self, SpecError> {
        match value {
            "iOS" => Ok(Self::Ios),
            "macOS" => Ok(Self::Macos),
            "tvOS" => Ok(Self::Tvos),
            "watchOS" => Ok(Self::Watchos),
            "visionOS" => Ok(Self::Visionos),
            "auto" => Ok(Self::Auto),
            other => Err(SpecError::UnknownTargetPlatform(other.to_owned())),
        }
    }

    pub fn deployment_target_setting(&self) -> &'static str {
        match self {
            Self::Auto => "",
            Self::Ios => "IPHONEOS_DEPLOYMENT_TARGET",
            Self::Macos => "MACOSX_DEPLOYMENT_TARGET",
            Self::Tvos => "TVOS_DEPLOYMENT_TARGET",
            Self::Watchos => "WATCHOS_DEPLOYMENT_TARGET",
            Self::Visionos => "XROS_DEPLOYMENT_TARGET",
        }
    }

    pub fn sdk_root(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Ios => "iphoneos",
            Self::Macos => "macosx",
            Self::Tvos => "appletvos",
            Self::Watchos => "watchos",
            Self::Visionos => "xros",
        }
    }
}

#[derive(Debug)]
pub struct SpecLoader;

impl SpecLoader {
    pub fn load_project(
        path: impl AsRef<Path>,
        project_root: Option<PathBuf>,
        variables: HashMap<String, String>,
    ) -> Result<Project, SpecError> {
        let spec = SpecFile::load_with_options(path, project_root, variables)?;
        let dictionary = spec.resolved_dictionary();
        Project::from_dictionary(spec.base_path.clone(), dictionary)
    }

    pub fn validate_project_dictionary_warnings() -> Result<(), SpecError> {
        Ok(())
    }
}

fn load_dictionary(path: &Path) -> Result<JsonMap, SpecError> {
    let data = fs::read_to_string(path).map_err(|source| SpecError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let value = if path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
    {
        serde_json::from_str::<Value>(&data).map_err(|error| SpecError::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
    } else {
        let yaml = serde_norway::from_str::<serde_norway::Value>(&data).map_err(|error| {
            SpecError::Parse {
                path: path.to_path_buf(),
                message: error.to_string(),
            }
        })?;
        serde_json::to_value(yaml).map_err(|error| SpecError::Parse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
    };
    value
        .as_object()
        .cloned()
        .ok_or_else(|| SpecError::ExpectedMapping(path.to_path_buf()))
}

fn resolve_project_paths(map: &mut JsonMap, relative_to: &Path) {
    prefix_string_or_map_values(map, "configFiles", relative_to);
    prefix_string_or_array_strings(map, "fileGroups", relative_to);
    prefix_object_path(map, "options", "carthageBuildPath", relative_to);

    prefix_nested_objects(
        map,
        "projectReferences",
        relative_to,
        |project_reference, relative_to| {
            prefix_string(project_reference, "path", relative_to);
        },
    );
    prefix_nested_objects(map, "packages", relative_to, |package, relative_to| {
        prefix_string(package, "path", relative_to);
    });
    prefix_nested_objects(map, "targetTemplates", relative_to, resolve_target_paths);
    prefix_nested_objects(map, "targets", relative_to, resolve_target_paths);
    prefix_nested_objects(
        map,
        "aggregateTargets",
        relative_to,
        |aggregate, relative_to| {
            prefix_string_or_map_values(aggregate, "configFiles", relative_to);
            prefix_object_or_array_objects(
                aggregate,
                "buildScripts",
                relative_to,
                resolve_build_script_paths,
            );
        },
    );

    if let Some(Value::Array(schemes)) = map.get_mut("schemes") {
        for scheme in schemes {
            if let Value::Object(scheme) = scheme {
                resolve_scheme_paths(scheme, relative_to);
            }
        }
    } else {
        prefix_nested_objects(map, "schemes", relative_to, resolve_scheme_paths);
    }
    prefix_nested_objects(map, "schemeTemplates", relative_to, resolve_scheme_paths);
}

fn resolve_target_paths(target: &mut JsonMap, relative_to: &Path) {
    prefix_string_or_map_values(target, "configFiles", relative_to);
    prefix_string_or_array_strings(target, "sources", relative_to);
    prefix_object_or_array_objects(target, "sources", relative_to, |source, relative_to| {
        prefix_string(source, "path", relative_to);
    });
    prefix_object_or_array_objects(
        target,
        "dependencies",
        relative_to,
        |dependency, relative_to| {
            prefix_string(dependency, "framework", relative_to);
        },
    );
    prefix_object_path(target, "info", "path", relative_to);
    prefix_object_path(target, "entitlements", "path", relative_to);
    prefix_object_or_array_objects(
        target,
        "preBuildScripts",
        relative_to,
        resolve_build_script_paths,
    );
    prefix_object_or_array_objects(
        target,
        "prebuildScripts",
        relative_to,
        resolve_build_script_paths,
    );
    prefix_object_or_array_objects(
        target,
        "postCompileScripts",
        relative_to,
        resolve_build_script_paths,
    );
    prefix_object_or_array_objects(
        target,
        "postBuildScripts",
        relative_to,
        resolve_build_script_paths,
    );
    prefix_object_or_array_objects(
        target,
        "postbuildScripts",
        relative_to,
        resolve_build_script_paths,
    );
    prefix_object_path(target, "legacy", "workingDirectory", relative_to);
    prefix_object_path(target, "scheme", "storeKitConfiguration", relative_to);
    prefix_object_path(target, "scheme", "commandLineArguments", relative_to);
    if let Some(Value::Object(scheme)) = target.get_mut("scheme") {
        resolve_target_scheme_paths(scheme, relative_to);
    }
}

fn resolve_build_script_paths(script: &mut JsonMap, relative_to: &Path) {
    prefix_string(script, "path", relative_to);
    prefix_string(script, "inputFileLists", relative_to);
    prefix_string_or_array_strings(script, "inputFiles", relative_to);
    prefix_string_or_array_strings(script, "outputFiles", relative_to);
}

fn resolve_target_scheme_paths(scheme: &mut JsonMap, relative_to: &Path) {
    prefix_object_or_array_objects(
        scheme,
        "testPlans",
        relative_to,
        |test_plan, relative_to| {
            prefix_string(test_plan, "path", relative_to);
        },
    );
    prefix_string(scheme, "storeKitConfiguration", relative_to);
}

fn resolve_scheme_paths(scheme: &mut JsonMap, relative_to: &Path) {
    if let Some(Value::Object(run)) = scheme.get_mut("run") {
        prefix_string(run, "storeKitConfiguration", relative_to);
    }
    if let Some(Value::Object(test)) = scheme.get_mut("test") {
        prefix_object_or_array_objects(test, "testPlans", relative_to, |test_plan, relative_to| {
            prefix_string(test_plan, "path", relative_to);
        });
        prefix_string(test, "storeKitConfiguration", relative_to);
    }
}

fn prefix_nested_objects<F>(map: &mut JsonMap, key: &str, relative_to: &Path, mut resolver: F)
where
    F: FnMut(&mut JsonMap, &Path),
{
    if let Some(Value::Object(objects)) = map.get_mut(key) {
        for value in objects.values_mut() {
            if let Value::Object(object) = value {
                resolver(object, relative_to);
            }
        }
    }
}

fn prefix_object_or_array_objects<F>(
    map: &mut JsonMap,
    key: &str,
    relative_to: &Path,
    mut resolver: F,
) where
    F: FnMut(&mut JsonMap, &Path),
{
    match map.get_mut(key) {
        Some(Value::Object(object)) => resolver(object, relative_to),
        Some(Value::Array(items)) => {
            for item in items {
                if let Value::Object(object) = item {
                    resolver(object, relative_to);
                }
            }
        }
        _ => {}
    }
}

fn prefix_object_path(map: &mut JsonMap, object_key: &str, path_key: &str, relative_to: &Path) {
    if let Some(Value::Object(object)) = map.get_mut(object_key) {
        prefix_string(object, path_key, relative_to);
    }
}

fn prefix_string_or_map_values(map: &mut JsonMap, key: &str, relative_to: &Path) {
    match map.get_mut(key) {
        Some(Value::String(_)) => prefix_string(map, key, relative_to),
        Some(Value::Object(values)) => {
            for value in values.values_mut() {
                if let Value::String(string) = value {
                    *string = prefixed_path(relative_to, string);
                }
            }
        }
        _ => {}
    }
}

fn prefix_string_or_array_strings(map: &mut JsonMap, key: &str, relative_to: &Path) {
    match map.get_mut(key) {
        Some(Value::String(_)) => prefix_string(map, key, relative_to),
        Some(Value::Array(values)) => {
            for value in values {
                if let Value::String(string) = value {
                    *string = prefixed_path(relative_to, string);
                }
            }
        }
        _ => {}
    }
}

fn prefix_string(map: &mut JsonMap, key: &str, relative_to: &Path) {
    if let Some(Value::String(string)) = map.get_mut(key) {
        *string = prefixed_path(relative_to, string);
    }
}

fn prefixed_path(relative_to: &Path, value: &str) -> String {
    normalize_path(relative_to.join(value))
        .to_string_lossy()
        .into_owned()
}

fn resolve_project(mut dictionary: JsonMap) -> Result<JsonMap, SpecError> {
    dictionary = resolve_multiplatform_targets(dictionary);
    dictionary = resolve_target_templates(dictionary);
    dictionary = resolve_scheme_templates(dictionary);
    dictionary = resolve_multiplatform_targets(dictionary);
    Ok(dictionary)
}

fn validate_configs_mapping_format(dictionary: &JsonMap) -> Result<(), SpecError> {
    validate_settings_configs(dictionary.get("settings"))?;

    if let Some(Value::Object(setting_groups)) = dictionary.get("settingGroups") {
        for setting_group in setting_groups.values() {
            validate_settings_configs(Some(setting_group))?;
        }
    }

    if let Some(Value::Object(targets)) = dictionary.get("targets") {
        for target in targets.values() {
            if let Some(settings) = target.get("settings") {
                validate_settings_configs(Some(settings))?;
            }
        }
    }

    if let Some(Value::Object(aggregate_targets)) = dictionary.get("aggregateTargets") {
        for target in aggregate_targets.values() {
            if let Some(settings) = target.get("settings") {
                validate_settings_configs(Some(settings))?;
            }
        }
    }

    Ok(())
}

fn validate_settings_configs(settings: Option<&Value>) -> Result<(), SpecError> {
    let Some(Value::Object(settings)) = settings else {
        return Ok(());
    };
    let Some(Value::Object(configs)) = settings.get("configs") else {
        return Ok(());
    };
    let mut invalid_keys = configs
        .iter()
        .filter_map(|(key, value)| (!value.is_object()).then_some(key.clone()))
        .collect::<Vec<_>>();
    if invalid_keys.is_empty() {
        Ok(())
    } else {
        invalid_keys.sort();
        Err(SpecError::InvalidConfigsMappingFormat(invalid_keys))
    }
}

fn validate_package_versions(packages: Option<&Value>) -> Result<(), SpecError> {
    let Some(Value::Object(packages)) = packages else {
        return Ok(());
    };
    for package in packages.values() {
        let Some(package) = package.as_object() else {
            continue;
        };
        for key in [
            "exactVersion",
            "version",
            "from",
            "majorVersion",
            "minorVersion",
            "minVersion",
            "maxVersion",
        ] {
            if let Some(version) = package.get(key).and_then(Value::as_str) {
                validate_semverish(version)?;
            }
        }
    }
    Ok(())
}

fn validate_semverish(version: &str) -> Result<(), SpecError> {
    let (core, prerelease) = version.split_once('-').unwrap_or((version, ""));
    let parts = core.split('.').collect::<Vec<_>>();
    let core_valid = (1..=3).contains(&parts.len())
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    let prerelease_valid = prerelease.is_empty()
        || prerelease
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'));
    if core_valid && prerelease_valid {
        Ok(())
    } else {
        Err(SpecError::InvalidVersion(version.to_owned()))
    }
}

fn resolve_multiplatform_targets(mut dictionary: JsonMap) -> JsonMap {
    let Some(Value::Object(targets)) = dictionary.get("targets") else {
        return dictionary;
    };
    let mut resolved = JsonMap::new();
    for (target_name, target_value) in targets {
        let Some(target) = target_value.as_object() else {
            resolved.insert(target_name.clone(), target_value.clone());
            continue;
        };
        if let Some(Value::Array(platforms)) = target.get("platform") {
            for platform_value in platforms {
                let Some(platform) = platform_value.as_str() else {
                    continue;
                };
                let mut platform_target = target.clone();
                platform_target.insert("isMultiPlatformTarget".to_owned(), Value::Bool(true));
                expand_variables_in_map(
                    &mut platform_target,
                    &HashMap::from([("platform".to_owned(), platform.to_owned())]),
                );
                platform_target.insert("platform".to_owned(), Value::String(platform.to_owned()));
                let suffix = platform_target
                    .get("platformSuffix")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("_{platform}"));
                let prefix = platform_target
                    .get("platformPrefix")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                if let Some(Value::Object(deployment_targets)) = target.get("deploymentTarget") {
                    if let Some(value) = deployment_targets.get(platform) {
                        platform_target.insert("deploymentTarget".to_owned(), value.clone());
                    }
                }
                ensure_product_name_setting(&mut platform_target, target_name);
                platform_target
                    .insert("productName".to_owned(), Value::String(target_name.clone()));
                resolved.insert(
                    format!("{prefix}{target_name}{suffix}"),
                    Value::Object(platform_target),
                );
            }
        } else {
            resolved.insert(target_name.clone(), target_value.clone());
        }
    }
    dictionary.insert("targets".to_owned(), Value::Object(resolved));
    dictionary
}

fn ensure_product_name_setting(target: &mut JsonMap, product_name: &str) {
    let settings = target
        .entry("settings".to_owned())
        .or_insert_with(|| Value::Object(JsonMap::new()));
    if let Value::Object(settings_map) = settings {
        if settings_map.contains_key("configs")
            || settings_map.contains_key("groups")
            || settings_map.contains_key("base")
        {
            let base = settings_map
                .entry("base".to_owned())
                .or_insert_with(|| Value::Object(JsonMap::new()));
            if let Value::Object(base_map) = base {
                base_map
                    .entry("PRODUCT_NAME".to_owned())
                    .or_insert_with(|| Value::String(product_name.to_owned()));
            }
        } else {
            settings_map
                .entry("PRODUCT_NAME".to_owned())
                .or_insert_with(|| Value::String(product_name.to_owned()));
        }
    }
}

fn resolve_target_templates(mut dictionary: JsonMap) -> JsonMap {
    resolve_templates_in_map(&mut dictionary, "targets", "targetTemplates", "target_name");
    dictionary
}

fn resolve_scheme_templates(mut dictionary: JsonMap) -> JsonMap {
    resolve_templates_in_map(&mut dictionary, "schemes", "schemeTemplates", "scheme_name");
    dictionary
}

fn resolve_templates_in_map(
    dictionary: &mut JsonMap,
    base_key: &str,
    templates_key: &str,
    name_to_replace: &str,
) {
    let templates = dictionary
        .get(templates_key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if templates.is_empty() {
        return;
    }

    if let Some(Value::Object(base_items)) = dictionary.get_mut(base_key) {
        for (reference_name, item) in base_items.iter_mut() {
            let Some(item_map) = item.as_object().cloned() else {
                continue;
            };
            let mut template_names = Vec::new();
            let mut insertion_index = 0;
            collect_template_names(
                &item_map,
                &templates,
                &mut template_names,
                &mut insertion_index,
            );
            if template_names.is_empty() {
                continue;
            }

            let mut variables = template_attributes(&item_map);
            let mut merged = JsonMap::new();
            for template_name in &template_names {
                if let Some(Value::Object(template)) = templates.get(template_name) {
                    merged = merge_maps(template.clone(), merged);
                    variables.extend(template_attributes(template));
                }
            }
            let mut resolved = merge_maps(item_map, merged);
            expand_variables_in_map(
                &mut resolved,
                &HashMap::from([(name_to_replace.to_owned(), reference_name.clone())]),
            );
            if !variables.is_empty() {
                expand_variables_in_map(&mut resolved, &variables);
            }
            *item = Value::Object(resolved);
        }
    }
}

fn template_attributes(item: &JsonMap) -> HashMap<String, String> {
    item.get("templateAttributes")
        .and_then(Value::as_object)
        .map(|attributes| {
            attributes
                .iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn collect_template_names(
    item: &JsonMap,
    templates: &JsonMap,
    all_templates: &mut Vec<String>,
    insertion_index: &mut usize,
) {
    let direct_templates =
        parse_string_array(item.get("templates").or_else(|| item.get("template")));
    for template_name in direct_templates {
        if all_templates.contains(&template_name) {
            continue;
        }
        let Some(Value::Object(template)) = templates.get(&template_name) else {
            continue;
        };
        all_templates.insert(*insertion_index, template_name);
        collect_template_names(template, templates, all_templates, insertion_index);
        *insertion_index += 1;
    }
}

fn merge_maps(incoming: JsonMap, mut base: JsonMap) -> JsonMap {
    for (key, value) in incoming {
        if let Some(stripped) = key.strip_suffix(":REPLACE") {
            base.insert(stripped.to_owned(), value);
            continue;
        }
        match (value, base.remove(&key)) {
            (Value::Object(incoming_map), Some(Value::Object(base_map))) => {
                base.insert(key, Value::Object(merge_maps(incoming_map, base_map)));
            }
            (Value::Array(mut incoming_array), Some(Value::Array(mut base_array))) => {
                base_array.append(&mut incoming_array);
                base.insert(key, Value::Array(base_array));
            }
            (incoming_value, _) => {
                base.insert(key, incoming_value);
            }
        }
    }
    base
}

fn expand_variables_in_map(map: &mut JsonMap, variables: &HashMap<String, String>) {
    if variables.is_empty() {
        return;
    }
    if map.keys().any(|key| key.contains("${")) {
        let old = std::mem::take(map);
        for (key, mut value) in old {
            expand_variables_in_value(&mut value, variables);
            map.insert(expand_string(&key, variables), value);
        }
    } else {
        for value in map.values_mut() {
            expand_variables_in_value(value, variables);
        }
    }
}

fn expand_variables_in_value(value: &mut Value, variables: &HashMap<String, String>) {
    match value {
        Value::String(string) => *string = expand_string(string, variables),
        Value::Array(items) => {
            for item in items {
                expand_variables_in_value(item, variables);
            }
        }
        Value::Object(map) => expand_variables_in_map(map, variables),
        _ => {}
    }
}

fn expand_string(string: &str, variables: &HashMap<String, String>) -> String {
    if !string.contains("${") {
        return string.to_owned();
    }
    let mut result = String::with_capacity(string.len());
    let mut rest = string;
    while let Some(start) = rest.find("${") {
        result.push_str(&rest[..start]);
        let after_open = &rest[start + 2..];
        if let Some(end) = after_open.find('}') {
            let key = &after_open[..end];
            if let Some(value) = variables.get(key) {
                result.push_str(value);
                rest = &after_open[end + 1..];
                continue;
            }
        }
        result.push_str("${");
        rest = after_open;
    }
    result.push_str(rest);
    result
}

fn parse_targets(value: Option<&Value>) -> Result<IndexMap<String, Target>, SpecError> {
    let mut targets = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (name, value) in map {
            targets.insert(name.clone(), Target::from_entry(name, value)?);
        }
        targets.sort_keys();
    }
    Ok(targets)
}

fn collect_target_validation_errors(
    project: &Project,
    target: &Target,
    errors: &mut Vec<ValidationError>,
) {
    collect_supported_destinations_validation_errors(target, errors);
    collect_settings_validation_errors(project, &target.settings_spec, errors);
    for (config, config_file) in &target.config_files {
        if !project.configs.contains_key(config) {
            errors.push(ValidationError::InvalidTargetConfigFile {
                target: target.name.clone(),
                config_file: config_file.clone(),
                config: config.clone(),
            });
        }
    }
    if let Some(scheme) = &target.target_scheme {
        for test_target in &scheme.test_targets {
            if !project.targets.contains_key(test_target) {
                errors.push(ValidationError::InvalidTargetSchemeTest {
                    target: target.name.clone(),
                    test_target: test_target.clone(),
                });
            }
        }
        collect_target_scheme_test_plan_errors(project, scheme, errors);
    }

    for source in &target.sources {
        if source.path.is_empty() {
            errors.push(ValidationError::EmptySourcePath {
                target: target.name.clone(),
            });
        } else if !source.optional && !project.path_exists(&source.path) {
            errors.push(ValidationError::InvalidTargetSource {
                target: target.name.clone(),
                source: source.path.clone(),
            });
        }
    }

    let mut seen_dependencies = HashSet::new();
    for dependency in &target.dependencies {
        match &dependency.dependency_type {
            DependencyType::Package { .. } => {
                if !project.package_specs.contains_key(&dependency.reference) {
                    errors.push(ValidationError::InvalidSwiftPackage {
                        name: dependency.reference.clone(),
                        target: target.name.clone(),
                    });
                }
                continue;
            }
            DependencyType::Target
                if !dependency.reference.contains('/')
                    && !project.targets.contains_key(&dependency.reference) =>
            {
                errors.push(ValidationError::InvalidTargetDependency {
                    target: target.name.clone(),
                    dependency: dependency.reference.clone(),
                });
            }
            DependencyType::Target => {
                if let Some(reference) = project_reference_name(&dependency.reference) {
                    if !project.project_references.contains_key(reference) {
                        errors.push(ValidationError::InvalidTargetDependency {
                            target: target.name.clone(),
                            dependency: dependency.reference.clone(),
                        });
                    }
                }
            }
            DependencyType::Sdk { .. } if !is_valid_sdk_dependency(&dependency.reference) => {
                errors.push(ValidationError::InvalidSdkDependency {
                    target: target.name.clone(),
                    dependency: dependency.reference.clone(),
                });
            }
            _ => {}
        }

        let duplicate_key = format!("{:?}:{}", dependency.dependency_type, dependency.reference);
        if !seen_dependencies.insert(duplicate_key) {
            errors.push(ValidationError::DuplicateDependencies {
                target: target.name.clone(),
                dependency_reference: dependency.reference.clone(),
            });
        }
    }

    for plugin in &target.build_tool_plugins {
        if !project.package_specs.contains_key(&plugin.package) {
            errors.push(ValidationError::InvalidPluginPackageReference {
                plugin: plugin.plugin.clone(),
                package: plugin.package.clone(),
            });
        }
    }

    collect_build_script_path_errors(project, &target.name, &target.pre_build_scripts, errors);
    collect_build_script_path_errors(project, &target.name, &target.post_compile_scripts, errors);
    collect_build_script_path_errors(project, &target.name, &target.post_build_scripts, errors);
}

fn collect_aggregate_target_validation_errors(
    project: &Project,
    aggregate: &AggregateTarget,
    errors: &mut Vec<ValidationError>,
) {
    for dependency in &aggregate.targets {
        if !project.targets.contains_key(dependency)
            && !project.aggregate_target_specs.contains_key(dependency)
        {
            errors.push(ValidationError::InvalidTargetDependency {
                target: aggregate.name.clone(),
                dependency: dependency.clone(),
            });
        }
    }
    for (config, config_file) in &aggregate.config_files {
        if !project.configs.contains_key(config) {
            errors.push(ValidationError::InvalidTargetConfigFile {
                target: aggregate.name.clone(),
                config_file: config_file.clone(),
                config: config.clone(),
            });
        }
    }
    if let Some(scheme) = &aggregate.scheme {
        for test_target in &scheme.test_targets {
            if !project.targets.contains_key(test_target) {
                errors.push(ValidationError::InvalidTargetSchemeTest {
                    target: aggregate.name.clone(),
                    test_target: test_target.clone(),
                });
            }
        }
        collect_target_scheme_test_plan_errors(project, scheme, errors);
    }
    collect_build_script_path_errors(project, &aggregate.name, &aggregate.build_scripts, errors);
}

fn collect_scheme_validation_errors(
    project: &Project,
    scheme: &Scheme,
    errors: &mut Vec<ValidationError>,
) {
    for build_target in &scheme.build.targets {
        if !project.targets.contains_key(&build_target.target)
            && !project
                .aggregate_target_specs
                .contains_key(&build_target.target)
            && !build_target.target.contains('/')
        {
            errors.push(ValidationError::InvalidSchemeTarget {
                scheme: scheme.name.clone(),
                target: build_target.target.clone(),
                action: "build".to_owned(),
            });
        }
        if let Some(reference) = project_reference_name(&build_target.target) {
            if !project.project_references.contains_key(reference) {
                errors.push(ValidationError::InvalidProjectReference {
                    scheme: scheme.name.clone(),
                    reference: reference.to_owned(),
                });
            }
        }
    }
    if let Some(run) = &scheme.run {
        collect_scheme_config_validation_error(project, scheme, run.config.as_deref(), errors);
    }
    if let Some(test) = &scheme.test {
        collect_scheme_config_validation_error(project, scheme, test.config.as_deref(), errors);
        for test_target in &test.targets {
            if !project.targets.contains_key(&test_target.target_reference)
                && !test_target.target_reference.contains('/')
            {
                errors.push(ValidationError::InvalidSchemeTarget {
                    scheme: scheme.name.clone(),
                    target: test_target.target_reference.clone(),
                    action: "test".to_owned(),
                });
            }
            if let Some(reference) = project_reference_name(&test_target.target_reference) {
                if !project.project_references.contains_key(reference) {
                    errors.push(ValidationError::InvalidProjectReference {
                        scheme: scheme.name.clone(),
                        reference: reference.to_owned(),
                    });
                }
            }
        }
        for coverage_target in &test.coverage_targets {
            if let Some(reference) = project_reference_name(coverage_target) {
                if !project.project_references.contains_key(reference) {
                    errors.push(ValidationError::InvalidProjectReference {
                        scheme: scheme.name.clone(),
                        reference: reference.to_owned(),
                    });
                }
            }
        }
        if test
            .test_plans
            .iter()
            .filter(|test_plan| test_plan.default_plan)
            .count()
            > 1
        {
            errors.push(ValidationError::MultipleDefaultTestPlans);
        }
        for test_plan in &test.test_plans {
            if !project.path_exists(&test_plan.path) {
                errors.push(ValidationError::InvalidTestPlan(test_plan.clone()));
            }
        }
    }
    if let Some(profile) = &scheme.profile {
        collect_scheme_config_validation_error(project, scheme, profile.config.as_deref(), errors);
    }
    if let Some(analyze) = &scheme.analyze {
        collect_scheme_config_validation_error(project, scheme, analyze.config.as_deref(), errors);
    }
    if let Some(archive) = &scheme.archive {
        collect_scheme_config_validation_error(project, scheme, archive.config.as_deref(), errors);
    }
}

fn collect_scheme_config_validation_error(
    project: &Project,
    scheme: &Scheme,
    config: Option<&str>,
    errors: &mut Vec<ValidationError>,
) {
    if let Some(config) = config {
        if !project.configs.contains_key(config) {
            errors.push(ValidationError::InvalidSchemeConfig {
                scheme: scheme.name.clone(),
                config: config.to_owned(),
            });
        }
    }
}

fn project_reference_name(reference: &str) -> Option<&str> {
    reference
        .split_once('/')
        .map(|(project_reference, _)| project_reference)
        .filter(|project_reference| !project_reference.is_empty())
}

fn collect_build_script_path_errors(
    project: &Project,
    target: &str,
    scripts: &[BuildScript],
    errors: &mut Vec<ValidationError>,
) {
    for script in scripts {
        if let BuildScriptKind::Path(path) = &script.script {
            if !project.path_exists(path) {
                errors.push(ValidationError::InvalidBuildScriptPath {
                    target: target.to_owned(),
                    name: script.name.clone(),
                    path: path.clone(),
                });
            }
        }
    }
}

fn collect_target_scheme_test_plan_errors(
    project: &Project,
    scheme: &TargetScheme,
    errors: &mut Vec<ValidationError>,
) {
    if scheme
        .test_plans
        .iter()
        .filter(|test_plan| test_plan.default_plan)
        .count()
        > 1
    {
        errors.push(ValidationError::MultipleDefaultTestPlans);
    }
    for test_plan in &scheme.test_plans {
        if !project.path_exists(&test_plan.path) {
            errors.push(ValidationError::InvalidTestPlan(test_plan.clone()));
        }
    }
}

fn collect_settings_validation_errors(
    project: &Project,
    settings: &Settings,
    errors: &mut Vec<ValidationError>,
) {
    for config in settings.config_settings.keys() {
        if !project.configs.contains_key(config) {
            errors.push(ValidationError::InvalidBuildSettingConfig(config.clone()));
        }
    }
    for group in &settings.groups {
        if !project.setting_group_specs.contains_key(group) {
            errors.push(ValidationError::InvalidSettingsGroup(group.clone()));
        }
    }
    for config_settings in settings.config_settings.values() {
        collect_settings_validation_errors(project, config_settings, errors);
    }
}

fn collect_per_config_settings_validation_errors(
    project: &Project,
    settings: &Settings,
    errors: &mut Vec<ValidationError>,
) {
    if settings
        .build_settings
        .keys()
        .any(|key| project.configs.contains_key(key))
    {
        errors.push(ValidationError::InvalidPerConfigSettings);
    }
    for config_settings in settings.config_settings.values() {
        collect_per_config_settings_validation_errors(project, config_settings, errors);
    }
}

fn collect_supported_destinations_validation_errors(
    target: &Target,
    errors: &mut Vec<ValidationError>,
) {
    if target.supported_destinations.is_empty() {
        return;
    }
    if target.platform == Platform::Watchos
        && target
            .supported_destinations
            .iter()
            .any(|destination| destination != "watchOS")
    {
        errors.push(
            ValidationError::UnexpectedTargetPlatformForSupportedDestinations {
                target: target.name.clone(),
                platform: target.platform.clone(),
            },
        );
    }
    if target.platform == Platform::Auto
        && matches!(target.target_type, ProductType::Application)
        && target
            .supported_destinations
            .iter()
            .any(|destination| destination == "watchOS")
    {
        errors.push(
            ValidationError::ContainsWatchOSDestinationForMultiplatformApp {
                target: target.name.clone(),
            },
        );
    }
    if target
        .supported_destinations
        .iter()
        .any(|destination| destination == "macOS")
        && target
            .supported_destinations
            .iter()
            .any(|destination| destination == "macCatalyst")
    {
        errors.push(
            ValidationError::MultipleMacPlatformsInSupportedDestinations {
                target: target.name.clone(),
            },
        );
    }
    if target
        .supported_destinations
        .iter()
        .any(|destination| destination == "macCatalyst")
        && !matches!(target.platform, Platform::Ios | Platform::Auto)
    {
        errors.push(
            ValidationError::InvalidTargetPlatformForSupportedDestinations {
                target: target.name.clone(),
            },
        );
    }
    if let Some(destination) = platform_destination(&target.platform) {
        if !target
            .supported_destinations
            .iter()
            .any(|supported| supported == destination)
        {
            errors.push(
                ValidationError::MissingTargetPlatformInSupportedDestinations {
                    target: target.name.clone(),
                    platform: target.platform.clone(),
                },
            );
        }
    }
}

fn platform_destination(platform: &Platform) -> Option<&'static str> {
    match platform {
        Platform::Ios => Some("iOS"),
        Platform::Macos => Some("macOS"),
        Platform::Tvos => Some("tvOS"),
        Platform::Watchos => Some("watchOS"),
        Platform::Visionos => Some("visionOS"),
        Platform::Auto => None,
    }
}

fn is_valid_sdk_dependency(reference: &str) -> bool {
    reference.ends_with(".framework")
        || reference.ends_with(".tbd")
        || reference.ends_with(".dylib")
        || reference.ends_with(".xcframework")
}

fn parse_sources(value: Option<&Value>) -> Vec<TargetSource> {
    match value {
        Some(Value::String(path)) => vec![TargetSource {
            path: path.clone(),
            name: None,
            group: None,
            compiler_flags: Vec::new(),
            excludes: Vec::new(),
            includes: Vec::new(),
            explicit_folders: Vec::new(),
            source_type: None,
            optional: false,
            build_phase: None,
            header_visibility: None,
            create_intermediate_groups: None,
            attributes: Vec::new(),
            resource_tags: Vec::new(),
            infer_destination_filters_by_path: None,
            destination_filters: Vec::new(),
            raw: Value::String(path.clone()),
        }],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(path) => Some(TargetSource {
                    path: path.clone(),
                    name: None,
                    group: None,
                    compiler_flags: Vec::new(),
                    excludes: Vec::new(),
                    includes: Vec::new(),
                    explicit_folders: Vec::new(),
                    source_type: None,
                    optional: false,
                    build_phase: None,
                    header_visibility: None,
                    create_intermediate_groups: None,
                    attributes: Vec::new(),
                    resource_tags: Vec::new(),
                    infer_destination_filters_by_path: None,
                    destination_filters: Vec::new(),
                    raw: item.clone(),
                }),
                Value::Object(map) => {
                    map.get("path")
                        .and_then(Value::as_str)
                        .map(|path| TargetSource {
                            path: path.to_owned(),
                            name: string_at(map, "name"),
                            group: string_at(map, "group"),
                            compiler_flags: parse_compiler_flags(map.get("compilerFlags")),
                            excludes: parse_string_array(map.get("excludes")),
                            includes: parse_string_array(map.get("includes")),
                            explicit_folders: parse_string_array(map.get("explicitFolders")),
                            source_type: string_at(map, "type")
                                .map(|value| SourceType::parse(&value)),
                            optional: boolish(map.get("optional")).unwrap_or(false),
                            build_phase: map.get("buildPhase").cloned(),
                            header_visibility: string_at(map, "headerVisibility"),
                            create_intermediate_groups: boolish(
                                map.get("createIntermediateGroups"),
                            ),
                            attributes: parse_string_array(map.get("attributes")),
                            resource_tags: parse_string_array(map.get("resourceTags")),
                            infer_destination_filters_by_path: boolish(
                                map.get("inferDestinationFiltersByPath"),
                            ),
                            destination_filters: parse_string_array(map.get("destinationFilters")),
                            raw: item.clone(),
                        })
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_dependencies(
    value: Option<&Value>,
    platform: &Platform,
) -> Result<Vec<Dependency>, SpecError> {
    let dependencies = parse_value_array(value)
        .iter()
        .map(Dependency::from_value)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(dependencies
        .into_iter()
        .filter(|dependency| dependency.supports_platform(platform))
        .collect())
}

fn parse_breakpoints(value: Option<&Value>) -> Result<Vec<Breakpoint>, SpecError> {
    parse_value_array(value)
        .iter()
        .map(Breakpoint::from_value)
        .collect()
}

fn parse_aggregate_targets(value: Option<&Value>) -> IndexMap<String, AggregateTarget> {
    let mut targets = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (name, value) in map {
            let target_map = value.as_object().cloned().unwrap_or_default();
            targets.insert(
                name.clone(),
                AggregateTarget {
                    name: name.clone(),
                    targets: parse_string_array(target_map.get("targets")),
                    settings: target_map.get("settings").cloned().unwrap_or(Value::Null),
                    settings_spec: Settings::from_value(target_map.get("settings")),
                    config_files: parse_string_map(target_map.get("configFiles"))
                        .unwrap_or_default(),
                    build_scripts: parse_build_scripts(target_map.get("buildScripts")),
                    build_tool_plugins: parse_build_tool_plugins(
                        target_map.get("buildToolPlugins"),
                    )
                    .unwrap_or_default(),
                    scheme: TargetScheme::from_value(target_map.get("scheme")),
                    raw: value.clone(),
                },
            );
        }
    }
    targets
}

fn parse_setting_groups(value: Option<&Value>) -> IndexMap<String, Settings> {
    let mut groups = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (name, value) in map {
            groups.insert(name.clone(), Settings::from_value(Some(value)));
        }
    }
    groups
}

fn parse_package_value_map(value: &IndexMap<String, Value>) -> IndexMap<String, SwiftPackage> {
    let mut packages = IndexMap::new();
    for (name, value) in value {
        let Some(package) = value.as_object() else {
            continue;
        };
        let parsed = if let Some(path) = string_at(package, "path") {
            SwiftPackage::Local {
                path,
                group: string_at(package, "group"),
                exclude_from_project: boolish(package.get("excludeFromProject")).unwrap_or(false),
            }
        } else {
            let url = string_at(package, "url").or_else(|| {
                string_at(package, "github").map(|repo| format!("https://github.com/{repo}"))
            });
            let Some(url) = url else {
                continue;
            };
            SwiftPackage::Remote {
                url,
                version_requirement: parse_package_requirement(package),
            }
        };
        packages.insert(name.clone(), parsed);
    }
    packages
}

fn parse_package_requirement(map: &JsonMap) -> PackageVersionRequirement {
    if let Some(value) = string_at(map, "exactVersion").or_else(|| string_at(map, "version")) {
        PackageVersionRequirement::Exact(value)
    } else if let Some(value) = string_at(map, "majorVersion").or_else(|| string_at(map, "from")) {
        PackageVersionRequirement::UpToNextMajorVersion(value)
    } else if let Some(value) = string_at(map, "minorVersion") {
        PackageVersionRequirement::UpToNextMinorVersion(value)
    } else if let Some(value) = string_at(map, "branch") {
        PackageVersionRequirement::Branch(value)
    } else if let Some(value) = string_at(map, "revision") {
        PackageVersionRequirement::Revision(value)
    } else if let (Some(from), Some(to)) =
        (string_at(map, "minVersion"), string_at(map, "maxVersion"))
    {
        PackageVersionRequirement::Range { from, to }
    } else {
        PackageVersionRequirement::UpToNextMajorVersion("0.0.0".to_owned())
    }
}

fn parse_file_types(value: Option<&Value>) -> IndexMap<String, FileType> {
    let mut file_types = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (extension, value) in map {
            let Some(file_type) = value.as_object() else {
                continue;
            };
            file_types.insert(
                extension.clone(),
                FileType {
                    file: boolish(file_type.get("file")).unwrap_or(true),
                    build_phase: string_at(file_type, "buildPhase")
                        .map(|value| FileBuildPhase::parse(&value)),
                    attributes: parse_string_array(file_type.get("attributes")),
                    resource_tags: parse_string_array(file_type.get("resourceTags")),
                    compiler_flags: parse_compiler_flags(file_type.get("compilerFlags")),
                },
            );
        }
    }
    file_types
}

fn parse_group_ordering(value: Option<&Value>) -> Vec<GroupOrdering> {
    let Some(Value::Array(items)) = value else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let map = item.as_object()?;
            Some(GroupOrdering {
                pattern: string_at(map, "pattern"),
                order: parse_string_array(map.get("order")),
            })
        })
        .collect()
}

fn parse_schemes(value: Option<&Value>) -> IndexMap<String, Scheme> {
    let mut schemes = IndexMap::new();
    match value {
        Some(Value::Object(map)) => {
            for (name, value) in map {
                schemes.insert(name.clone(), Scheme::from_entry(name, value));
            }
        }
        Some(Value::Array(items)) => {
            for value in items {
                if let Some(map) = value.as_object() {
                    if let Some(name) = string_at(map, "name") {
                        schemes.insert(name.clone(), Scheme::from_entry(&name, value));
                    }
                }
            }
        }
        _ => {}
    }
    schemes
}

fn parse_build_targets(value: Option<&Value>) -> Vec<SchemeBuildTarget> {
    let mut targets = Vec::new();
    if let Some(Value::Object(map)) = value {
        for (target, build_types) in map {
            targets.push(SchemeBuildTarget {
                target: target.clone(),
                build_types: parse_build_types(build_types),
            });
        }
    }
    targets
}

fn parse_build_types(value: &Value) -> Vec<BuildType> {
    match value {
        Value::String(value) => build_types_for_name(value),
        Value::Array(values) => dedupe_build_types(
            values
                .iter()
                .flat_map(|value| value.as_str().map(build_types_for_name).unwrap_or_default())
                .collect(),
        ),
        Value::Object(map) => dedupe_build_types(
            map.iter()
                .filter(|(_, value)| boolish(Some(value)).unwrap_or(false))
                .flat_map(|(key, _)| build_types_for_name(key))
                .collect(),
        ),
        _ => Vec::new(),
    }
}

fn build_types_for_name(value: &str) -> Vec<BuildType> {
    match value {
        "all" => vec![
            BuildType::Running,
            BuildType::Testing,
            BuildType::Profiling,
            BuildType::Analyzing,
            BuildType::Archiving,
        ],
        "test" | "testing" => vec![BuildType::Testing, BuildType::Analyzing],
        "run" | "running" => vec![BuildType::Running],
        "profile" | "profiling" => vec![BuildType::Profiling],
        "analyze" | "analyzing" => vec![BuildType::Analyzing],
        "archive" | "archiving" => vec![BuildType::Archiving],
        "none" => Vec::new(),
        _ => Vec::new(),
    }
}

fn dedupe_build_types(types: Vec<BuildType>) -> Vec<BuildType> {
    let mut deduped = Vec::new();
    for build_type in types {
        if !deduped.contains(&build_type) {
            deduped.push(build_type);
        }
    }
    deduped
}

fn parse_test_targets(value: Option<&Value>) -> Vec<SchemeTestTarget> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(target) => Some(SchemeTestTarget {
                    target_reference: target.clone(),
                    random_execution_order: false,
                    parallelizable: false,
                    location: None,
                    skipped: false,
                    skipped_tests: Vec::new(),
                    selected_tests: Vec::new(),
                }),
                Value::Object(map) => string_at(map, "name")
                    .or_else(|| string_at(map, "target"))
                    .map(|name| SchemeTestTarget {
                        target_reference: name,
                        random_execution_order: boolish(map.get("randomExecutionOrder"))
                            .unwrap_or(false),
                        parallelizable: boolish(map.get("parallelizable")).unwrap_or(false),
                        location: string_at(map, "location"),
                        skipped: boolish(map.get("skipped")).unwrap_or(false),
                        skipped_tests: parse_string_array(map.get("skippedTests")),
                        selected_tests: parse_string_array(map.get("selectedTests")),
                    }),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_target_names(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(target) => Some(target.clone()),
                Value::Object(map) => string_at(map, "name"),
                _ => None,
            })
            .collect(),
        _ => parse_string_array(value),
    }
}

fn parse_test_plans(value: Option<&Value>) -> Vec<TestPlan> {
    parse_value_array(value)
        .iter()
        .filter_map(|value| {
            let map = value.as_object()?;
            Some(TestPlan {
                path: string_at(map, "path")?,
                default_plan: boolish(map.get("default"))
                    .or_else(|| boolish(map.get("defaultPlan")))
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn parse_scheme_actions(value: Option<&Value>) -> Vec<SchemeAction> {
    parse_value_array(value)
        .iter()
        .filter_map(|value| {
            let map = value.as_object()?;
            Some(SchemeAction {
                name: string_at(map, "name").unwrap_or_else(|| "Run Script".to_owned()),
                script: string_at(map, "script")?,
                settings_target: string_at(map, "settingsTarget"),
            })
        })
        .collect()
}

fn parse_environment_variables(value: Option<&Value>) -> Vec<EnvironmentVariable> {
    match value {
        Some(Value::Object(map)) => map
            .iter()
            .filter_map(|(key, value)| {
                Some(EnvironmentVariable {
                    variable: key.clone(),
                    value: environment_value(value)?,
                    enabled: true,
                })
            })
            .collect(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|value| {
                let map = value.as_object()?;
                Some(EnvironmentVariable {
                    variable: string_at(map, "variable")?,
                    value: environment_value(map.get("value")?)?,
                    enabled: boolish(map.get("isEnabled")).unwrap_or(true),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn environment_value(value: &Value) -> Option<String> {
    match value {
        Value::Bool(true) => Some("YES".to_owned()),
        Value::Bool(false) => Some("NO".to_owned()),
        _ => scalar_to_string(Some(value)),
    }
}

fn parse_bool_map(value: Option<&Value>) -> IndexMap<String, bool> {
    let mut parsed = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (key, value) in map {
            if let Some(value) = boolish(Some(value)) {
                parsed.insert(key.clone(), value);
            }
        }
    }
    parsed
}

fn parse_build_scripts(value: Option<&Value>) -> Vec<BuildScript> {
    parse_value_array(value)
        .iter()
        .filter_map(BuildScript::from_value)
        .collect()
}

fn parse_build_rules(value: Option<&Value>) -> Vec<BuildRule> {
    parse_value_array(value)
        .iter()
        .filter_map(BuildRule::from_value)
        .collect()
}

fn parse_build_tool_plugins(value: Option<&Value>) -> Result<Vec<BuildToolPlugin>, SpecError> {
    parse_value_array(value)
        .iter()
        .map(BuildToolPlugin::from_value)
        .collect()
}

fn parse_breakpoint_actions(value: Option<&Value>) -> Result<Vec<BreakpointAction>, SpecError> {
    parse_value_array(value)
        .iter()
        .map(|value| {
            let map = value
                .as_object()
                .ok_or_else(|| SpecError::UnknownBreakpoint {
                    kind: BreakpointField::ActionType,
                    value: value.to_string(),
                })?;
            let id = string_at(map, "type").ok_or_else(|| SpecError::UnknownBreakpoint {
                kind: BreakpointField::ActionType,
                value: String::new(),
            })?;
            Ok(match id.as_str() {
                "DebuggerCommand" => BreakpointAction::DebuggerCommand(string_at(map, "command")),
                "Log" => BreakpointAction::Log {
                    message: string_at(map, "message"),
                    conveyance_type: parse_breakpoint_conveyance(
                        string_at(map, "conveyanceType")
                            .as_deref()
                            .unwrap_or("console"),
                    )?,
                },
                "ShellCommand" => BreakpointAction::ShellCommand {
                    path: string_at(map, "path"),
                    arguments: string_at(map, "arguments"),
                    wait_until_done: boolish(map.get("waitUntilDone")).unwrap_or(false),
                },
                "GraphicsTrace" => BreakpointAction::GraphicsTrace,
                "AppleScript" => BreakpointAction::AppleScript(string_at(map, "script")),
                "Sound" => BreakpointAction::Sound(parse_breakpoint_sound(
                    string_at(map, "sound").as_deref().unwrap_or("Basso"),
                )?),
                other => {
                    return Err(SpecError::UnknownBreakpoint {
                        kind: BreakpointField::ActionType,
                        value: other.to_owned(),
                    })
                }
            })
        })
        .collect()
}

fn parse_breakpoint_scope(value: &str) -> Result<BreakpointScope, SpecError> {
    match value.to_lowercase().as_str() {
        "all" => Ok(BreakpointScope::All),
        "objective-c" => Ok(BreakpointScope::ObjectiveC),
        "c++" => Ok(BreakpointScope::Cpp),
        other => Err(SpecError::UnknownBreakpoint {
            kind: BreakpointField::Scope,
            value: other.to_owned(),
        }),
    }
}

fn parse_breakpoint_stop_on_style(value: &str) -> Result<BreakpointStopOnStyle, SpecError> {
    match value.to_lowercase().as_str() {
        "throw" => Ok(BreakpointStopOnStyle::Throw),
        "catch" => Ok(BreakpointStopOnStyle::Catch),
        other => Err(SpecError::UnknownBreakpoint {
            kind: BreakpointField::StopOnStyle,
            value: other.to_owned(),
        }),
    }
}

fn parse_breakpoint_conveyance(value: &str) -> Result<BreakpointLogConveyanceType, SpecError> {
    match value.to_lowercase().as_str() {
        "console" => Ok(BreakpointLogConveyanceType::Console),
        "speak" => Ok(BreakpointLogConveyanceType::Speak),
        other => Err(SpecError::UnknownBreakpoint {
            kind: BreakpointField::ActionConveyanceType,
            value: other.to_owned(),
        }),
    }
}

fn parse_breakpoint_sound(value: &str) -> Result<BreakpointSound, SpecError> {
    match value {
        "Basso" => Ok(BreakpointSound::Basso),
        "Blow" => Ok(BreakpointSound::Blow),
        "Bottle" => Ok(BreakpointSound::Bottle),
        "Frog" => Ok(BreakpointSound::Frog),
        "Funk" => Ok(BreakpointSound::Funk),
        "Glass" => Ok(BreakpointSound::Glass),
        "Hero" => Ok(BreakpointSound::Hero),
        "Morse" => Ok(BreakpointSound::Morse),
        "Ping" => Ok(BreakpointSound::Ping),
        "Pop" => Ok(BreakpointSound::Pop),
        "Purr" => Ok(BreakpointSound::Purr),
        "Sosumi" => Ok(BreakpointSound::Sosumi),
        "Submarine" => Ok(BreakpointSound::Submarine),
        "Tink" => Ok(BreakpointSound::Tink),
        other => Err(SpecError::UnknownBreakpoint {
            kind: BreakpointField::ActionSoundName,
            value: other.to_owned(),
        }),
    }
}

fn parse_compiler_flags(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(flags)) => flags.split_whitespace().map(str::to_owned).collect(),
        Some(Value::Array(_)) => parse_string_array(value),
        _ => Vec::new(),
    }
}

fn parse_value_array(value: Option<&Value>) -> Vec<Value> {
    value.and_then(Value::as_array).cloned().unwrap_or_default()
}

fn parse_string_array(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(string)) => vec![string.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_string_map(value: Option<&Value>) -> Option<IndexMap<String, String>> {
    let Value::Object(map) = value? else {
        return None;
    };
    let mut parsed = IndexMap::new();
    for (key, value) in map {
        if let Some(value) = scalar_to_string(Some(value)) {
            parsed.insert(key.clone(), value);
        }
    }
    Some(parsed)
}

fn parse_value_map(value: Option<&Value>) -> IndexMap<String, Value> {
    let mut parsed = IndexMap::new();
    if let Some(Value::Object(map)) = value {
        for (key, value) in map {
            parsed.insert(key.clone(), value.clone());
        }
    }
    parsed
}

fn default_configs() -> IndexMap<String, String> {
    IndexMap::from([
        ("Debug".to_owned(), "debug".to_owned()),
        ("Release".to_owned(), "release".to_owned()),
    ])
}

fn scalar_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn string_at(map: &JsonMap, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn boolish(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.as_str() {
            "true" | "TRUE" | "YES" | "yes" | "1" => Some(true),
            "false" | "FALSE" | "NO" | "no" | "0" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn compare_versions(left: &str, right: &str) -> Option<std::cmp::Ordering> {
    let left = parse_version_parts(left)?;
    let right = parse_version_parts(right)?;
    Some(left.cmp(&right))
}

fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let mut parts = version
        .split('.')
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    while parts.len() < 3 {
        parts.push(0);
    }
    Some(parts)
}

fn path_has_wildcards(path: &str) -> bool {
    path.contains('*') || path.contains('?') || path.contains('{') || path.contains('[')
}

fn absolute_path(path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    if path.is_absolute() {
        normalize_path(path)
    } else {
        normalize_path(
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(path),
        )
    }
}

fn normalize_path(path: impl AsRef<Path>) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.as_ref().components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn pathdiff(path: &Path, base: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_includes_additively_and_supports_replace() {
        let mut base = JsonMap::new();
        base.insert("items".to_owned(), serde_json::json!(["a"]));
        base.insert(
            "settings".to_owned(),
            serde_json::json!({"one": 1, "two": 2}),
        );
        let mut incoming = JsonMap::new();
        incoming.insert("items".to_owned(), serde_json::json!(["b"]));
        incoming.insert(
            "settings:REPLACE".to_owned(),
            serde_json::json!({"three": 3}),
        );
        let merged = merge_maps(incoming, base);
        assert_eq!(merged["items"], serde_json::json!(["a", "b"]));
        assert_eq!(merged["settings"], serde_json::json!({"three": 3}));
    }

    #[test]
    fn expands_multiplatform_targets() {
        let dictionary = serde_json::json!({
            "name": "App",
            "targets": {
                "Shared": {
                    "type": "application",
                    "platform": ["iOS", "macOS"],
                    "sources": ["Sources/${platform}"]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        assert!(project.targets.contains_key("Shared_iOS"));
        assert!(project.targets.contains_key("Shared_macOS"));
        assert_eq!(project.targets["Shared_iOS"].sources[0].path, "Sources/iOS");
    }

    #[test]
    fn parses_nested_target_templates_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Templates",
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "deploymentTarget": "1.2.0",
                    "sources": ["target"],
                    "templates": ["temp"],
                    "templateAttributes": {
                        "a": "a-by-target",
                        "b": "b-by-target",
                        "d": "d-by-target",
                        "temp": "temp-by-target"
                    }
                }
            },
            "targetTemplates": {
                "temp": {
                    "platform": "tvOS",
                    "sources": ["temp", "${temp}"],
                    "templates": ["a", "d"],
                    "templateAttributes": {
                        "c": "c-by-temp",
                        "d": "d-by-temp"
                    }
                },
                "a": {
                    "templates": ["b", "c"],
                    "sources": ["a", "${a}"],
                    "templateAttributes": {"c": "c-by-a"}
                },
                "b": {"sources": ["b", "${b}"]},
                "c": {"sources": ["c", "${c}"]},
                "d": {
                    "sources": ["d", "${d}"],
                    "templates": ["e"],
                    "templateAttributes": {"e": "e-by-d"}
                },
                "e": {"sources": ["e", "${e}"]}
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let target = &project.targets["Framework"];
        assert_eq!(target.target_type, ProductType::Framework);
        assert_eq!(target.platform, Platform::Ios);
        assert_eq!(target.deployment_target.as_deref(), Some("1.2"));
        assert_eq!(
            target
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "b",
                "b-by-target",
                "c",
                "c-by-temp",
                "a",
                "a-by-target",
                "e",
                "e-by-d",
                "d",
                "d-by-temp",
                "temp",
                "temp-by-target",
                "target"
            ]
        );
    }

    #[test]
    fn parses_nested_target_templates_with_cycle_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "TemplateCycle",
            "targets": {
                "Framework": {
                    "deploymentTarget": "1.2.0",
                    "sources": ["targetSource"],
                    "templates": ["temp2"]
                }
            },
            "targetTemplates": {
                "temp": {
                    "type": "framework",
                    "platform": "iOS",
                    "templates": ["temp1"],
                    "sources": ["nestedTemplateSource1"]
                },
                "temp1": {
                    "platform": "macOS",
                    "templates": ["temp2"],
                    "sources": ["nestedTemplateSource2"]
                },
                "temp2": {
                    "platform": "tvOS",
                    "deploymentTarget": "1.1.0",
                    "configFiles": {"debug": "Configs/${target_name}/debug.xcconfig"},
                    "templates": ["temp", "temp1"],
                    "sources": ["templateSource"]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let target = &project.targets["Framework"];
        assert_eq!(target.target_type, ProductType::Framework);
        assert_eq!(target.platform, Platform::Tvos);
        assert_eq!(target.deployment_target.as_deref(), Some("1.2"));
        assert_eq!(
            target
                .sources
                .iter()
                .map(|source| source.path.as_str())
                .collect::<Vec<_>>(),
            vec![
                "nestedTemplateSource2",
                "nestedTemplateSource1",
                "templateSource",
                "targetSource"
            ]
        );
        assert_eq!(
            target.config_files.get("debug").map(String::as_str),
            Some("Configs/Framework/debug.xcconfig")
        );
    }

    #[test]
    fn parses_sources_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "App",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "sourceString",
                        {"path": "sourceObject"},
                        {"path": "sourceWithFlagsArray", "compilerFlags": ["-Werror"]},
                        {"path": "sourceWithFlagsString", "compilerFlags": "-Werror -Wextra"},
                        {"path": "sourceWithExcludes", "excludes": ["Foo.swift"]},
                        {"path": "sourceWithFileType", "type": "file"},
                        {"path": "sourceWithFolderType", "type": "folder"},
                        {"path": "sourceWithResourceTags", "resourceTags": ["tag1", "tag2"]}
                    ]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let sources = &project.targets["App"].sources;
        assert_eq!(sources[0].path, "sourceString");
        assert_eq!(sources[2].compiler_flags, vec!["-Werror"]);
        assert_eq!(sources[3].compiler_flags, vec!["-Werror", "-Wextra"]);
        assert_eq!(sources[4].excludes, vec!["Foo.swift"]);
        assert_eq!(sources[5].source_type, Some(SourceType::File));
        assert_eq!(sources[6].source_type, Some(SourceType::Folder));
        assert_eq!(sources[7].resource_tags, vec!["tag1", "tag2"]);
    }

    #[test]
    fn parses_and_filters_dependencies_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "App",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "transitivelyLinkDependencies": true,
                    "dependencies": [
                        {"target": "name", "embed": false, "platformFilter": "all"},
                        {"target": "project/name", "embed": false, "platformFilter": "macOS"},
                        {"carthage": "name", "findFrameworks": true, "platformFilter": "iOS"},
                        {"carthage": "name", "findFrameworks": true, "linkType": "static"},
                        {"framework": "path", "weak": true},
                        {"sdk": "Contacts.framework"},
                        {"sdk": "Platforms/iPhoneOS.platform/Developer/Library/Frameworks/XCTest.framework", "root": "DEVELOPER_DIR"},
                        {"target": "conditionalMatch", "platforms": ["iOS"]},
                        {"target": "conditionalMiss", "platforms": ["watchOS"]}
                    ]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let dependencies = &project.targets["App"].dependencies;
        assert_eq!(
            project.targets["App"].transitively_link_dependencies,
            Some(true)
        );
        assert_eq!(dependencies.len(), 8);
        assert_eq!(dependencies[0].dependency_type, DependencyType::Target);
        assert_eq!(dependencies[0].reference, "name");
        assert_eq!(dependencies[0].embed, Some(false));
        assert_eq!(dependencies[2].platform_filter, PlatformFilter::Ios);
        assert_eq!(
            dependencies[3].dependency_type,
            DependencyType::Carthage {
                find_frameworks: Some(true),
                link_type: CarthageLinkType::Static
            }
        );
        assert!(dependencies[4].weak_link);
        assert_eq!(
            dependencies[6].dependency_type,
            DependencyType::Sdk {
                root: Some("DEVELOPER_DIR".to_owned())
            }
        );
        assert_eq!(dependencies[7].reference, "conditionalMatch");
    }

    #[test]
    fn parses_plists_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Plists",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "info": {
                        "path": "Info.plist",
                        "properties": {
                            "CFBundleName": "MyAppName",
                            "UIBackgroundModes": ["fetch"]
                        }
                    },
                    "entitlements": {
                        "path": "app.entitlements",
                        "properties": {
                            "com.apple.security.application-groups": "com.group"
                        }
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let target = &project.targets["App"];
        let info = target.info_plist.as_ref().unwrap();
        assert_eq!(info.path.as_deref(), Some("Info.plist"));
        assert_eq!(info.attributes["CFBundleName"], "MyAppName");
        assert_eq!(
            info.attributes["UIBackgroundModes"],
            serde_json::json!(["fetch"])
        );

        let entitlements = target.entitlements_plist.as_ref().unwrap();
        assert_eq!(entitlements.path.as_deref(), Some("app.entitlements"));
        assert_eq!(
            entitlements.attributes["com.apple.security.application-groups"],
            "com.group"
        );
    }

    #[test]
    fn parses_settings_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "SettingsTest",
            "configs": {"config1": "debug", "config2": "release"},
            "settingGroups": {
                "preset1": {"SETTING": "value"},
                "preset2": {"configs": {"config1": {"SETTING1": "value"}}},
                "preset3": {"base": {"SETTING": "value"}, "configs": {"config1": {"SETTING1": "value"}}},
                "preset5": {"groups": ["preset1"], "base": {"SETTING": "value"}},
                "preset7": {"base": {"SETTING": "value"}, "configs": {"config1": {"groups": ["preset1"], "base": {"SETTING": "value"}}}}
            },
            "settings": {
                "base": {"SETTING 5": "value 5"},
                "groups": ["preset7"],
                "configs": {"config1": {"SETTING 6": "value 6"}}
            },
            "targets": {
                "Target": {
                    "type": "application",
                    "platform": "iOS",
                    "settings": {
                        "groups": ["preset7"],
                        "base": {"SETTING 2": "value 2"},
                        "configs": {
                            "config1": {
                                "groups": ["preset1"],
                                "base": {"SETTING 3": "value 3"}
                            }
                        }
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        assert_eq!(
            project.setting_group_specs["preset1"].build_settings["SETTING"],
            "value"
        );
        assert_eq!(
            project.setting_group_specs["preset2"].config_settings["config1"].build_settings
                ["SETTING1"],
            "value"
        );
        assert_eq!(
            project.setting_group_specs["preset5"].groups,
            vec!["preset1"]
        );
        assert_eq!(
            project.setting_group_specs["preset7"].config_settings["config1"].groups,
            vec!["preset1"]
        );
        assert_eq!(project.settings_spec.groups, vec!["preset7"]);
        assert_eq!(
            project.settings_spec.config_settings["config1"].build_settings["SETTING 6"],
            "value 6"
        );

        let target_settings = &project.targets["Target"].settings_spec;
        assert_eq!(target_settings.groups, vec!["preset7"]);
        assert_eq!(target_settings.build_settings["SETTING 2"], "value 2");
        assert_eq!(
            target_settings.config_settings["config1"].build_settings["SETTING 3"],
            "value 3"
        );
        assert_eq!(
            target_settings.config_settings["config1"].groups,
            vec!["preset1"]
        );
    }

    #[test]
    fn exposes_product_type_and_platform_metadata_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Products",
            "targets": {
                "MyFramework": {"type": "framework", "platform": "iOS"},
                "MyStaticLibrary": {"type": "library.static", "platform": "iOS"},
                "MyDynamicLibrary": {"type": "dynamicLibrary", "platform": "iOS"}
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();

        assert!(project.targets["MyFramework"].target_type.is_framework());
        assert!(project.targets["MyStaticLibrary"].target_type.is_library());
        assert!(project.targets["MyDynamicLibrary"].target_type.is_library());
        assert_eq!(
            project.targets["MyFramework"].filename(),
            "MyFramework.framework"
        );
        assert_eq!(
            project.targets["MyStaticLibrary"].filename(),
            "libMyStaticLibrary.a"
        );
        assert_eq!(
            project.targets["MyDynamicLibrary"].filename(),
            "MyDynamicLibrary.dylib"
        );

        assert_eq!(Platform::Auto.deployment_target_setting(), "");
        assert_eq!(
            Platform::Ios.deployment_target_setting(),
            "IPHONEOS_DEPLOYMENT_TARGET"
        );
        assert_eq!(Platform::Macos.sdk_root(), "macosx");
        assert_eq!(Platform::Visionos.sdk_root(), "xros");
    }

    #[test]
    fn supports_legacy_local_packages() {
        let dictionary = serde_json::json!({
            "name": "spm",
            "localPackages": ["../XcodeGen", "Yams"]
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::from("/tmp/fixtures"), dictionary).unwrap();

        assert_eq!(
            project
                .packages
                .get("XcodeGen")
                .and_then(|value| value.get("path"))
                .and_then(Value::as_str),
            Some("../XcodeGen")
        );
        assert_eq!(
            project
                .packages
                .get("Yams")
                .and_then(|value| value.get("path"))
                .and_then(Value::as_str),
            Some("Yams")
        );
    }

    #[test]
    fn parses_breakpoints_and_actions_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Debug",
            "breakpoints": [
                {"type": "File", "path": "Foo.swift", "line": 7, "column": 14, "condition": "bar == nil"},
                {"type": "Exception", "scope": "All", "stopOnStyle": "Catch", "actions": [
                    {"type": "DebuggerCommand", "command": "po $arg1"},
                    {"type": "Log", "message": "message", "conveyanceType": "speak"},
                    {"type": "ShellCommand", "path": "script.sh", "arguments": "argument1, argument2", "waitUntilDone": true},
                    {"type": "GraphicsTrace"},
                    {"type": "AppleScript", "script": "display alert \"Hello!\""},
                    {"type": "Sound", "sound": "Hero"}
                ]},
                {"type": "SwiftError", "enabled": false},
                {"type": "OpenGLError", "ignoreCount": 2},
                {"type": "Symbolic", "symbol": "UIViewAlertForUnsatisfiableConstraints", "module": "UIKitCore"},
                {"type": "IDEConstraintError", "continueAfterRunningActions": true},
                {"type": "IDETestFailure"},
                {"type": "RuntimeIssue"}
            ]
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        assert_eq!(project.breakpoints.len(), 8);
        assert_eq!(
            project.breakpoints[0].breakpoint_type,
            BreakpointType::File {
                path: "Foo.swift".to_owned(),
                line: 7,
                column: Some(14)
            }
        );
        assert_eq!(
            project.breakpoints[0].condition.as_deref(),
            Some("bar == nil")
        );
        assert_eq!(
            project.breakpoints[1].breakpoint_type,
            BreakpointType::Exception {
                scope: BreakpointScope::All,
                stop_on_style: BreakpointStopOnStyle::Catch
            }
        );
        assert_eq!(
            project.breakpoints[1].actions[0],
            BreakpointAction::DebuggerCommand(Some("po $arg1".to_owned()))
        );
        assert_eq!(
            project.breakpoints[1].actions[1],
            BreakpointAction::Log {
                message: Some("message".to_owned()),
                conveyance_type: BreakpointLogConveyanceType::Speak
            }
        );
        assert_eq!(
            project.breakpoints[1].actions[5],
            BreakpointAction::Sound(BreakpointSound::Hero)
        );
        assert!(!project.breakpoints[2].enabled);
        assert_eq!(project.breakpoints[3].ignore_count, 2);
        assert!(project.breakpoints[6].enabled);
    }

    #[test]
    fn removes_empty_arrays_dictionaries_and_nulls_like_xcodegen() {
        let input = serde_json::json!({
            "inner1": "value1",
            "inner2": "value2",
            "inner3": null,
            "inner4": {
                "inner1": {
                    "inner1": "value1",
                    "inner2": "value2",
                    "inner3": null,
                    "inner4": [1, 2, 3]
                },
                "inner2": {
                    "inner1": "value1",
                    "inner2": "value2",
                    "inner3": {
                        "inner1": "value1",
                        "inner2": "value2",
                        "inner3": null,
                        "inner4": [1, 2, 3]
                    },
                    "inner4": [1, 2, 3]
                },
                "inner4": "value4",
                "inner5": null
            },
            "inner5": [],
            "inner6": {
                "inner1": "value1",
                "inner2": "value2",
                "inner3": [
                    {"inner1": "value1", "inner2": "value2", "inner3": null, "inner4": [1, 2, 3]},
                    {"inner1": "value1", "inner2": "value2", "inner3": null, "inner4": [1, 2, 3]}
                ]
            },
            "inner7": {}
        });
        let expected = serde_json::json!({
            "inner1": "value1",
            "inner2": "value2",
            "inner4": {
                "inner1": {"inner1": "value1", "inner2": "value2", "inner4": [1, 2, 3]},
                "inner2": {
                    "inner1": "value1",
                    "inner2": "value2",
                    "inner3": {"inner1": "value1", "inner2": "value2", "inner4": [1, 2, 3]},
                    "inner4": [1, 2, 3]
                },
                "inner4": "value4"
            },
            "inner6": {
                "inner1": "value1",
                "inner2": "value2",
                "inner3": [
                    {"inner1": "value1", "inner2": "value2", "inner4": [1, 2, 3]},
                    {"inner1": "value1", "inner2": "value2", "inner4": [1, 2, 3]}
                ]
            }
        });
        assert_eq!(remove_empty_arrays_dictionaries_and_nulls(input), expected);
    }

    #[test]
    fn formats_deployment_target_versions_like_xcodegen() {
        assert_eq!(format_deployment_target("2").unwrap(), "2.0");
        assert_eq!(format_deployment_target("2.0").unwrap(), "2.0");
        assert_eq!(format_deployment_target("2.1").unwrap(), "2.1");
        assert_eq!(format_deployment_target("2.10").unwrap(), "2.10");
        assert_eq!(format_deployment_target("2.1.0").unwrap(), "2.1");
        assert_eq!(format_deployment_target("2.12.0").unwrap(), "2.12");
        assert_eq!(format_deployment_target("2.1.2").unwrap(), "2.1.2");
        assert_eq!(format_deployment_target("2.10.2").unwrap(), "2.10.2");
        assert_eq!(format_deployment_target("2.0.2").unwrap(), "2.0.2");

        let dictionary = serde_json::json!({
            "name": "DeploymentTarget",
            "targets": {
                "App": {"type": "application", "platform": "iOS", "deploymentTarget": "2.1.0"}
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        assert_eq!(
            project.targets["App"].deployment_target.as_deref(),
            Some("2.1")
        );
    }

    #[test]
    fn parses_run_scripts_like_xcodegen() {
        let scripts = serde_json::json!([
            {"path": "script.sh"},
            {"script": "shell script\ndo thing", "name": "myscript", "inputFiles": ["file", "file2"], "outputFiles": ["file", "file2"], "shell": "bin/customshell", "runOnlyWhenInstalling": true},
            {"script": "shell script\ndo thing", "name": "myscript", "inputFiles": ["file", "file2"], "outputFiles": ["file", "file2"], "shell": "bin/customshell", "showEnvVars": false},
            {"script": "shell script\ndo thing", "name": "myscript", "inputFiles": ["file", "file2"], "outputFiles": ["file", "file2"], "shell": "bin/customshell", "basedOnDependencyAnalysis": false},
            {"script": "shell script\nwith file lists", "name": "myscript", "inputFileLists": ["inputList.xcfilelist"], "outputFileLists": ["outputList.xcfilelist"], "shell": "bin/customshell", "runOnlyWhenInstalling": true}
        ]);
        let dictionary = serde_json::json!({
            "name": "Scripts",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "preBuildScripts": scripts,
                    "postCompileScripts": scripts,
                    "postBuildScripts": scripts
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let target = &project.targets["App"];
        assert_eq!(target.pre_build_scripts.len(), 5);
        assert_eq!(target.post_compile_scripts, target.pre_build_scripts);
        assert_eq!(target.post_build_scripts, target.pre_build_scripts);
        assert_eq!(
            target.pre_build_scripts[0].script,
            BuildScriptKind::Path("script.sh".to_owned())
        );
        assert!(target.pre_build_scripts[1].run_only_when_installing);
        assert!(!target.pre_build_scripts[2].show_env_vars);
        assert!(!target.pre_build_scripts[3].based_on_dependency_analysis);
        assert_eq!(
            target.pre_build_scripts[4].input_file_lists,
            vec!["inputList.xcfilelist"]
        );
    }

    #[test]
    fn parses_build_rules_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Rules",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "buildRules": [
                        {
                            "name": "My Rule",
                            "script": "my script",
                            "filePattern": "*.swift",
                            "outputFiles": ["file1", "file2"],
                            "outputFilesCompilerFlags": ["-a", "-b"]
                        },
                        {
                            "compilerSpec": "apple.tool",
                            "fileType": "sourcecode.swift"
                        }
                    ]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let rules = &project.targets["App"].build_rules;
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[0].file_type,
            BuildRuleFileType::Pattern("*.swift".to_owned())
        );
        assert_eq!(
            rules[0].action,
            BuildRuleAction::Script("my script".to_owned())
        );
        assert_eq!(rules[0].name.as_deref(), Some("My Rule"));
        assert_eq!(rules[0].output_files, vec!["file1", "file2"]);
        assert_eq!(rules[0].output_files_compiler_flags, vec!["-a", "-b"]);
        assert_eq!(
            rules[1].file_type,
            BuildRuleFileType::Type("sourcecode.swift".to_owned())
        );
        assert_eq!(
            rules[1].action,
            BuildRuleAction::CompilerSpec("apple.tool".to_owned())
        );
        assert!(rules[1].run_once_per_architecture);
    }

    #[test]
    fn parses_build_tool_plugins_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Plugins",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "buildToolPlugins": [
                        {"plugin": "FirstPlugin", "package": "FirstPackage"},
                        {"plugin": "SecondPlugin", "package": "SecondPackage"}
                    ]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let plugins = &project.targets["App"].build_tool_plugins;
        assert_eq!(plugins.len(), 2);
        assert_eq!(plugins[0].plugin, "FirstPlugin");
        assert_eq!(plugins[0].package, "FirstPackage");
        assert_eq!(plugins[0].unique_id(), "FirstPlugin/FirstPackage");
        assert_eq!(plugins[1].plugin, "SecondPlugin");
        assert_eq!(plugins[1].package, "SecondPackage");
    }

    #[test]
    fn parses_aggregate_targets_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "AggregateProject",
            "aggregateTargets": {
                "AggregateTarget": {
                    "targets": ["target_1", "target_2"],
                    "settings": {"SETTING": "VALUE"},
                    "configFiles": {"debug": "file.xcconfig"},
                    "buildScripts": [{"script": "echo build"}]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let aggregate = &project.aggregate_target_specs["AggregateTarget"];
        assert_eq!(aggregate.name, "AggregateTarget");
        assert_eq!(aggregate.targets, vec!["target_1", "target_2"]);
        assert_eq!(aggregate.settings["SETTING"], "VALUE");
        assert_eq!(
            aggregate.config_files.get("debug").map(String::as_str),
            Some("file.xcconfig")
        );
        assert_eq!(
            aggregate.build_scripts[0].script,
            BuildScriptKind::Script("echo build".to_owned())
        );
    }

    #[test]
    fn parses_options_and_packages_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "spm",
            "options": {
                "carthageBuildPath": "../Carthage/Build",
                "carthageExecutablePath": "../bin/carthage",
                "bundleIdPrefix": "com.test",
                "createIntermediateGroups": true,
                "defaultSourceDirectoryType": "syncedFolder",
                "developmentLanguage": "ja",
                "deploymentTarget": {"iOS": 11.1, "tvOS": 10.0, "watchOS": "3", "macOS": "10.12.1"},
                "findCarthageFrameworks": true,
                "preGenCommand": "swiftgen",
                "postGenCommand": "pod install",
                "fileTypes": {"abc": {
                    "file": false,
                    "buildPhase": "sources",
                    "attributes": ["a1", "a2"],
                    "resourceTags": ["r1", "r2"],
                    "compilerFlags": ["c1", "c2"]
                }},
                "schemePathPrefix": "../",
                "localPackagesGroup": "MyPackages",
                "transitivelyLinkDependencies": true
            },
            "packages": {
                "package1": {"url": "package.git", "exactVersion": "1.2.2"},
                "package2": {"url": "package.git", "majorVersion": "1.2.2"},
                "package3": {"url": "package.git", "minorVersion": "1.2.2"},
                "package4": {"url": "package.git", "branch": "master"},
                "package5": {"url": "package.git", "revision": "x"},
                "package6": {"url": "package.git", "minVersion": "1.2.0", "maxVersion": "1.2.5"},
                "package7": {"github": "yonaskolb/XcodeGen", "version": "1.2.2"},
                "package8": {"path": "package/package", "group": "Packages/Feature"}
            },
            "localPackages": ["../XcodeGen"]
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::from("/tmp/fixtures"), dictionary).unwrap();
        let options = &project.spec_options;
        assert_eq!(
            options.carthage_build_path.as_deref(),
            Some("../Carthage/Build")
        );
        assert!(options.create_intermediate_groups);
        assert_eq!(
            options.default_source_directory_type,
            Some(SourceType::SyncedFolder)
        );
        assert_eq!(options.bundle_id_prefix.as_deref(), Some("com.test"));
        assert_eq!(options.deployment_target.ios.as_deref(), Some("11.1"));
        assert_eq!(options.deployment_target.tvos.as_deref(), Some("10.0"));
        assert_eq!(options.deployment_target.watchos.as_deref(), Some("3.0"));
        assert_eq!(options.deployment_target.macos.as_deref(), Some("10.12.1"));
        assert_eq!(
            options.file_types["abc"].build_phase,
            Some(FileBuildPhase::Sources)
        );
        assert_eq!(options.file_types["abc"].compiler_flags, vec!["c1", "c2"]);
        assert_eq!(options.local_packages_group.as_deref(), Some("MyPackages"));
        assert!(options.transitively_link_dependencies);

        assert_eq!(
            project.package_specs["package1"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::Exact("1.2.2".to_owned())
            }
        );
        assert_eq!(
            project.package_specs["package2"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::UpToNextMajorVersion(
                    "1.2.2".to_owned()
                )
            }
        );
        assert_eq!(
            project.package_specs["package3"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::UpToNextMinorVersion(
                    "1.2.2".to_owned()
                )
            }
        );
        assert_eq!(
            project.package_specs["package4"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::Branch("master".to_owned())
            }
        );
        assert_eq!(
            project.package_specs["package5"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::Revision("x".to_owned())
            }
        );
        assert_eq!(
            project.package_specs["package6"],
            SwiftPackage::Remote {
                url: "package.git".to_owned(),
                version_requirement: PackageVersionRequirement::Range {
                    from: "1.2.0".to_owned(),
                    to: "1.2.5".to_owned()
                }
            }
        );
        assert_eq!(
            project.package_specs["package7"],
            SwiftPackage::Remote {
                url: "https://github.com/yonaskolb/XcodeGen".to_owned(),
                version_requirement: PackageVersionRequirement::Exact("1.2.2".to_owned())
            }
        );
        assert_eq!(
            project.package_specs["package8"],
            SwiftPackage::Local {
                path: "package/package".to_owned(),
                group: Some("Packages/Feature".to_owned()),
                exclude_from_project: false
            }
        );
        assert_eq!(
            project.package_specs["XcodeGen"],
            SwiftPackage::Local {
                path: "../XcodeGen".to_owned(),
                group: None,
                exclude_from_project: false
            }
        );
    }

    #[test]
    fn parses_target_schemes_and_schemes_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Schemes",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {
                        "testTargets": ["t1", {"name": "t2"}],
                        "configVariants": ["dev", "app-store"],
                        "commandLineArguments": {"ENV1": true},
                        "gatherCoverageData": true,
                        "coverageTargets": ["t1"],
                        "storeKitConfiguration": "Configuration.storekit",
                        "language": "en",
                        "region": "US",
                        "disableMainThreadChecker": true,
                        "stopOnEveryMainThreadCheckerIssue": true,
                        "disableThreadPerformanceChecker": true,
                        "environmentVariables": {"TEST_VAR": "TEST_VAL"},
                        "preActions": [{"script": "dothing", "name": "Do Thing", "settingsTarget": "test"}],
                        "postActions": [{"script": "hello"}],
                        "management": {"shared": false, "isShown": true, "orderHint": 10}
                    }
                }
            },
            "schemes": {
                "Scheme": {
                    "build": {
                        "parallelizeBuild": false,
                        "buildImplicitDependencies": false,
                        "runPostActionsOnFailure": true,
                        "targets": {
                            "Target1": "all",
                            "Target2": "testing",
                            "Target3": "none",
                            "Target4": {"testing": true},
                            "Target5": {"testing": false},
                            "Target6": ["test", "analyze"],
                            "ExternalProject/Target7": ["run"]
                        },
                        "preActions": [{"script": "echo Before Build", "name": "Before Build", "settingsTarget": "Target1"}]
                    },
                    "run": {
                        "config": "debug",
                        "launchAutomaticallySubstyle": 2,
                        "enableGPUFrameCaptureMode": "disabled",
                        "storeKitConfiguration": "Configuration.storekit",
                        "disableThreadPerformanceChecker": true,
                        "environmentVariables": [
                            {"variable": "BOOL_TRUE", "value": true},
                            {"variable": "OTHER_ENV_VAR", "value": "VAL", "isEnabled": false}
                        ]
                    },
                    "test": {
                        "config": "debug",
                        "targets": [
                            "Target1",
                            {
                                "name": "ExternalProject/Target2",
                                "parallelizable": true,
                                "skipped": true,
                                "location": "test.gpx",
                                "randomExecutionOrder": true,
                                "skippedTests": ["Test/testExample()"]
                            }
                        ],
                        "gatherCoverageData": true,
                        "disableMainThreadChecker": true,
                        "stopOnEveryMainThreadCheckerIssue": true,
                        "testPlans": [{"path": "Path/Plan.xctestplan"}, {"path": "Path/Plan2.xctestplan", "defaultPlan": true}],
                        "preferredScreenCaptureFormat": "screenshots"
                    },
                    "management": {"isShown": false, "orderHint": 4}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let target_scheme = project.targets["App"].target_scheme.as_ref().unwrap();
        assert_eq!(target_scheme.test_targets, vec!["t1", "t2"]);
        assert_eq!(target_scheme.config_variants, vec!["dev", "app-store"]);
        assert_eq!(
            target_scheme.store_kit_configuration.as_deref(),
            Some("Configuration.storekit")
        );
        assert_eq!(
            target_scheme.environment_variables,
            vec![EnvironmentVariable {
                variable: "TEST_VAR".to_owned(),
                value: "TEST_VAL".to_owned(),
                enabled: true
            }]
        );
        assert_eq!(target_scheme.pre_actions[0].name, "Do Thing");
        assert_eq!(target_scheme.post_actions[0].name, "Run Script");
        assert_eq!(
            target_scheme.management,
            Some(SchemeManagement {
                shared: false,
                order_hint: Some(10),
                is_shown: Some(true)
            })
        );

        let scheme = &project.scheme_specs["Scheme"];
        assert_eq!(scheme.name, "Scheme");
        assert!(!scheme.build.parallelize_build);
        assert!(!scheme.build.build_implicit_dependencies);
        assert!(scheme.build.run_post_actions_on_failure);
        assert_eq!(scheme.build.targets[0].build_types.len(), 5);
        assert_eq!(
            scheme.build.targets[1].build_types,
            vec![BuildType::Testing, BuildType::Analyzing]
        );
        assert!(scheme.build.targets[2].build_types.is_empty());
        assert_eq!(scheme.build.pre_actions[0].script, "echo Before Build");

        let run = scheme.run.as_ref().unwrap();
        assert_eq!(run.launch_automatically_substyle.as_deref(), Some("2"));
        assert_eq!(
            run.store_kit_configuration.as_deref(),
            Some("Configuration.storekit")
        );
        assert!(run.disable_thread_performance_checker);
        assert_eq!(
            run.environment_variables[0],
            EnvironmentVariable {
                variable: "BOOL_TRUE".to_owned(),
                value: "YES".to_owned(),
                enabled: true
            }
        );
        assert!(!run.environment_variables[1].enabled);

        let test = scheme.test.as_ref().unwrap();
        assert!(test.gather_coverage_data);
        assert_eq!(test.targets[1].target_reference, "ExternalProject/Target2");
        assert!(test.targets[1].parallelizable);
        assert!(test.targets[1].random_execution_order);
        assert_eq!(test.targets[1].skipped_tests, vec!["Test/testExample()"]);
        assert!(test.test_plans[1].default_plan);
        assert_eq!(
            test.preferred_screen_capture_format.as_deref(),
            Some("screenshots")
        );
        assert_eq!(
            scheme.management,
            SchemeManagement {
                shared: true,
                order_hint: Some(4),
                is_shown: Some(false)
            }
        );
    }

    #[test]
    fn resolves_scheme_templates_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "SchemeTemplates",
            "schemeTemplates": {
                "base_scheme": {
                    "build": {
                        "parallelizeBuild": false,
                        "buildImplicitDependencies": false,
                        "runPostActionsOnFailure": true,
                        "targets": {
                            "Target${name_1}": "all",
                            "Target2": "testing",
                            "Target${name_3}": "none"
                        },
                        "preActions": [{
                            "script": "${pre-action-name}",
                            "name": "Before Build ${scheme_name}",
                            "settingsTarget": "Target${name_1}"
                        }]
                    },
                    "run": {"storeKitConfiguration": "Configuration.storekit"},
                    "test": {
                        "config": "debug",
                        "targets": [
                            "Target${name_1}",
                            {
                                "name": "Target2",
                                "parallelizable": true,
                                "randomExecutionOrder": true,
                                "skippedTests": ["Test/testExample()"]
                            }
                        ],
                        "gatherCoverageData": true,
                        "disableMainThreadChecker": true
                    },
                    "management": {"shared": false, "orderHint": 8}
                }
            },
            "schemes": {
                "temp2": {
                    "templates": ["base_scheme"],
                    "templateAttributes": {
                        "pre-action-name": "modified-name",
                        "name_1": "FirstTarget",
                        "name_3": "ThirdTarget"
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let scheme = &project.scheme_specs["temp2"];
        assert_eq!(scheme.name, "temp2");
        assert_eq!(scheme.build.targets[0].target, "TargetFirstTarget");
        assert_eq!(scheme.build.targets[1].target, "Target2");
        assert_eq!(scheme.build.targets[2].target, "TargetThirdTarget");
        assert_eq!(scheme.build.pre_actions[0].script, "modified-name");
        assert_eq!(scheme.build.pre_actions[0].name, "Before Build temp2");
        assert_eq!(
            scheme.build.pre_actions[0].settings_target.as_deref(),
            Some("TargetFirstTarget")
        );
        assert_eq!(
            scheme
                .run
                .as_ref()
                .unwrap()
                .store_kit_configuration
                .as_deref(),
            Some("Configuration.storekit")
        );
        assert_eq!(
            scheme.test.as_ref().unwrap().targets[1].skipped_tests,
            vec!["Test/testExample()"]
        );
        assert_eq!(
            scheme.management,
            SchemeManagement {
                shared: false,
                order_hint: Some(8),
                is_shown: None
            }
        );
    }

    #[test]
    fn validates_duplicate_dependencies_empty_sources_and_packages_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Validation",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["", "validSource"],
                    "dependencies": [
                        {"target": "Framework"},
                        {"target": "Framework"},
                        {"framework": "Vendor.framework"},
                        {"framework": "Vendor.framework"},
                        {"package": "MissingPackage", "product": "MissingProduct"},
                        {"package": "AllowedPackage", "product": "One"},
                        {"package": "AllowedPackage", "product": "Two"}
                    ],
                    "buildToolPlugins": [
                        {"plugin": "Plugin", "package": "MissingPluginPackage"}
                    ]
                },
                "Framework": {
                    "type": "framework",
                    "platform": "iOS"
                }
            },
            "packages": {
                "AllowedPackage": {"url": "package.git", "branch": "main"}
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(&ValidationError::EmptySourcePath {
            target: "App".to_owned()
        }));
        assert!(errors.contains(&ValidationError::DuplicateDependencies {
            target: "App".to_owned(),
            dependency_reference: "Framework".to_owned()
        }));
        assert!(errors.contains(&ValidationError::DuplicateDependencies {
            target: "App".to_owned(),
            dependency_reference: "Vendor.framework".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSwiftPackage {
            name: "MissingPackage".to_owned(),
            target: "App".to_owned()
        }));
        assert!(
            errors.contains(&ValidationError::InvalidPluginPackageReference {
                plugin: "Plugin".to_owned(),
                package: "MissingPluginPackage".to_owned()
            })
        );
        assert!(!errors.contains(&ValidationError::DuplicateDependencies {
            target: "App".to_owned(),
            dependency_reference: "AllowedPackage".to_owned()
        }));
    }

    #[test]
    fn validates_build_tool_plugin_package_reference_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Plugins",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "buildToolPlugins": [{"plugin": "Plugin", "package": "Package"}]
                }
            },
            "packages": {
                "Package": {"url": "url", "branch": "branch"}
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        assert!(project.validate().is_ok());
    }

    #[test]
    fn validates_configs_settings_groups_and_target_schemes_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Validation",
            "configs": {"Debug": "debug", "Release": "release"},
            "options": {"defaultConfig": "Missing"},
            "settings": {
                "groups": ["missingSettingsGroup"],
                "configs": {"MissingConfig": {"SETTING": "VALUE"}}
            },
            "settingGroups": {
                "preset": {
                    "groups": ["missingNestedSettingsGroup"],
                    "configs": {"MissingNestedConfig": {"SETTING": "VALUE"}}
                }
            },
            "configFiles": {"MissingConfigFileConfig": "invalid.xcconfig"},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "settings": {
                        "groups": ["missingTargetSettingsGroup"],
                        "configs": {"MissingTargetConfig": {"SETTING": "VALUE"}}
                    },
                    "configFiles": {"MissingTargetConfigFileConfig": "target.xcconfig"},
                    "scheme": {"testTargets": ["MissingTests"]}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(&ValidationError::MissingDefaultConfig {
            config_name: "Missing".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidConfigFileConfig(
            "MissingConfigFileConfig".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidBuildSettingConfig(
            "MissingConfig".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidSettingsGroup(
            "missingSettingsGroup".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidBuildSettingConfig(
            "MissingNestedConfig".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidSettingsGroup(
            "missingNestedSettingsGroup".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidBuildSettingConfig(
            "MissingTargetConfig".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidSettingsGroup(
            "missingTargetSettingsGroup".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidTargetConfigFile {
            target: "App".to_owned(),
            config_file: "target.xcconfig".to_owned(),
            config: "MissingTargetConfigFileConfig".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidTargetSchemeTest {
            target: "App".to_owned(),
            test_target: "MissingTests".to_owned()
        }));
    }

    #[test]
    fn validates_supported_destinations_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Destinations",
            "targets": {
                "WatchApp": {
                    "type": "application",
                    "platform": "watchOS",
                    "supportedDestinations": ["macOS"]
                },
                "MultiApp": {
                    "type": "application",
                    "platform": "auto",
                    "supportedDestinations": ["watchOS"]
                },
                "MacConflict": {
                    "type": "application",
                    "platform": "iOS",
                    "supportedDestinations": ["macOS", "macCatalyst"]
                },
                "BadCatalyst": {
                    "type": "application",
                    "platform": "tvOS",
                    "supportedDestinations": ["tvOS", "macCatalyst"]
                },
                "MissingPlatform": {
                    "type": "application",
                    "platform": "iOS",
                    "supportedDestinations": ["tvOS"]
                },
                "WatchFrameworkAllowed": {
                    "type": "framework",
                    "platform": "auto",
                    "supportedDestinations": ["watchOS"]
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(
            &ValidationError::UnexpectedTargetPlatformForSupportedDestinations {
                target: "WatchApp".to_owned(),
                platform: Platform::Watchos
            }
        ));
        assert!(errors.contains(
            &ValidationError::ContainsWatchOSDestinationForMultiplatformApp {
                target: "MultiApp".to_owned()
            }
        ));
        assert!(errors.contains(
            &ValidationError::MultipleMacPlatformsInSupportedDestinations {
                target: "MacConflict".to_owned()
            }
        ));
        assert!(errors.contains(
            &ValidationError::InvalidTargetPlatformForSupportedDestinations {
                target: "BadCatalyst".to_owned()
            }
        ));
        assert!(errors.contains(
            &ValidationError::MissingTargetPlatformInSupportedDestinations {
                target: "MissingPlatform".to_owned(),
                platform: Platform::Ios
            }
        ));
        assert!(!errors.contains(
            &ValidationError::ContainsWatchOSDestinationForMultiplatformApp {
                target: "WatchFrameworkAllowed".to_owned()
            }
        ));
    }

    #[test]
    fn validates_aggregate_targets_schemes_and_default_test_plans_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Validation",
            "configs": {"Debug": "debug", "Release": "release"},
            "targets": {
                "App": {"type": "application", "platform": "iOS"}
            },
            "aggregateTargets": {
                "Aggregate": {
                    "targets": ["MissingDependency"],
                    "configFiles": {"MissingConfig": "aggregate.xcconfig"},
                    "scheme": {"testTargets": ["MissingAggregateTests"]}
                }
            },
            "schemes": {
                "BrokenScheme": {
                    "build": {"targets": {"MissingBuildTarget": "all"}},
                    "run": {"config": "MissingRunConfig"},
                    "test": {
                        "config": "MissingTestConfig",
                        "targets": ["MissingTestTarget"],
                        "testPlans": [
                            {"path": "Plan1.xctestplan", "defaultPlan": true},
                            {"path": "Plan2.xctestplan", "defaultPlan": true}
                        ]
                    },
                    "archive": {"config": "MissingArchiveConfig"}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(&ValidationError::InvalidTargetDependency {
            target: "Aggregate".to_owned(),
            dependency: "MissingDependency".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidTargetConfigFile {
            target: "Aggregate".to_owned(),
            config_file: "aggregate.xcconfig".to_owned(),
            config: "MissingConfig".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidTargetSchemeTest {
            target: "Aggregate".to_owned(),
            test_target: "MissingAggregateTests".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSchemeTarget {
            scheme: "BrokenScheme".to_owned(),
            target: "MissingBuildTarget".to_owned(),
            action: "build".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSchemeTarget {
            scheme: "BrokenScheme".to_owned(),
            target: "MissingTestTarget".to_owned(),
            action: "test".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSchemeConfig {
            scheme: "BrokenScheme".to_owned(),
            config: "MissingRunConfig".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSchemeConfig {
            scheme: "BrokenScheme".to_owned(),
            config: "MissingTestConfig".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSchemeConfig {
            scheme: "BrokenScheme".to_owned(),
            config: "MissingArchiveConfig".to_owned()
        }));
        assert!(errors.contains(&ValidationError::MultipleDefaultTestPlans));
    }

    #[test]
    fn validates_project_references_in_schemes_and_dependencies_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "ProjectReferences",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "MissingProject/ExternalTarget"},
                        {"target": "ValidProject/ExternalTarget"}
                    ]
                }
            },
            "projectReferences": {
                "ValidProject": {"path": "ValidProject.xcodeproj"}
            },
            "schemes": {
                "BrokenScheme": {
                    "build": {"targets": {"MissingProject/ExternalTarget": "all"}},
                    "test": {
                        "targets": ["MissingProject/ExternalTests"],
                        "coverageTargets": ["MissingCoverageProject/ExternalTarget"]
                    }
                },
                "AllowedScheme": {
                    "build": {"targets": {"ValidProject/ExternalTarget": "all"}}
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(&ValidationError::InvalidTargetDependency {
            target: "App".to_owned(),
            dependency: "MissingProject/ExternalTarget".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidProjectReference {
            scheme: "BrokenScheme".to_owned(),
            reference: "MissingProject".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidProjectReference {
            scheme: "BrokenScheme".to_owned(),
            reference: "MissingCoverageProject".to_owned()
        }));
        assert!(!errors.contains(&ValidationError::InvalidTargetDependency {
            target: "App".to_owned(),
            dependency: "ValidProject/ExternalTarget".to_owned()
        }));
        assert!(!errors.contains(&ValidationError::InvalidProjectReference {
            scheme: "AllowedScheme".to_owned(),
            reference: "ValidProject".to_owned()
        }));
    }

    #[test]
    fn validates_minimum_xcodegen_version_like_xcodegen() {
        let dictionary = serde_json::json!({
            "name": "Versioned",
            "options": {"minimumXcodeGenVersion": "1.11.1"}
        })
        .as_object()
        .unwrap()
        .clone();
        let project = Project::from_dictionary(PathBuf::new(), dictionary).unwrap();

        for version in ["1.11.0", "1.10.99", "0.99"] {
            let errors = project
                .validate_minimum_xcodegen_version(version)
                .unwrap_err()
                .errors;
            assert_eq!(
                errors,
                vec![ValidationError::InvalidXcodeGenVersion {
                    minimum_version: "1.11.1".to_owned(),
                    version: version.to_owned()
                }]
            );
        }
        assert!(project.validate_minimum_xcodegen_version("1.11.1").is_ok());
        assert!(project.validate_minimum_xcodegen_version("1.12.0").is_ok());
    }

    #[test]
    fn validates_missing_files_and_invalid_sdk_like_xcodegen() {
        let temp = tempfile::TempDir::new().unwrap();
        let dictionary = serde_json::json!({
            "name": "FilesystemValidation",
            "configs": {"Debug": "debug", "Release": "release"},
            "settings": {"Debug": "VALUE"},
            "configFiles": {"Debug": "missing.xcconfig"},
            "fileGroups": ["MissingGroup"],
            "packages": {
                "MissingLocalPackage": {"path": "MissingPackage"}
            },
            "projectReferences": {
                "MissingProject": {"path": "MissingProject.xcodeproj"}
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Missing.swift",
                        {"path": "Generated.swift", "optional": true}
                    ],
                    "dependencies": [
                        {"sdk": "invalidDependency"}
                    ],
                    "preBuildScripts": [{"path": "missing-pre.sh", "name": "pre"}],
                    "postCompileScripts": [{"path": "missing-post-compile.sh"}],
                    "postBuildScripts": [{"path": "missing-post.sh"}]
                }
            },
            "aggregateTargets": {
                "Aggregate": {
                    "buildScripts": [{"path": "missing-aggregate.sh", "name": "aggregate"}]
                }
            },
            "schemes": {
                "App": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "testPlans": [{"path": "Missing.xctestplan"}]
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(temp.path().to_path_buf(), dictionary).unwrap();
        let errors = project.validate().unwrap_err().errors;
        assert!(errors.contains(&ValidationError::InvalidPerConfigSettings));
        assert!(errors.contains(&ValidationError::InvalidConfigFile {
            config_file: "missing.xcconfig".to_owned(),
            config: "Debug".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidFileGroup(
            "MissingGroup".to_owned()
        )));
        assert!(errors.contains(&ValidationError::InvalidLocalPackage(
            "MissingLocalPackage".to_owned()
        )));
        assert!(
            errors.contains(&ValidationError::InvalidProjectReferencePath {
                name: "MissingProject".to_owned(),
                path: "MissingProject.xcodeproj".to_owned()
            })
        );
        assert!(errors.contains(&ValidationError::InvalidTargetSource {
            target: "App".to_owned(),
            source: "Missing.swift".to_owned()
        }));
        assert!(!errors.contains(&ValidationError::InvalidTargetSource {
            target: "App".to_owned(),
            source: "Generated.swift".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidSdkDependency {
            target: "App".to_owned(),
            dependency: "invalidDependency".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidBuildScriptPath {
            target: "App".to_owned(),
            name: Some("pre".to_owned()),
            path: "missing-pre.sh".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidBuildScriptPath {
            target: "App".to_owned(),
            name: None,
            path: "missing-post-compile.sh".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidBuildScriptPath {
            target: "App".to_owned(),
            name: None,
            path: "missing-post.sh".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidBuildScriptPath {
            target: "Aggregate".to_owned(),
            name: Some("aggregate".to_owned()),
            path: "missing-aggregate.sh".to_owned()
        }));
        assert!(errors.contains(&ValidationError::InvalidTestPlan(TestPlan {
            path: "Missing.xctestplan".to_owned(),
            default_plan: false
        })));
    }

    #[test]
    fn allows_existing_files_project_references_and_optional_sources_like_xcodegen() {
        let temp = tempfile::TempDir::new().unwrap();
        fs::write(temp.path().join("debug.xcconfig"), "").unwrap();
        fs::write(temp.path().join("Sources.swift"), "").unwrap();
        fs::write(temp.path().join("script.sh"), "").unwrap();
        fs::write(temp.path().join("Plan.xctestplan"), "{}").unwrap();
        fs::create_dir(temp.path().join("FileGroup")).unwrap();
        fs::create_dir(temp.path().join("LocalPackage")).unwrap();
        fs::create_dir(temp.path().join("External.xcodeproj")).unwrap();

        let dictionary = serde_json::json!({
            "name": "ValidFilesystem",
            "configs": {"Debug": "debug"},
            "configFiles": {"Debug": "debug.xcconfig"},
            "fileGroups": ["FileGroup"],
            "packages": {
                "LocalPackage": {"path": "LocalPackage"}
            },
            "projectReferences": {
                "External": {"path": "External.xcodeproj"}
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources.swift",
                        {"path": "Generated.swift", "optional": true}
                    ],
                    "dependencies": [
                        {"sdk": "Contacts.framework"},
                        {"target": "External/ExternalTarget"}
                    ],
                    "preBuildScripts": [{"path": "script.sh"}]
                }
            },
            "schemes": {
                "App": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "testPlans": [{"path": "Plan.xctestplan"}]
                    }
                }
            }
        })
        .as_object()
        .unwrap()
        .clone();

        let project = Project::from_dictionary(temp.path().to_path_buf(), dictionary).unwrap();
        assert!(project.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_package_versions_like_xcodegen() {
        for package in [
            serde_json::json!({"url": "package.git", "majorVersion": "master"}),
            serde_json::json!({"url": "package.git", "from": "develop"}),
            serde_json::json!({"url": "package.git", "minVersion": "feature/swift5.2", "maxVersion": "9.1.0"}),
            serde_json::json!({"url": "package.git", "minorVersion": "x.1.2"}),
            serde_json::json!({"url": "package.git", "exactVersion": "1.2.3.1"}),
            serde_json::json!({"url": "package.git", "version": "foo-bar"}),
        ] {
            let dictionary = serde_json::json!({
                "name": "Packages",
                "packages": {
                    "BadPackage": package
                }
            })
            .as_object()
            .unwrap()
            .clone();
            assert!(matches!(
                Project::from_dictionary(PathBuf::new(), dictionary),
                Err(SpecError::InvalidVersion(_))
            ));
        }

        let dictionary = serde_json::json!({
            "name": "Packages",
            "packages": {
                "BetaPackage": {"url": "package.git", "majorVersion": "4.0.0-beta.5"}
            }
        })
        .as_object()
        .unwrap()
        .clone();
        assert!(Project::from_dictionary(PathBuf::new(), dictionary).is_ok());
    }
}
