use oxidegen::{Project, ProjectWriter};
use serde_json::Value;
use std::fs;

fn project_from_json(base_path: std::path::PathBuf, value: Value) -> Project {
    Project::from_dictionary(base_path, value.as_object().unwrap().clone()).unwrap()
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
    assert!(scheme.contains("parallelizeBuildables=\"NO\""));
    assert!(scheme.contains("buildImplicitDependencies=\"NO\""));
    assert!(scheme.contains("BuildableName=\"App.app\""));
    assert!(scheme.contains("BlueprintName=\"App\""));
    assert!(scheme.contains("title=\"Script\""));
    assert!(scheme.contains("scriptText=\"echo Starting\""));
    assert!(scheme.contains("selectedDebuggerIdentifier=\"\""));
    assert!(
        scheme.contains("selectedLauncherIdentifier=\"Xcode.IDEFoundation.Launcher.PosixSpawn\"")
    );
    assert!(scheme.contains("launchAutomaticallySubstyle=\"2\""));
    assert!(scheme.contains("askForAppToLaunch=\"YES\""));
    assert!(scheme.contains("customLLDBInitFile=\"/sample/.lldbinit\""));
    assert!(scheme.contains("useCustomWorkingDirectory=\"YES\""));
    assert!(scheme.contains("customWorkingDirectory=\"/test\""));
    assert!(scheme.contains("enableGPUFrameCaptureMode=\"metal\""));
    assert!(scheme.contains("language=\"en\""));
    assert!(scheme.contains("region=\"US\""));
    assert!(scheme.contains("argument=\"-UITestMode\" isEnabled=\"YES\""));
    assert!(scheme.contains("argument=\"-SkipIntro\" isEnabled=\"NO\""));
    assert!(scheme.contains("customLLDBInitFile=\"/test/.lldbinit\""));
    assert!(scheme.contains("identifier=\"../Configuration.storekit\""));
    assert!(scheme.contains("identifier=\"../test.gpx\" referenceType=\"0\""));
    assert!(scheme.contains("identifier=\"New York, NY, USA\" referenceType=\"1\""));
    assert!(scheme.contains("key=\"RUN_ENV\" value=\"ENABLED\" isEnabled=\"YES\""));
    assert!(scheme.contains("key=\"TEST_ENV\" value=\"1\" isEnabled=\"NO\""));
    assert!(scheme.contains("codeCoverageEnabled=\"YES\""));
    assert!(scheme.contains("reference=\"container:App.xctestplan\" default=\"YES\""));
    assert!(scheme.contains("ProfileAction buildConfiguration=\"Release\""));
    assert!(scheme.contains("askForAppToLaunch=\"YES\""));
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

    assert!(staging.contains("buildConfiguration=\"Staging-Debug\""));
    assert!(staging.contains("buildConfiguration=\"Staging Release\""));
    assert!(production.contains("buildConfiguration=\"Production-Debug\""));
    assert!(production.contains("buildConfiguration=\"Production Release\""));
    assert!(staging.contains("BuildableName=\"Tests.xctest\""));
    assert!(staging.contains("codeCoverageEnabled=\"YES\""));
    assert!(staging.contains("identifier=\"Configuration.storekit\""));
    assert!(staging.contains("language=\"fr\""));
    assert!(staging.contains("region=\"CA\""));
    assert!(staging.contains("argument=\"-TargetScheme\" isEnabled=\"YES\""));
    assert!(staging.contains("key=\"ENV\" value=\"VALUE\" isEnabled=\"YES\""));
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
    assert_eq!(scheme.matches("<MacroExpansion>").count(), 2);
    assert!(scheme.contains("BuildableName=\"MyApp.app\""));
    assert!(scheme.contains("BuildableName=\"MyAppExtension.appex\""));
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
    assert_eq!(scheme.matches("<MacroExpansion>").count(), 2);
    assert!(scheme.contains("BuildableName=\"MyApp.app\""));
    assert!(scheme.contains("BuildableName=\"MockApp.app\""));
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
    assert!(scheme.contains("BlueprintIdentifier=\"XcodeGenKitTests\""));
    assert!(scheme.contains("BuildableName=\"XcodeGenKitTests\""));
    assert!(scheme.contains("BlueprintName=\"XcodeGenKitTests\""));
    assert!(scheme.contains("ReferencedContainer=\"container:../\""));
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
    assert!(breakpoints.contains("Xcode.Breakpoint.ExceptionBreakpoint"));
    assert!(breakpoints.contains("Xcode.Breakpoint.FileBreakpoint"));
    assert!(breakpoints.contains("filePath=\"Sources/App.swift\""));
    assert!(breakpoints.contains("startingLineNumber=\"7\""));
    assert!(breakpoints.contains("startingColumnNumber=\"13\""));
    assert!(breakpoints.contains("condition=\"launchOptions == nil\""));
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
    assert!(launch_action.contains("BuildableName=\"App.app\""));
    assert!(!launch_action.contains("BuildableName=\"Framework.framework\""));
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
    assert!(scheme.contains("BlueprintName=\"ExternalTarget\""));
    assert!(scheme.contains("ReferencedContainer=\"container:ExternalProject.xcodeproj\""));
    assert!(scheme.contains("<CodeCoverageTargets>"));
    assert_eq!(
        scheme.matches("BlueprintName=\"ExternalTarget\"").count(),
        2
    );
    assert!(scheme.contains("BlueprintName=\"Framework\""));
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
    assert!(scheme.contains("<RemoteRunnable runnableDebuggingMode=\"2\">"));
    assert!(scheme.contains("BuildableName=\"WatchApp.app\""));
    assert!(scheme.contains("BuildableName=\"HostApp.app\""));
    assert!(scheme.contains("identifier=\"../Configuration.storekit\""));
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
    assert!(scheme.contains("<PreActions>"));
    assert!(scheme.contains("title=\"Run\""));
    assert!(scheme.contains("scriptText=\"do\""));
    assert!(scheme.contains("<PostActions>"));
    assert!(scheme.contains("title=\"Cleanup\""));
    assert!(scheme.contains("scriptText=\"done\""));
    assert!(scheme.contains("BuildableName=\"App.app\""));
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
            .join("xcuserdata/oxidegen.xcuserdatad/xcschemes/xcschememanagement.plist"),
    )
    .unwrap();
    assert!(management.contains("MyApp.xcscheme_^#shared#^_"));
    assert!(management.contains("<key>isShown</key>"));
    assert!(management.contains("<false />"));
    assert!(!management.contains("<key>orderHint</key>"));
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
    assert!(test_action.contains("selectedDebuggerIdentifier=\"\""));
    assert!(test_action
        .contains("selectedLauncherIdentifier=\"Xcode.IDEFoundation.Launcher.PosixSpawn\""));
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

    assert!(!delete_on_success.contains("systemAttachmentLifetime="));
    assert!(keep_always.contains("systemAttachmentLifetime=\"keepAlways\""));
    assert!(keep_never.contains("systemAttachmentLifetime=\"keepNever\""));
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
    assert!(screenshots.contains("preferredScreenCaptureFormat=\"screenshots\""));
    assert!(recording.contains("preferredScreenCaptureFormat=\"screenRecording\""));
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
    assert!(scheme.contains("LastUpgradeVersion=\"1234\""));
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
    assert!(scheme.contains("LastUpgradeVersion=\"1600\""));
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
    assert!(scheme.contains("LocationScenarioReference"));
    assert!(scheme.contains("identifier=\"New York, NY, USA\""));
    assert!(scheme.contains("referenceType=\"1\""));
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
    assert!(scheme.contains("LocationScenarioReference"));
    assert!(scheme.contains("identifier=\"../../File.gpx\""));
    assert!(scheme.contains("referenceType=\"0\""));
}
