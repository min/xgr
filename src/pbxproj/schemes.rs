use super::graph::{mapped_id, object_id};
use super::{
    is_watch_app_product, project_xcode_version_last_upgrade_check, xml_escape, ProjectWriteError,
};
use crate::spec::{DependencyType, ProductType, Project, Target};
use serde_json::Value;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

pub(super) struct SchemeManagementState {
    pub(super) name: String,
    pub(super) shared: bool,
    pub(super) is_shown: Option<bool>,
    pub(super) order_hint: Option<i64>,
}

pub(super) fn write_schemes(
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

pub(super) fn write_scheme_management(
    project: &Project,
    project_path: &Path,
) -> Result<(), ProjectWriteError> {
    let states = scheme_management_states(project);
    if states.is_empty() {
        return Ok(());
    }
    let schemes_dir = project_path.join("xcuserdata/xgr.xcuserdatad/xcschemes");
    fs::create_dir_all(&schemes_dir).map_err(|source| ProjectWriteError::Write {
        path: schemes_dir.clone(),
        source,
    })?;
    let path = schemes_dir.join("xcschememanagement.plist");
    fs::write(&path, scheme_management_plist(&states))
        .map_err(|source| ProjectWriteError::Write { path, source })
}

pub(super) fn write_breakpoints(
    project: &Project,
    project_path: &Path,
) -> Result<(), ProjectWriteError> {
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
