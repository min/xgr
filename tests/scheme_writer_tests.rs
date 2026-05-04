use serde_json::Value;
use std::fs;
use xgr::{Project, ProjectWriter};

fn project_from_json(base_path: std::path::PathBuf, value: Value) -> Project {
    Project::from_dictionary(base_path, value.as_object().unwrap().clone()).unwrap()
}

fn compact_xml(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" = ", "=")
}

fn xml_contains(haystack: &str, needle: &str) -> bool {
    compact_xml(haystack).contains(&compact_xml(needle))
}

fn xml_match_count(haystack: &str, needle: &str) -> usize {
    compact_xml(haystack).matches(&compact_xml(needle)).count()
}

#[test]
fn writer_generates_shared_scheme_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SchemeProject",
            "options": {"schemePathPrefix": "../"},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "Tests"}]
                },
                "Tests": {
                    "type": "unitTestBundle",
                    "platform": "iOS",
                    "dependencies": [{"target": "App"}]
                }
            },
            "schemes": {
                "MyScheme": {
                    "build": {
                        "parallelizeBuild": false,
                        "buildImplicitDependencies": false,
                        "targets": {"App": "all"},
                        "preActions": [
                            {"name": "Script", "script": "echo Starting", "settingsTarget": "App"}
                        ]
                    },
                    "run": {
                        "config": "Debug",
                        "debugEnabled": false,
                        "launchAutomaticallySubstyle": "2",
                        "askForAppToLaunch": true,
                        "customLLDBInit": "/sample/.lldbinit",
                        "customWorkingDirectory": "/test",
                        "enableGPUFrameCaptureMode": "metal",
                        "storeKitConfiguration": "Configuration.storekit",
                        "language": "en",
                        "region": "US",
                        "commandLineArguments": {
                            "-UITestMode": true,
                            "-SkipIntro": false
                        },
                        "environmentVariables": {"RUN_ENV": "ENABLED"}
                    },
                    "test": {
                        "config": "Debug",
                        "customLLDBInit": "/test/.lldbinit",
                        "targets": [
                            {"target": "Tests", "location": "test.gpx"},
                            {"target": "Tests", "location": "New York, NY, USA"}
                        ],
                        "gatherCoverageData": true,
                        "coverageTargets": ["App"],
                        "testPlans": [{"path": "App.xctestplan", "default": true}],
                        "environmentVariables": [
                            {"variable": "TEST_ENV", "value": "1", "isEnabled": false}
                        ]
                    },
                    "profile": {"config": "Release", "askForAppToLaunch": true}
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/MyScheme.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "parallelizeBuildables=\"NO\""));
    assert!(xml_contains(&scheme, "buildImplicitDependencies=\"NO\""));
    assert!(xml_contains(&scheme, "BuildableName=\"App.app\""));
    assert!(xml_contains(&scheme, "BlueprintName=\"App\""));
    assert!(xml_contains(&scheme, "title=\"Script\""));
    assert!(xml_contains(&scheme, "scriptText=\"echo Starting\""));
    assert!(xml_contains(&scheme, "selectedDebuggerIdentifier=\"\""));
    assert!(xml_contains(
        &scheme,
        "selectedLauncherIdentifier=\"Xcode.IDEFoundation.Launcher.PosixSpawn\""
    ));
    assert!(xml_contains(&scheme, "launchAutomaticallySubstyle=\"2\""));
    assert!(xml_contains(&scheme, "askForAppToLaunch=\"YES\""));
    assert!(xml_contains(
        &scheme,
        "customLLDBInitFile=\"/sample/.lldbinit\""
    ));
    assert!(xml_contains(&scheme, "useCustomWorkingDirectory=\"YES\""));
    assert!(xml_contains(&scheme, "customWorkingDirectory=\"/test\""));
    assert!(xml_contains(&scheme, "enableGPUFrameCaptureMode=\"metal\""));
    assert!(xml_contains(&scheme, "language=\"en\""));
    assert!(xml_contains(&scheme, "region=\"US\""));
    assert!(xml_contains(
        &scheme,
        "argument=\"-UITestMode\" isEnabled=\"YES\""
    ));
    assert!(xml_contains(
        &scheme,
        "argument=\"-SkipIntro\" isEnabled=\"NO\""
    ));
    assert!(xml_contains(
        &scheme,
        "customLLDBInitFile=\"/test/.lldbinit\""
    ));
    assert!(xml_contains(
        &scheme,
        "identifier=\"../Configuration.storekit\""
    ));
    assert!(xml_contains(
        &scheme,
        "identifier=\"../test.gpx\" referenceType=\"0\""
    ));
    assert!(xml_contains(
        &scheme,
        "identifier=\"New York, NY, USA\" referenceType=\"1\""
    ));
    assert!(xml_contains(
        &scheme,
        "key=\"RUN_ENV\" value=\"ENABLED\" isEnabled=\"YES\""
    ));
    assert!(xml_contains(
        &scheme,
        "key=\"TEST_ENV\" value=\"1\" isEnabled=\"NO\""
    ));
    assert!(xml_contains(&scheme, "codeCoverageEnabled=\"YES\""));
    assert!(xml_contains(
        &scheme,
        "reference=\"container:App.xctestplan\" default=\"YES\""
    ));
    assert!(xml_contains(
        &scheme,
        "ProfileAction buildConfiguration=\"Release\""
    ));
    assert!(xml_contains(&scheme, "askForAppToLaunch=\"YES\""));
}

#[test]
fn writer_generates_target_scheme_config_variants_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "TargetSchemes",
            "configs": {
                "Staging-Debug": "debug",
                "Production-Debug": "debug",
                "Staging Release": "release",
                "Production Release": "release"
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {
                        "configVariants": ["Staging", "Production"],
                        "testTargets": ["Tests"],
                        "gatherCoverageData": true,
                        "coverageTargets": ["App"],
                        "storeKitConfiguration": "Configuration.storekit",
                        "language": "fr",
                        "region": "CA",
                        "commandLineArguments": {"-TargetScheme": true},
                        "environmentVariables": {"ENV": "VALUE"}
                    }
                },
                "Tests": {
                    "type": "unitTestBundle",
                    "platform": "iOS",
                    "dependencies": [{"target": "App"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let staging = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/App Staging.xcscheme"),
    )
    .unwrap();
    let production = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/App Production.xcscheme"),
    )
    .unwrap();

    assert!(xml_contains(
        &staging,
        "buildConfiguration=\"Staging-Debug\""
    ));
    assert!(xml_contains(
        &staging,
        "buildConfiguration=\"Staging Release\""
    ));
    assert!(xml_contains(
        &production,
        "buildConfiguration=\"Production-Debug\""
    ));
    assert!(xml_contains(
        &production,
        "buildConfiguration=\"Production Release\""
    ));
    assert!(xml_contains(&staging, "BuildableName=\"Tests.xctest\""));
    assert!(xml_contains(&staging, "codeCoverageEnabled=\"YES\""));
    assert!(xml_contains(
        &staging,
        "identifier=\"Configuration.storekit\""
    ));
    assert!(xml_contains(&staging, "language=\"fr\""));
    assert!(xml_contains(&staging, "region=\"CA\""));
    assert!(xml_contains(
        &staging,
        "argument=\"-TargetScheme\" isEnabled=\"YES\""
    ));
    assert!(xml_contains(
        &staging,
        "key=\"ENV\" value=\"VALUE\" isEnabled=\"YES\""
    ));
}

#[test]
fn writer_generates_macro_expansion_for_run_and_test_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "MacroExpansion",
            "targets": {
                "MyApp": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "MyAppExtension", "embed": false}]
                },
                "MyAppExtension": {
                    "type": "app-extension",
                    "platform": "iOS"
                }
            },
            "schemes": {
                "TestScheme": {
                    "build": {
                        "targets": {
                            "MyApp": ["run"],
                            "MyAppExtension": ["run"]
                        }
                    },
                    "run": {
                        "config": "Debug",
                        "macroExpansion": "MyApp"
                    },
                    "test": {
                        "macroExpansion": "MyAppExtension"
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/TestScheme.xcscheme"),
    )
    .unwrap();
    assert_eq!(xml_match_count(&scheme, "<MacroExpansion>"), 2);
    assert!(xml_contains(&scheme, "BuildableName=\"MyApp.app\""));
    assert!(xml_contains(
        &scheme,
        "BuildableName=\"MyAppExtension.appex\""
    ));
}

#[test]
fn writer_uses_testing_runnable_for_test_macro_expansion_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "MacroExpansion",
            "targets": {
                "MyApp": {"type": "application", "platform": "iOS"},
                "MockApp": {"type": "application", "platform": "iOS"},
                "TestBundle": {"type": "unitTestBundle", "platform": "iOS"}
            },
            "schemes": {
                "TestScheme": {
                    "build": {
                        "targets": {
                            "MyApp": ["run"],
                            "MockApp": ["test"],
                            "TestBundle": ["test"]
                        }
                    },
                    "run": {
                        "config": "Debug",
                        "macroExpansion": "MyApp"
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/TestScheme.xcscheme"),
    )
    .unwrap();
    assert_eq!(xml_match_count(&scheme, "<MacroExpansion>"), 2);
    assert!(xml_contains(&scheme, "BuildableName=\"MyApp.app\""));
    assert!(xml_contains(&scheme, "BuildableName=\"MockApp.app\""));
}

#[test]
fn writer_generates_test_target_references_for_local_swift_packages_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "PackageTests",
            "packages": {
                "XcodeGen": {"path": "../"}
            },
            "targets": {
                "MyApp": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "XcodeGen"}],
                    "scheme": {
                        "testTargets": ["XcodeGen/XcodeGenKitTests"]
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/MyApp.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(
        &scheme,
        "BlueprintIdentifier=\"XcodeGenKitTests\""
    ));
    assert!(xml_contains(&scheme, "BuildableName=\"XcodeGenKitTests\""));
    assert!(xml_contains(&scheme, "BlueprintName=\"XcodeGenKitTests\""));
    assert!(xml_contains(
        &scheme,
        "ReferencedContainer=\"container:../\""
    ));
}

#[test]
fn writer_generates_breakpoints_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Breakpoints",
            "breakpoints": [
                {"type": "Exception", "scope": "All", "stopOnStyle": "Catch"},
                {
                    "type": "File",
                    "path": "Sources/App.swift",
                    "line": 7,
                    "column": 13,
                    "condition": "launchOptions == nil"
                }
            ]
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let breakpoints = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcdebugger/Breakpoints_v2.xcbkptlist"),
    )
    .unwrap();
    assert!(xml_contains(
        &breakpoints,
        "Xcode.Breakpoint.ExceptionBreakpoint"
    ));
    assert!(xml_contains(
        &breakpoints,
        "Xcode.Breakpoint.FileBreakpoint"
    ));
    assert!(xml_contains(&breakpoints, "filePath=\"Sources/App.swift\""));
    assert!(xml_contains(&breakpoints, "startingLineNumber=\"7\""));
    assert!(xml_contains(&breakpoints, "startingColumnNumber=\"13\""));
    assert!(xml_contains(
        &breakpoints,
        "condition=\"launchOptions == nil\""
    ));
}

#[test]
fn writer_selects_first_runnable_scheme_target_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "RunnableSelection",
            "targets": {
                "Framework": {"type": "framework", "platform": "iOS"},
                "App": {"type": "application", "platform": "iOS"}
            },
            "schemes": {
                "MyScheme": {
                    "build": {
                        "targets": {
                            "Framework": ["archive"],
                            "App": ["run"]
                        }
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/MyScheme.xcscheme"),
    )
    .unwrap();
    let launch_action = scheme
        .split("<LaunchAction")
        .nth(1)
        .and_then(|value| value.split("</LaunchAction>").next())
        .unwrap();
    assert!(xml_contains(launch_action, "BuildableName=\"App.app\""));
    assert!(!xml_contains(
        launch_action,
        "BuildableName=\"Framework.framework\""
    ));
}

#[test]
fn writer_generates_external_project_build_and_coverage_references_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let external_path = temp.path().join("ExternalProject.xcodeproj");
    fs::create_dir(&external_path).unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "ExternalReferences",
            "projectReferences": {
                "ExternalProject": {"path": "ExternalProject.xcodeproj"}
            },
            "targets": {
                "Framework": {"type": "framework", "platform": "iOS"}
            },
            "schemes": {
                "ExternalProjectScheme": {
                    "build": {
                        "targets": {
                            "ExternalProject/ExternalTarget": "all"
                        }
                    },
                    "test": {
                        "config": "Debug",
                        "gatherCoverageData": true,
                        "coverageTargets": [
                            "ExternalProject/ExternalTarget",
                            "Framework"
                        ]
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/ExternalProjectScheme.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "BlueprintName=\"ExternalTarget\""));
    assert!(xml_contains(
        &scheme,
        "ReferencedContainer=\"container:ExternalProject.xcodeproj\""
    ));
    assert!(xml_contains(&scheme, "<CodeCoverageTargets>"));
    assert_eq!(
        xml_match_count(&scheme, "BlueprintName=\"ExternalTarget\""),
        2
    );
    assert!(xml_contains(&scheme, "BlueprintName=\"Framework\""));
}

#[test]
fn writer_generates_watch_target_scheme_remote_runnable_and_host_build_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "WatchScheme",
            "options": {"schemePathPrefix": "../"},
            "targets": {
                "WatchExtension": {
                    "type": "watch2Extension",
                    "platform": "watchOS"
                },
                "WatchApp": {
                    "type": "watch2App",
                    "platform": "watchOS",
                    "dependencies": [{"target": "WatchExtension"}],
                    "scheme": {"storeKitConfiguration": "Configuration.storekit"}
                },
                "HostApp": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "WatchApp"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/WatchApp.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(
        &scheme,
        "<RemoteRunnable runnableDebuggingMode=\"2\">"
    ));
    assert!(xml_contains(&scheme, "BuildableName=\"WatchApp.app\""));
    assert!(xml_contains(&scheme, "BuildableName=\"HostApp.app\""));
    assert!(xml_contains(
        &scheme,
        "identifier=\"../Configuration.storekit\""
    ));
}

#[test]
fn writer_generates_pre_and_post_actions_for_target_schemes_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "TargetSchemeActions",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {
                        "preActions": [
                            {"name": "Run", "script": "do", "settingsTarget": "App"}
                        ],
                        "postActions": [
                            {"name": "Cleanup", "script": "done", "settingsTarget": "App"}
                        ]
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/App.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "<PreActions>"));
    assert!(xml_contains(&scheme, "title=\"Run\""));
    assert!(xml_contains(&scheme, "scriptText=\"do\""));
    assert!(xml_contains(&scheme, "<PostActions>"));
    assert!(xml_contains(&scheme, "title=\"Cleanup\""));
    assert!(xml_contains(&scheme, "scriptText=\"done\""));
    assert!(xml_contains(&scheme, "BuildableName=\"App.app\""));
}

#[test]
fn writer_generates_scheme_management_for_hidden_target_scheme_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SchemeManagement",
            "targets": {
                "MyApp": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {
                        "management": {"isShown": false}
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let management = fs::read_to_string(
        generated
            .project_path
            .join("xcuserdata/xgr.xcuserdatad/xcschemes/xcschememanagement.plist"),
    )
    .unwrap();
    assert!(xml_contains(&management, "MyApp.xcscheme_^#shared#^_"));
    assert!(xml_contains(&management, "<key>isShown</key>"));
    assert!(xml_contains(&management, "<false />"));
    assert!(!xml_contains(&management, "<key>orderHint</key>"));
}

#[test]
fn writer_generates_scheme_without_debugger_for_test_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "NoDebuggerTest",
            "targets": {
                "App": {"type": "application", "platform": "iOS"},
                "Tests": {"type": "unitTestBundle", "platform": "iOS"}
            },
            "schemes": {
                "TestScheme": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "debugEnabled": false,
                        "targets": ["Tests"]
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/TestScheme.xcscheme"),
    )
    .unwrap();
    let test_action = scheme
        .split("<TestAction")
        .nth(1)
        .and_then(|value| value.split("</TestAction>").next())
        .unwrap();
    assert!(xml_contains(test_action, "selectedDebuggerIdentifier=\"\""));
    assert!(xml_contains(
        test_action,
        "selectedLauncherIdentifier=\"Xcode.IDEFoundation.Launcher.PosixSpawn\""
    ));
}

#[test]
fn writer_generates_checker_toggles_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "CheckerToggles",
            "targets": {
                "App": {"type": "application", "platform": "iOS"},
                "Tests": {"type": "unitTestBundle", "platform": "iOS"}
            },
            "schemes": {
                "CheckerScheme": {
                    "build": {"targets": {"App": "all"}},
                    "run": {
                        "config": "Debug",
                        "disableMainThreadChecker": true,
                        "stopOnEveryMainThreadCheckerIssue": true,
                        "disableThreadPerformanceChecker": true
                    },
                    "test": {
                        "config": "Debug",
                        "targets": ["Tests"],
                        "disableMainThreadChecker": true,
                        "stopOnEveryMainThreadCheckerIssue": true
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/CheckerScheme.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(
        &scheme,
        "LaunchAction buildConfiguration=\"Debug\""
    ));
    assert!(xml_contains(&scheme, "disableMainThreadChecker=\"YES\""));
    assert!(xml_contains(
        &scheme,
        "stopOnEveryMainThreadCheckerIssue=\"YES\""
    ));
    assert!(xml_contains(
        &scheme,
        "disableThreadPerformanceChecker=\"YES\""
    ));
    assert!(xml_contains(
        &scheme,
        "TestAction buildConfiguration=\"Debug\""
    ));
}

#[test]
fn writer_generates_screenshot_capture_preferences_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Screenshots",
            "targets": {"App": {"type": "application", "platform": "iOS"}},
            "schemes": {
                "DeleteOnSuccess": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "captureScreenshotsAutomatically": true,
                        "deleteScreenshotsWhenEachTestSucceeds": true
                    }
                },
                "KeepAlways": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "captureScreenshotsAutomatically": true,
                        "deleteScreenshotsWhenEachTestSucceeds": false
                    }
                },
                "KeepNever": {
                    "build": {"targets": {"App": "all"}},
                    "test": {
                        "config": "Debug",
                        "captureScreenshotsAutomatically": false,
                        "deleteScreenshotsWhenEachTestSucceeds": true
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let delete_on_success = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/DeleteOnSuccess.xcscheme"),
    )
    .unwrap();
    let keep_always = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/KeepAlways.xcscheme"),
    )
    .unwrap();
    let keep_never = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/KeepNever.xcscheme"),
    )
    .unwrap();

    assert!(!xml_contains(
        &delete_on_success,
        "systemAttachmentLifetime="
    ));
    assert!(xml_contains(
        &keep_always,
        "systemAttachmentLifetime=\"keepAlways\""
    ));
    assert!(xml_contains(
        &keep_never,
        "systemAttachmentLifetime=\"keepNever\""
    ));
}

#[test]
fn writer_generates_preferred_screen_capture_format_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "ScreenCaptureFormat",
            "targets": {"App": {"type": "application", "platform": "iOS"}},
            "schemes": {
                "Screenshots": {
                    "build": {"targets": {"App": "all"}},
                    "test": {"config": "Debug", "preferredScreenCaptureFormat": "screenshots"}
                },
                "Recording": {
                    "build": {"targets": {"App": "all"}},
                    "test": {"config": "Debug", "preferredScreenCaptureFormat": "screenRecording"}
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let screenshots = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/Screenshots.xcscheme"),
    )
    .unwrap();
    let recording = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/Recording.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(
        &screenshots,
        "preferredScreenCaptureFormat=\"screenshots\""
    ));
    assert!(xml_contains(
        &recording,
        "preferredScreenCaptureFormat=\"screenRecording\""
    ));
}

#[test]
fn writer_uses_last_upgrade_check_for_scheme_last_upgrade_version_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "LastUpgrade",
            "attributes": {"LastUpgradeCheck": "1234"},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {}
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/App.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "LastUpgradeVersion=\"1234\""));
}

#[test]
fn writer_defaults_scheme_last_upgrade_version_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "DefaultLastUpgrade",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "scheme": {}
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/App.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "LastUpgradeVersion=\"1430\""));
}

#[test]
fn writer_generates_predefined_location_simulation_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "PredefinedLocation",
            "targets": {"App": {"type": "application", "platform": "iOS"}},
            "schemes": {
                "Location": {
                    "build": {"targets": {"App": "all"}},
                    "run": {
                        "config": "Debug",
                        "simulateLocation": {
                            "allow": true,
                            "defaultLocation": "New York, NY, USA"
                        }
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/Location.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "LocationScenarioReference"));
    assert!(xml_contains(&scheme, "identifier=\"New York, NY, USA\""));
    assert!(xml_contains(&scheme, "referenceType=\"1\""));
}

#[test]
fn writer_generates_gpx_location_simulation_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "GpxLocation",
            "options": {"schemePathPrefix": "../../"},
            "targets": {"App": {"type": "application", "platform": "iOS"}},
            "schemes": {
                "Location": {
                    "build": {"targets": {"App": "all"}},
                    "run": {
                        "config": "Debug",
                        "simulateLocation": {
                            "allow": true,
                            "defaultLocation": "File.gpx"
                        }
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::write(&project, None).unwrap();
    let scheme = fs::read_to_string(
        generated
            .project_path
            .join("xcshareddata/xcschemes/Location.xcscheme"),
    )
    .unwrap();
    assert!(xml_contains(&scheme, "LocationScenarioReference"));
    assert!(xml_contains(&scheme, "identifier=\"../../File.gpx\""));
    assert!(xml_contains(&scheme, "referenceType=\"0\""));
}
