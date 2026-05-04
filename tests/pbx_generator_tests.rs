use serde_json::Value;
use std::fs;
use xcodegenrust::{Project, ProjectWriter};

fn project_from_json(base_path: std::path::PathBuf, value: Value) -> Project {
    Project::from_dictionary(base_path, value.as_object().unwrap().clone()).unwrap()
}

fn main_group_children_block(pbxproj: &str) -> &str {
    let marker = "mainGroup = ";
    let main_group_start = pbxproj.find(marker).unwrap() + marker.len();
    let main_group_id = pbxproj[main_group_start..]
        .split_whitespace()
        .next()
        .unwrap()
        .trim_end_matches(';');
    let object_start = pbxproj.find(&format!("\n\t\t{main_group_id}")).unwrap();
    let children_start = pbxproj[object_start..].find("children = (").unwrap() + object_start;
    let children_end = pbxproj[children_start..].find("\n\t\t\t);").unwrap() + children_start;
    &pbxproj[children_start..children_end]
}

fn group_children_block_with_path<'a>(pbxproj: &'a str, path: &str) -> &'a str {
    let path_marker = format!("path = {path};");
    let object_start = pbxproj
        .find(&format!("/* {path} */ = {{"))
        .and_then(|index| pbxproj[..index].rfind("\n\t\t"))
        .or_else(|| {
            let path_index = pbxproj.find(&path_marker)?;
            let comment_index = pbxproj[..path_index].rfind(" = {")?;
            pbxproj[..comment_index].rfind("\n\t\t")
        })
        .unwrap();
    let children_start = pbxproj[object_start..].find("children = (").unwrap() + object_start;
    let children_end = pbxproj[children_start..].find("\n\t\t\t);").unwrap() + children_start;
    &pbxproj[children_start..children_end]
}

fn assert_names_in_order(block: &str, names: &[&str]) {
    let mut cursor = 0;
    for name in names {
        let index = block[cursor..]
            .find(name)
            .unwrap_or_else(|| panic!("{name} should appear after offset {cursor} in:\n{block}"));
        cursor += index + name.len();
    }
}

#[test]
fn generator_generates_bundle_identifier_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "bundleIdPrefix": "com.test"
            },
            "targets": {
                "MyFramework": {
                    "type": "framework",
                    "platform": "iOS"
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("PRODUCT_BUNDLE_IDENTIFIER = com.test.MyFramework;"));
}

#[test]
fn generator_applies_group_ordering_at_top_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for path in [
        "Configurations/file.swift",
        "Resources/file.swift",
        "Sources/MainScreen/mainScreen1.swift",
        "Sources/MainScreen/mainScreen2.swift",
        "Sources/MainScreen/Assembly/file.swift",
        "Sources/MainScreen/Entities/file.swift",
        "Sources/MainScreen/Interactor/file.swift",
        "Sources/MainScreen/Presenter/file.swift",
        "Sources/MainScreen/View/file.swift",
        "Support files/file.swift",
        "Tests/file.swift",
        "UITests/file.swift",
    ] {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "GroupOrdering",
            "options": {
                "groupSortPosition": "top",
                "groupOrdering": [
                    {"order": ["Sources", "Resources", "Tests", "Support files", "Configurations"]},
                    {
                        "pattern": "^.*Screen$",
                        "order": ["View", "Presenter", "Interactor", "Entities", "Assembly"]
                    }
                ]
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Configurations", "Resources", "Sources", "Support files", "Tests", "UITests"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        main_group_children_block(&generated),
        &[
            "Sources",
            "Resources",
            "Tests",
            "Support files",
            "Configurations",
            "UITests",
            "Products",
        ],
    );
    assert_names_in_order(
        group_children_block_with_path(&generated, "MainScreen"),
        &[
            "View",
            "Presenter",
            "Interactor",
            "Entities",
            "Assembly",
            "mainScreen1.swift",
            "mainScreen2.swift",
        ],
    );
}

#[test]
fn generator_applies_group_ordering_at_bottom_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for path in [
        "Sources/MainScreen/mainScreen1.swift",
        "Sources/MainScreen/mainScreen2.swift",
        "Sources/MainScreen/Assembly/file.swift",
        "Sources/MainScreen/Entities/file.swift",
        "Sources/MainScreen/Interactor/file.swift",
        "Sources/MainScreen/Presenter/file.swift",
        "Sources/MainScreen/View/file.swift",
    ] {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "GroupOrdering",
            "options": {
                "groupSortPosition": "bottom",
                "groupOrdering": [
                    {
                        "pattern": "^.*Screen$",
                        "order": ["View", "Presenter", "Interactor", "Entities", "Assembly"]
                    }
                ]
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        group_children_block_with_path(&generated, "MainScreen"),
        &[
            "mainScreen1.swift",
            "mainScreen2.swift",
            "View",
            "Presenter",
            "Interactor",
            "Entities",
            "Assembly",
        ],
    );
}

#[test]
fn generator_applies_group_ordering_to_local_packages_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for path in [
        "Sources/file.swift",
        "Resources/file.swift",
        "Tests/file.swift",
        "Packages/Common/Package.swift",
        "Packages/FeatureA/Package.swift",
        "Packages/FeatureB/Package.swift",
    ] {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "GroupOrdering",
            "options": {
                "groupSortPosition": "top",
                "groupOrdering": [
                    {"order": ["Sources", "Resources", "Tests", "Packages"]},
                    {"pattern": "Packages", "order": ["FeatureA", "FeatureB", "Common"]}
                ]
            },
            "packages": {
                "Common": {"path": "Packages/Common"},
                "FeatureA": {"path": "Packages/FeatureA"},
                "FeatureB": {"path": "Packages/FeatureB"}
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources", "Resources", "Tests"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        main_group_children_block(&generated),
        &["Sources", "Resources", "Tests", "Packages", "Products"],
    );
    assert_names_in_order(
        group_children_block_with_path(&generated, "Packages"),
        &["FeatureA", "FeatureB", "Common"],
    );
}

#[test]
fn generator_sorts_synced_folders_with_group_ordering_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for path in [
        "Resources/file.swift",
        "Sources/file.swift",
        "SyncedSources/file.swift",
    ] {
        let path = temp.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "").unwrap();
    }
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "GroupOrdering",
            "options": {
                "groupSortPosition": "top",
                "groupOrdering": [
                    {"order": ["Sources", "SyncedSources", "Resources"]}
                ]
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources",
                        {"path": "SyncedSources", "type": "syncedFolder"},
                        "Resources"
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        main_group_children_block(&generated),
        &["Sources", "SyncedSources", "Resources", "Products"],
    );
}

#[test]
fn generator_generates_development_language_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "developmentLanguage": "de"
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("developmentRegion = de;"));
    assert!(generated.pbxproj.contains("knownRegions = ("));
    assert!(generated.pbxproj.contains("\t\t\t\tde,"));
}

#[test]
fn generator_uses_default_configuration_name_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "defaultConfig": "Bconfig"
            },
            "configs": {
                "Aconfig": "debug",
                "Bconfig": "release"
            },
            "targets": {
                "One": {
                    "type": "framework",
                    "platform": "iOS"
                },
                "Two": {
                    "type": "framework",
                    "platform": "iOS"
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("defaultConfigurationName = Bconfig;")
            .count(),
        3
    );
}

#[test]
fn generator_applies_partial_config_settings_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Generated",
            "configs": {
                "Release": "release",
                "Staging Debug": "debug",
                "Staging Release": "release"
            },
            "settings": {
                "configs": {
                    "staging": {"SETTING1": "VALUE1"},
                    "debug": {"SETTING2": "VALUE2"},
                    "Release": {"SETTING3": "VALUE3"}
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("name = \"Staging Debug\";"));
    assert!(generated.contains("SETTING1 = VALUE1;"));
    assert!(generated.contains("SETTING2 = VALUE2;"));
    assert!(generated.contains("name = \"Staging Release\";"));
    assert_eq!(generated.matches("SETTING3 = VALUE3;").count(), 1);
}

#[test]
fn generator_sets_project_sdkroot_for_single_platform_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS"
                },
                "Framework": {
                    "type": "framework",
                    "platform": "iOS"
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("SDKROOT = iphoneos;"));
}

#[test]
fn generator_sets_platform_deployment_targets_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "deploymentTarget": {
                    "iOS": "10.0",
                    "watchOS": "3.0"
                }
            },
            "targets": {
                "WatchApp": {
                    "type": "watch2App",
                    "platform": "watchOS",
                    "deploymentTarget": "2.0"
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("IPHONEOS_DEPLOYMENT_TARGET = 10.0;")
            .count(),
        2
    );
    assert_eq!(
        generated
            .pbxproj
            .matches("WATCHOS_DEPLOYMENT_TARGET = 3.0;")
            .count(),
        2
    );
    assert_eq!(
        generated
            .pbxproj
            .matches("WATCHOS_DEPLOYMENT_TARGET = 2.0;")
            .count(),
        2
    );
    assert!(!generated.pbxproj.contains("TVOS_DEPLOYMENT_TARGET"));
}

#[test]
fn generator_sets_supported_destinations_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "auto",
                    "supportedDestinations": ["tvOS", "iOS"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("SDKROOT = auto;"));
    assert!(generated.pbxproj.contains(
        "SUPPORTED_PLATFORMS = \"iphoneos iphonesimulator appletvos appletvsimulator\";"
    ));
    assert!(generated
        .pbxproj
        .contains("TARGETED_DEVICE_FAMILY = \"1,2,3\";"));
}

#[test]
fn generator_merges_supported_destination_presets_like_xcodegen() {
    let cases = [
        (
            &["iOS", "visionOS"][..],
            "application",
            "iphoneos iphonesimulator xros xrsimulator",
            "1,2,7",
            false,
            true,
            Some(false),
            Some("AppIcon"),
            None,
            true,
        ),
        (
            &["iOS", "tvOS", "macOS"][..],
            "application",
            "iphoneos iphonesimulator appletvos appletvsimulator macosx",
            "1,2,3",
            false,
            false,
            Some(true),
            Some("AppIcon"),
            None,
            true,
        ),
        (
            &["iOS", "tvOS", "macCatalyst"][..],
            "application",
            "iphoneos iphonesimulator appletvos appletvsimulator",
            "1,2,3",
            true,
            false,
            Some(true),
            Some("AppIcon"),
            None,
            true,
        ),
        (
            &["iOS", "macOS"][..],
            "application",
            "iphoneos iphonesimulator macosx",
            "1,2",
            false,
            false,
            Some(true),
            Some("AppIcon"),
            None,
            true,
        ),
        (
            &["tvOS", "macOS"][..],
            "application",
            "appletvos appletvsimulator macosx",
            "3",
            false,
            false,
            None,
            Some("App Icon & Top Shelf Image"),
            Some("LaunchImage"),
            false,
        ),
        (
            &["visionOS", "macOS"][..],
            "application",
            "xros xrsimulator macosx",
            "7",
            false,
            false,
            Some(false),
            Some("AppIcon"),
            None,
            false,
        ),
        (
            &["iOS", "macCatalyst"][..],
            "application",
            "iphoneos iphonesimulator",
            "1,2",
            true,
            false,
            Some(true),
            Some("AppIcon"),
            None,
            true,
        ),
        (
            &["iOS", "watchOS"][..],
            "framework",
            "iphoneos iphonesimulator watchos watchsimulator",
            "1,2,4",
            false,
            true,
            Some(true),
            None,
            None,
            false,
        ),
        (
            &["visionOS", "watchOS"][..],
            "framework",
            "watchos watchsimulator xros xrsimulator",
            "4,7",
            false,
            false,
            Some(false),
            None,
            None,
            false,
        ),
    ];

    for (
        destinations,
        target_type,
        supported_platforms,
        device_family,
        supports_mac_catalyst,
        supports_mac_designed,
        supports_xr_designed,
        app_icon,
        launch_image,
        code_sign_identity,
    ) in cases
    {
        let temp = tempfile::TempDir::new().unwrap();
        let project = project_from_json(
            temp.path().to_path_buf(),
            serde_json::json!({
                "name": "Generated",
                "targets": {
                    "Target": {
                        "type": target_type,
                        "platform": "auto",
                        "supportedDestinations": destinations
                    }
                }
            }),
        );
        let generated = ProjectWriter::generate(&project).unwrap().pbxproj;

        assert!(generated.contains(&format!("SUPPORTED_PLATFORMS = \"{supported_platforms}\";")));
        if device_family.contains(',') {
            assert!(generated.contains(&format!("TARGETED_DEVICE_FAMILY = \"{device_family}\";")));
        } else {
            assert!(generated.contains(&format!("TARGETED_DEVICE_FAMILY = {device_family};")));
        }
        assert!(generated.contains(&format!(
            "SUPPORTS_MACCATALYST = {};",
            if supports_mac_catalyst { "YES" } else { "NO" }
        )));
        assert!(generated.contains(&format!(
            "SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD = {};",
            if supports_mac_designed { "YES" } else { "NO" }
        )));
        if let Some(supports_xr_designed) = supports_xr_designed {
            assert!(generated.contains(&format!(
                "SUPPORTS_XR_DESIGNED_FOR_IPHONE_IPAD = {};",
                if supports_xr_designed { "YES" } else { "NO" }
            )));
        }
        if let Some(app_icon) = app_icon {
            if app_icon.contains(' ') {
                assert!(generated.contains(&format!(
                    "ASSETCATALOG_COMPILER_APPICON_NAME = \"{app_icon}\";"
                )));
            } else {
                assert!(generated
                    .contains(&format!("ASSETCATALOG_COMPILER_APPICON_NAME = {app_icon};")));
            }
        }
        if let Some(launch_image) = launch_image {
            assert!(generated.contains(&format!(
                "ASSETCATALOG_COMPILER_LAUNCHIMAGE_NAME = {launch_image};"
            )));
        }
        if code_sign_identity {
            assert!(generated.contains("CODE_SIGN_IDENTITY = \"iPhone Developer\";"));
        }
    }
}

#[test]
fn generator_respects_setting_presets_none_for_supported_destinations_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "settingPresets": "none"
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "auto",
                    "supportedDestinations": ["iOS", "macOS"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("SDKROOT = auto;"));
    assert!(!generated.contains("SUPPORTED_PLATFORMS"));
    assert!(!generated.contains("TARGETED_DEVICE_FAMILY"));
    assert!(!generated.contains("SUPPORTS_MACCATALYST"));
    assert!(!generated.contains("SUPPORTS_MAC_DESIGNED_FOR_IPHONE_IPAD"));
    assert!(!generated.contains("SUPPORTS_XR_DESIGNED_FOR_IPHONE_IPAD"));
}

#[test]
fn generator_clears_setting_presets_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Generated",
            "options": {
                "settingPresets": "none"
            },
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "settings": {
                        "SETTING_2": "VALUE"
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("SDKROOT = iphoneos;"));
    assert!(generated.contains("SETTING_2 = VALUE;"));
    assert!(!generated.contains("TARGETED_DEVICE_FAMILY"));
    assert!(!generated.contains("ASSETCATALOG_COMPILER_APPICON_NAME"));
    assert!(!generated.contains("CODE_SIGN_IDENTITY"));
}

#[test]
fn generator_adds_files_to_correct_build_phases_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/App.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/ViewController.m"), "").unwrap();
    fs::write(temp.path().join("Sources/Assets.xcassets"), "").unwrap();
    fs::write(temp.path().join("Sources/Info.plist"), "").unwrap();
    fs::write(temp.path().join("Sources/Notes.md"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("App.swift in Sources"));
    assert!(generated.pbxproj.contains("ViewController.m in Sources"));
    assert!(generated.pbxproj.contains("Assets.xcassets in Resources"));
    assert!(generated
        .pbxproj
        .contains("INFOPLIST_FILE = Sources/Info.plist;"));
    assert!(!generated.pbxproj.contains("/* Info.plist in Resources */"));
    assert!(generated.pbxproj.contains("Notes.md in Resources"));
    assert!(!generated.pbxproj.contains("Notes.md in Sources"));
}

#[test]
fn generator_supports_frameworks_in_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/Foo.framework"), "").unwrap();
    fs::write(temp.path().join("Sources/Bar.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("Bar.swift in Sources"));
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated.pbxproj.contains("Foo.framework in CopyFiles"));
    assert!(!generated.pbxproj.contains("Foo.framework in Sources"));
    assert!(!generated.pbxproj.contains("Foo.framework in Resources"));
}

#[test]
fn generator_emits_synced_folder_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::write(temp.path().join("Sources/A/a.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "type": "syncedFolder"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("isa = PBXFileSystemSynchronizedRootGroup;"));
    assert!(generated
        .pbxproj
        .contains("fileSystemSynchronizedGroups = ("));
    assert!(generated.pbxproj.contains("path = Sources;"));
    assert!(generated.pbxproj.contains("/* Resources */"));
    assert!(!generated.pbxproj.contains("a.swift in Sources"));
}

#[test]
fn generator_emits_source_groups_in_main_navigator_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Nested")).unwrap();
    fs::write(temp.path().join("Sources/App.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Nested/Feature.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("/* Sources */ = {"));
    assert!(generated.pbxproj.contains("path = Sources;"));
    assert!(generated.pbxproj.contains("path = Nested;"));
    assert!(generated.pbxproj.contains("path = App.swift;"));
    assert!(generated.pbxproj.contains("path = Feature.swift;"));
}

#[test]
fn generator_emits_intermediate_source_groups_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/F/G")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/B")).unwrap();
    fs::write(temp.path().join("Sources/A/b.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/F/G/h.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/B/b.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "options": {"createIntermediateGroups": true},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources/A/b.swift",
                        "Sources/F/G/h.swift",
                        {"path": "Sources/B", "createIntermediateGroups": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("path = Sources;"));
    assert!(generated.contains("path = A;"));
    assert!(generated.contains("path = F;"));
    assert!(generated.contains("path = G;"));
    assert!(generated.contains("path = h.swift;"));
    assert!(generated.contains("path = Sources/B;"));
}

#[test]
fn generator_emits_custom_source_groups_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/F/G")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/B/C")).unwrap();
    fs::write(temp.path().join("Sources/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/A/b.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/F/G/h.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/F/G/i.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/B/b.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/B/C/c.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "options": {"createIntermediateGroups": true},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Sources/a.swift", "group": "CustomGroup1"},
                        {"path": "Sources/A/b.swift", "group": "CustomGroup1"},
                        {"path": "Sources/F/G/h.swift", "group": "CustomGroup1"},
                        {"path": "Sources/B", "group": "CustomGroup2", "createIntermediateGroups": false},
                        {"path": "Sources/F/G/i.swift", "group": "Sources/F/G/CustomGroup3"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("path = CustomGroup1;"));
    assert!(generated.contains("path = Sources/a.swift;"));
    assert!(generated.contains("path = Sources/A/b.swift;"));
    assert!(generated.contains("path = Sources/F/G/h.swift;"));
    assert!(generated.contains("path = CustomGroup2;"));
    assert!(generated.contains("path = Sources/B;"));
    assert!(generated.contains("path = CustomGroup3;"));
    assert!(generated.contains("path = i.swift;"));
}

#[test]
fn generator_emits_folder_references_with_intermediate_groups_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::write(temp.path().join("Sources/A/a.resource"), "").unwrap();
    fs::write(temp.path().join("Sources/A/b.resource"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "options": {"createIntermediateGroups": true},
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources/A", "type": "folder"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("path = Sources;"));
    assert!(generated.contains("path = Sources/A;"));
    assert!(generated.contains("lastKnownFileType = folder;"));
    assert!(!generated.contains("a.resource in Resources"));
}

#[test]
fn generator_keeps_distinct_source_groups_with_the_same_display_name_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("A")).unwrap();
    fs::create_dir(temp.path().join("B")).unwrap();
    fs::create_dir(temp.path().join("Source")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/group")).unwrap();
    fs::create_dir_all(temp.path().join("Z/A")).unwrap();
    fs::write(temp.path().join("A/A.swift"), "").unwrap();
    fs::write(temp.path().join("B/file.swift"), "").unwrap();
    fs::write(temp.path().join("Source/file.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/group/file.swift"), "").unwrap();
    fs::write(temp.path().join("Z/A/file.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources",
                        {"path": "Source", "name": "S"},
                        "A",
                        {"path": "Z/A", "name": "B"},
                        "B"
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    let main_children = main_group_children_block(&generated);
    assert_eq!(main_children.matches("/* B */").count(), 2);
    assert!(generated.contains("path = B;"));
    assert!(generated.contains("path = Z/A;"));
    assert!(generated.contains("name = S;"));
    assert!(generated.contains("path = Source;"));
}

#[test]
fn generator_groups_relative_sources_outside_base_path_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Inside/Inside2")).unwrap();
    fs::write(temp.path().join("Sources/Inside/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Inside/Inside2/b.swift"), "").unwrap();

    let outside = temp.path().parent().unwrap().join("OtherDirectory");
    let _ = fs::remove_dir_all(&outside);
    fs::create_dir_all(outside.join("Outside/Outside2")).unwrap();
    fs::write(outside.join("Outside/a.swift"), "").unwrap();
    fs::write(outside.join("Outside/Outside2/b.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources",
                        "../OtherDirectory"
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("path = Sources;"));
    assert!(generated.contains("path = Inside;"));
    assert!(generated.contains("path = Inside2;"));
    assert!(generated.contains("path = ../OtherDirectory;"));
    assert!(generated.contains("path = Outside;"));
    assert!(generated.contains("path = Outside2;"));
}

#[test]
fn generator_respects_default_source_directory_type_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::write(temp.path().join("Sources/A/a.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "options": {
                "defaultSourceDirectoryType": "syncedFolder"
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("isa = PBXFileSystemSynchronizedRootGroup;"));
    assert!(generated
        .pbxproj
        .contains("fileSystemSynchronizedGroups = ("));
    assert!(!generated.pbxproj.contains("a.swift in Sources"));
}

#[test]
fn generator_deduplicates_synced_folders_across_targets_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/a.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "type": "syncedFolder"}]
                },
                "Tests": {
                    "type": "unit-test",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "type": "syncedFolder"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXFileSystemSynchronizedRootGroup;")
            .count(),
        1
    );
    assert_eq!(
        generated
            .pbxproj
            .matches("fileSystemSynchronizedGroups = (")
            .count(),
        2
    );
}

#[test]
fn generator_merges_synced_folder_explicit_folders_across_targets_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/FolderA")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/FolderB")).unwrap();
    fs::write(temp.path().join("Sources/FolderA/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/FolderB/b.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "Target1": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "type": "syncedFolder", "explicitFolders": ["FolderA"]}]
                },
                "Target2": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "type": "syncedFolder", "explicitFolders": ["FolderB"]}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXFileSystemSynchronizedRootGroup;")
            .count(),
        1
    );
    assert!(generated.pbxproj.contains("FolderA,"));
    assert!(generated.pbxproj.contains("FolderB,"));
}

#[test]
fn generator_adds_synced_folder_membership_exceptions_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Generated")).unwrap();
    fs::write(temp.path().join("Sources/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/b.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Info.plist"), "").unwrap();
    fs::write(temp.path().join("Sources/Generated/c.generated.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Generated/d.generated.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "settings": {"INFOPLIST_FILE": "Sources/Info.plist"},
                    "sources": [{
                        "path": "Sources",
                        "type": "syncedFolder",
                        "excludes": ["b.swift", "Generated/*.generated.swift"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("isa = PBXFileSystemSynchronizedBuildFileExceptionSet;"));
    assert!(generated.pbxproj.contains("membershipExceptions = ("));
    assert!(generated.pbxproj.contains("b.swift,"));
    assert!(generated.pbxproj.contains("Generated/c.generated.swift,"));
    assert!(generated.pbxproj.contains("Generated/d.generated.swift,"));
    assert!(generated.pbxproj.contains("Info.plist,"));
    assert!(!generated.pbxproj.contains("a.swift,"));
}

#[test]
fn generator_adds_synced_folder_include_exceptions_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Nested")).unwrap();
    fs::write(temp.path().join("Sources/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Nested/b.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Nested/c.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "type": "syncedFolder",
                        "includes": ["Nested/b.swift"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("a.swift,"));
    assert!(generated.pbxproj.contains("Nested/c.swift,"));
    assert!(!generated.pbxproj.contains("Nested/b.swift,"));
}

#[test]
fn generator_keeps_separate_synced_folder_exceptions_per_target_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/target1.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/target2.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/common.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "Target1": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "type": "syncedFolder",
                        "includes": ["target1.swift", "common.swift"]
                    }]
                },
                "Target2": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "type": "syncedFolder",
                        "includes": ["target2.swift", "common.swift"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXFileSystemSynchronizedBuildFileExceptionSet;")
            .count(),
        2
    );
    assert!(generated.pbxproj.contains("target1.swift,"));
    assert!(generated.pbxproj.contains("target2.swift,"));
    assert!(generated.pbxproj.contains("target = "));
    assert!(generated.pbxproj.contains("/* Target1 */"));
    assert!(generated.pbxproj.contains("/* Target2 */"));
}

#[test]
fn generator_expands_synced_folder_explicit_folders_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Images")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/MainSuite/FeatureATests")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/MainSuite/FeatureBTests")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/NotATest")).unwrap();
    fs::write(temp.path().join("Sources/Images/image.png"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SyncedFolders",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "type": "syncedFolder",
                        "explicitFolders": ["Images", "**/*Tests"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("explicitFolders = ("));
    assert!(generated.pbxproj.contains("Images,"));
    assert!(generated.pbxproj.contains("MainSuite/FeatureATests,"));
    assert!(generated.pbxproj.contains("MainSuite/FeatureBTests,"));
    assert!(!generated.pbxproj.contains("NotATest,"));
}

#[test]
fn generator_treats_core_data_mapping_models_as_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::create_dir(temp.path().join("Sources/model.xcmappingmodel")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/model.xcmappingmodel/xcmapping.xml"),
        "",
    )
    .unwrap();
    fs::create_dir(temp.path().join("Sources/model.xcdatamodeld")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/model.xcdatamodeld/model.xcdatamodel"),
        "",
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("model.xcmappingmodel in Sources"));
    assert!(generated.pbxproj.contains("model.xcdatamodeld in Sources"));
}

#[test]
fn generator_deduplicates_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("A.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["A.swift", "A.swift"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(generated.pbxproj.matches("A.swift in Sources").count(), 2);
}

#[test]
fn generator_renames_explicit_file_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("OtherSource")).unwrap();
    fs::write(temp.path().join("OtherSource/b.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "OtherSource/b.swift", "name": "c.swift"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("c.swift in Sources"));
    assert!(generated.pbxproj.contains("name = c.swift;"));
    assert!(generated.pbxproj.contains("path = OtherSource/b.swift;"));
    assert!(!generated.pbxproj.contains("b.swift in Sources"));
}

#[test]
fn generator_excludes_default_ignored_files_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::write(temp.path().join("Sources/A/a.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/A/.DS_Store"), "").unwrap();
    fs::write(temp.path().join("Sources/A/a.swift.orig"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("a.swift in Sources"));
    assert!(!generated.pbxproj.contains(".DS_Store"));
    assert!(!generated.pbxproj.contains("a.swift.orig"));
}

#[test]
fn generator_supports_bracket_globs_in_source_excludes_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/types")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/numbers")).unwrap();
    for file in ["a.swift", "a.m", "a.h", "a.x"] {
        fs::write(temp.path().join("Sources/types").join(file), "").unwrap();
    }
    for file in ["file1.a", "file2.a", "file3.a", "file4.a"] {
        fs::write(temp.path().join("Sources/numbers").join(file), "").unwrap();
    }

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "excludes": ["types/*.[hx]", "numbers/file[2-3].a"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("a.swift in Sources"));
    assert!(generated.pbxproj.contains("a.m in Sources"));
    assert!(generated.pbxproj.contains("file1.a in Resources"));
    assert!(generated.pbxproj.contains("file4.a in Resources"));
    assert!(!generated.pbxproj.contains("a.h in Headers"));
    assert!(!generated.pbxproj.contains("a.x in Resources"));
    assert!(!generated.pbxproj.contains("file2.a in Resources"));
    assert!(!generated.pbxproj.contains("file3.a in Resources"));
}

#[test]
fn generator_emits_folder_reference_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/A")).unwrap();
    fs::write(temp.path().join("Sources/A/a.resource"), "").unwrap();
    fs::write(temp.path().join("Sources/A/b.resource"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources/A", "type": "folder"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("A in Resources"));
    assert!(generated.pbxproj.contains("lastKnownFileType = folder;"));
    assert!(generated.pbxproj.contains("path = Sources/A;"));
    assert!(!generated.pbxproj.contains("a.resource in Resources"));
    assert!(!generated.pbxproj.contains("b.resource in Resources"));
}

#[test]
fn generator_adds_missing_optional_files_and_folders_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        {"path": "File1.swift", "optional": true},
                        {"path": "File2.swift", "type": "file", "optional": true},
                        {"path": "Group", "type": "folder", "optional": true}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("File1.swift in Sources"));
    assert!(generated.pbxproj.contains("File2.swift in Sources"));
    assert!(generated.pbxproj.contains("Group in Resources"));
}

#[test]
fn generator_allows_missing_optional_groups_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Group1", "optional": true},
                        {"path": "Group2", "type": "group", "optional": true},
                        {"path": "Group3", "type": "group", "optional": true}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(!generated.pbxproj.contains("path = Group1;"));
    assert!(!generated.pbxproj.contains("path = Group2;"));
    assert!(!generated.pbxproj.contains("path = Group3;"));
}

#[test]
fn generator_includes_only_matching_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/file3.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file3Tests.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file2.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file2Tests.swift"), "").unwrap();
    fs::create_dir(temp.path().join("Sources/group2")).unwrap();
    fs::write(temp.path().join("Sources/group2/file.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/group2/fileTests.swift"), "").unwrap();
    fs::create_dir_all(temp.path().join("Sources/group3/group4/group5")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/group3/group4/group5/file5Tests.swift"),
        "",
    )
    .unwrap();
    fs::write(
        temp.path()
            .join("Sources/group3/group4/group5/file6Tests.m"),
        "",
    )
    .unwrap();
    fs::write(
        temp.path()
            .join("Sources/group3/group4/group5/file6Tests.h"),
        "",
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "includes": ["**/*Tests.*"]}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("file2Tests.swift in Sources"));
    assert!(generated.pbxproj.contains("file3Tests.swift in Sources"));
    assert!(generated.pbxproj.contains("fileTests.swift in Sources"));
    assert!(generated.pbxproj.contains("file5Tests.swift in Sources"));
    assert!(generated.pbxproj.contains("file6Tests.m in Sources"));
    assert!(!generated.pbxproj.contains("file2.swift in Sources"));
    assert!(!generated.pbxproj.contains("file3.swift in Sources"));
}

#[test]
fn generator_handles_includes_with_no_matches_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/file3.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file3Tests.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file2.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file2Tests.swift"), "").unwrap();
    fs::create_dir(temp.path().join("Sources/group2")).unwrap();
    fs::write(temp.path().join("Sources/group2/file.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/group2/fileTests.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "includes": ["**/*NonExistent.*"]}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(!generated.pbxproj.contains("file2.swift in Sources"));
    assert!(!generated.pbxproj.contains("file3.swift in Sources"));
    assert!(!generated.pbxproj.contains("file2Tests.swift in Sources"));
    assert!(!generated.pbxproj.contains("file3Tests.swift in Sources"));
    assert!(!generated.pbxproj.contains("fileTests.swift in Sources"));
}

#[test]
fn generator_prioritizes_excludes_over_includes_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/file3Tests.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/file2Tests.swift"), "").unwrap();
    fs::create_dir(temp.path().join("Sources/group2")).unwrap();
    fs::write(temp.path().join("Sources/group2/fileTests.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{
                        "path": "Sources",
                        "includes": ["**/*Tests.*"],
                        "excludes": ["group2"]
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("file2Tests.swift in Sources"));
    assert!(generated.pbxproj.contains("file3Tests.swift in Sources"));
    assert!(!generated.pbxproj.contains("fileTests.swift in Sources"));
}

#[test]
fn generator_places_resources_before_sources_when_requested_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("App.swift"), "").unwrap();
    fs::write(temp.path().join("Image.png"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["App.swift", "Image.png"],
                    "putResourcesBeforeSourcesBuildPhase": true
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    let resources_index = generated
        .pbxproj
        .find("/* Resources */")
        .expect("resources phase should exist");
    let sources_index = generated
        .pbxproj
        .find("/* Sources */")
        .expect("sources phase should exist");
    assert!(
        resources_index < sources_index,
        "resources build phase should be listed before sources"
    );
}

#[test]
fn generator_emits_target_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Dependencies",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "Framework"}]
                },
                "Framework": {
                    "type": "framework",
                    "platform": "iOS"
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXTargetDependency;"));
    assert!(generated.pbxproj.contains("remoteInfo = Framework;"));
}

#[test]
fn generator_handles_cyclical_target_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CyclicalDependencies",
            "targets": {
                "target1": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [{"target": "target2"}]
                },
                "target2": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [{"target": "target1"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("remoteInfo = target1;"));
    assert!(generated.pbxproj.contains("remoteInfo = target2;"));
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXTargetDependency;")
            .count(),
        2
    );
}

#[test]
fn generator_sets_products_group_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Products",
            "targets": {
                "TestApp": {"type": "application", "platform": "iOS"},
                "TestFramework": {"type": "framework", "platform": "iOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("/* Products */"));
    assert!(generated.pbxproj.contains("path = TestApp.app;"));
    assert!(generated
        .pbxproj
        .contains("path = TestFramework.framework;"));
    assert!(generated.pbxproj.contains("productRefGroup"));
}

#[test]
fn generator_sets_empty_products_group_without_targets_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({"name": "Products"}),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("/* Products */"));
    assert!(generated.pbxproj.contains("productRefGroup"));
}

#[test]
fn generator_sets_last_upgrade_check_like_xcodegen() {
    let default_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({"name": "Upgrade"}),
    );
    let default_generated = ProjectWriter::generate(&default_project).unwrap();
    assert!(default_generated
        .pbxproj
        .contains("LastUpgradeCheck = 1430;"));

    let overridden_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Upgrade",
            "attributes": {"LastUpgradeCheck": "1234"}
        }),
    );
    let overridden_generated = ProjectWriter::generate(&overridden_project).unwrap();
    assert!(overridden_generated
        .pbxproj
        .contains("LastUpgradeCheck = 1234;"));

    let invalid_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Upgrade",
            "attributes": {"LastUpgradeCheck": 1234}
        }),
    );
    let invalid_generated = ProjectWriter::generate(&invalid_project).unwrap();
    assert!(invalid_generated
        .pbxproj
        .contains("LastUpgradeCheck = 1430;"));
}

#[test]
fn generator_emits_target_attributes_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "TargetAttributes",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "settings": {
                        "DEVELOPMENT_TEAM": "123"
                    },
                    "attributes": {
                        "ProvisioningStyle": "Automatic"
                    }
                },
                "Framework": {
                    "type": "framework",
                    "platform": "iOS"
                },
                "AppTests": {
                    "type": "unit-test",
                    "platform": "iOS",
                    "settings": {
                        "CODE_SIGN_STYLE": "Manual"
                    },
                    "dependencies": [{"target": "App"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("TargetAttributes = {"));
    assert!(generated.pbxproj.contains("DevelopmentTeam = 123;"));
    assert!(generated.pbxproj.contains("ProvisioningStyle = Automatic;"));
    assert!(generated.pbxproj.contains("ProvisioningStyle = Manual;"));
    assert!(generated.pbxproj.contains("/* App */"));
}

#[test]
fn writer_generates_info_plist_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "info": {
                        "path": "Info.plist",
                        "properties": {
                            "UISupportedInterfaceOrientations": [
                                "UIInterfaceOrientationPortrait",
                                "UIInterfaceOrientationLandscapeLeft"
                            ]
                        }
                    }
                }
            }
        }),
    );

    let generated =
        ProjectWriter::write(&project, Some(&temp.path().join("Generated.xcodeproj"))).unwrap();
    assert!(generated.pbxproj.contains("INFOPLIST_FILE = Info.plist;"));

    let plist = fs::read_to_string(temp.path().join("Info.plist")).unwrap();
    assert!(plist.contains("<key>CFBundleIdentifier</key>"));
    assert!(plist.contains("<string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>"));
    assert!(plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<string>$(EXECUTABLE_NAME)</string>"));
    assert!(plist.contains("<key>CFBundlePackageType</key>"));
    assert!(plist.contains("<string>APPL</string>"));
    assert!(plist.contains("<key>UISupportedInterfaceOrientations</key>"));
    assert!(plist.contains("<string>UIInterfaceOrientationLandscapeLeft</string>"));
}

#[test]
fn generator_does_not_override_explicit_info_plist_setting_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "settings": {"INFOPLIST_FILE": "Predefined.plist"},
                    "info": {
                        "path": "Info.plist",
                        "properties": {"CFBundleName": "GeneratedName"}
                    }
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("INFOPLIST_FILE = Predefined.plist;"));
    assert!(!generated.pbxproj.contains("INFOPLIST_FILE = Info.plist;"));
}

#[test]
fn writer_generates_bundle_info_plist_without_executable_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "Resources": {
                    "type": "bundle",
                    "platform": "iOS",
                    "info": {"path": "Info.plist"}
                }
            }
        }),
    );

    let generated =
        ProjectWriter::write(&project, Some(&temp.path().join("Generated.xcodeproj"))).unwrap();
    assert!(generated.pbxproj.contains("INFOPLIST_FILE = Info.plist;"));

    let plist = fs::read_to_string(temp.path().join("Info.plist")).unwrap();
    assert!(!plist.contains("<key>CFBundleExecutable</key>"));
    assert!(plist.contains("<key>CFBundlePackageType</key>"));
    assert!(plist.contains("<string>BNDL</string>"));
}

#[test]
fn writer_generates_entitlements_and_build_setting_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "entitlements": {
                        "path": "App.entitlements",
                        "properties": {
                            "com.apple.security.app-sandbox": true
                        }
                    }
                }
            }
        }),
    );

    let generated =
        ProjectWriter::write(&project, Some(&temp.path().join("Generated.xcodeproj"))).unwrap();
    assert!(generated
        .pbxproj
        .contains("CODE_SIGN_ENTITLEMENTS = App.entitlements;"));

    let plist = fs::read_to_string(temp.path().join("App.entitlements")).unwrap();
    assert!(plist.contains("<key>com.apple.security.app-sandbox</key>"));
    assert!(plist.contains("<true/>"));
}

#[test]
fn generator_omits_configured_info_plist_from_resources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/App.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Info.plist"), "").unwrap();
    fs::write(temp.path().join("Sources/GoogleService-Info.plist"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Generated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"],
                    "info": {"path": "Sources/Info.plist"}
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("App.swift in Sources"));
    assert!(generated.pbxproj.contains("Info.plist"));
    assert!(!generated.pbxproj.contains("/* Info.plist in Resources */"));
    assert!(generated
        .pbxproj
        .contains("GoogleService-Info.plist in Resources"));
}

#[test]
fn generator_emits_run_script_phases_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Scripts",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "preBuildScripts": [{"script": "script1"}],
                    "postCompileScripts": [{"script": "script2"}],
                    "postBuildScripts": [
                        {"script": "script3"},
                        {
                            "script": "script4",
                            "discoveredDependencyFile": "$(DERIVED_FILE_DIR)/target.d",
                            "basedOnDependencyAnalysis": false,
                            "showEnvVars": false
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXShellScriptBuildPhase;")
            .count(),
        4
    );
    assert!(generated.pbxproj.contains("shellScript = script1;"));
    assert!(generated.pbxproj.contains("shellScript = script2;"));
    assert!(generated.pbxproj.contains("shellScript = script3;"));
    assert!(generated.pbxproj.contains("shellScript = script4;"));
    assert!(generated
        .pbxproj
        .contains("dependencyFile = \"$(DERIVED_FILE_DIR)/target.d\";"));
    assert!(generated.pbxproj.contains("alwaysOutOfDate = 1;"));
    assert!(generated.pbxproj.contains("showEnvVarsInLog = 0;"));

    assert!(generated.pbxproj.contains("/* Run Script */"));
}

#[test]
fn generator_emits_build_rules_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Rules",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "buildRules": [
                        {
                            "fileType": "sourcecode.swift",
                            "script": "do thing",
                            "name": "My Rule",
                            "outputFiles": ["file1.swift", "file2.swift"],
                            "outputFilesCompilerFlags": ["--zee", "--bee"]
                        },
                        {
                            "filePattern": "*.plist",
                            "compilerSpec": "com.apple.build-tasks.copy-plist-file"
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(generated.pbxproj.matches("isa = PBXBuildRule;").count(), 2);
    assert!(generated.pbxproj.contains("name = \"My Rule\";"));
    assert!(generated
        .pbxproj
        .contains("compilerSpec = com.apple.compilers.proxy.script;"));
    assert!(generated.pbxproj.contains("fileType = sourcecode.swift;"));
    assert!(generated.pbxproj.contains("script = \"do thing\";"));
    assert!(generated.pbxproj.contains("file1.swift"));
    assert!(generated.pbxproj.contains("--zee"));
    assert!(generated.pbxproj.contains("name = \"Build Rule\";"));
    assert!(generated.pbxproj.contains("fileType = pattern.proxy;"));
    assert!(generated.pbxproj.contains("filePatterns = \"*.plist\";"));
    assert!(generated
        .pbxproj
        .contains("compilerSpec = \"com.apple.build-tasks.copy-plist-file\";"));
}

#[test]
fn generator_emits_aggregate_target_dependencies_and_scripts_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Aggregates",
            "targets": {
                "MyApp": {"type": "application", "platform": "iOS"},
                "MyFramework": {"type": "framework", "platform": "iOS"},
                "Other": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [{"target": "AggregateTarget"}]
                },
                "Other2": {
                    "type": "framework",
                    "platform": "iOS",
                    "transitivelyLinkDependencies": true,
                    "dependencies": [{"target": "Other"}]
                }
            },
            "aggregateTargets": {
                "AggregateTarget": {
                    "targets": ["MyApp", "MyFramework"],
                    "buildScripts": [{"script": "echo aggregate"}]
                },
                "AggregateTarget2": {
                    "targets": ["AggregateTarget"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXAggregateTarget;")
            .count(),
        2
    );
    assert!(generated.pbxproj.contains("remoteInfo = MyApp;"));
    assert!(generated.pbxproj.contains("remoteInfo = MyFramework;"));
    assert!(generated.pbxproj.contains("remoteInfo = Other;"));
    assert!(generated.pbxproj.contains("remoteInfo = AggregateTarget;"));
    assert_eq!(
        generated
            .pbxproj
            .matches("remoteInfo = AggregateTarget;")
            .count(),
        2
    );
    assert!(generated
        .pbxproj
        .contains("shellScript = \"echo aggregate\";"));
}

#[test]
fn generator_copies_swift_objc_interface_header_for_static_libraries_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "StaticLibraries",
            "targets": {
                "SwiftStaticLibraryWithHeader": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "sources": ["StaticLibrary.swift"]
                },
                "SwiftStaticLibraryWithoutHeaderName": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "settings": {"SWIFT_OBJC_INTERFACE_HEADER_NAME": ""},
                    "sources": ["StaticLibrary.swift"]
                },
                "SwiftStaticLibraryWithoutHeaderBool": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "settings": {"SWIFT_INSTALL_OBJC_HEADER": false},
                    "sources": ["StaticLibrary.swift"]
                },
                "SwiftStaticLibraryWithoutHeaderString": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "settings": {"SWIFT_INSTALL_OBJC_HEADER": "NO"},
                    "sources": ["StaticLibrary.swift"]
                },
                "ObjCStaticLibrary": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "sources": ["StaticLibrary.m"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("Copy Swift Objective-C Interface Header")
            .count(),
        3
    );
    assert!(generated
        .pbxproj
        .contains("$(DERIVED_SOURCES_DIR)/$(SWIFT_OBJC_INTERFACE_HEADER_NAME)"));
    assert!(generated.pbxproj.contains(
        "$(BUILT_PRODUCTS_DIR)/include/$(PRODUCT_MODULE_NAME)/$(SWIFT_OBJC_INTERFACE_HEADER_NAME)"
    ));
    assert!(generated.pbxproj.contains(
        "shellScript = \"ditto \\\"${SCRIPT_INPUT_FILE_0}\\\" \\\"${SCRIPT_OUTPUT_FILE_0}\\\"\\n\";"
    ));
}

#[test]
fn generator_honors_local_swift_package_group_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "options": {"localPackagesGroup": "MyPackages"},
            "packages": {
                "Yams": {"path": "../Yams"}
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "Yams"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("/* MyPackages */"));
    assert!(generated
        .pbxproj
        .contains("isa = XCLocalSwiftPackageReference;"));
    assert!(generated.pbxproj.contains("relativePath = ../Yams;"));
    assert!(generated.pbxproj.contains("lastKnownFileType = folder;"));
    assert!(generated.pbxproj.contains("path = ../Yams;"));
    assert!(generated.pbxproj.contains("productName = Yams;"));
}

#[test]
fn generator_excludes_local_swift_packages_from_project_when_requested_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "packages": {
                "XcodeGen": {
                    "path": "../XcodeGen",
                    "excludeFromProject": true
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "XcodeGen"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(!generated
        .pbxproj
        .contains("isa = XCLocalSwiftPackageReference;"));
    assert!(!generated.pbxproj.contains("relativePath = ../XcodeGen;"));
    assert!(!generated.pbxproj.contains("path = ../XcodeGen;"));
    assert!(generated.pbxproj.contains("productName = XcodeGen;"));
}

#[test]
fn generator_places_local_swift_packages_in_custom_group_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "packages": {
                "XcodeGen": {
                    "path": "../XcodeGen",
                    "group": "Packages/Feature"
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "XcodeGen"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("/* Packages */ = {"));
    assert!(generated.pbxproj.contains("/* Feature */ = {"));
    assert!(generated.pbxproj.contains("path = ../XcodeGen;"));
    assert!(generated.pbxproj.contains("productName = XcodeGen;"));
}

#[test]
fn generator_places_local_swift_packages_at_top_level_like_xcodegen() {
    let package_group_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "options": {"localPackagesGroup": ""},
            "packages": {
                "Yams": {"path": "../Yams"}
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "Yams"}]
                }
            }
        }),
    );
    let per_package_group_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "packages": {
                "XcodeGen": {
                    "path": "../XcodeGen",
                    "group": ""
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"package": "XcodeGen"}]
                }
            }
        }),
    );

    let generated_from_option = ProjectWriter::generate(&package_group_project).unwrap().pbxproj;
    assert!(generated_from_option.contains("path = ../Yams;"));
    assert!(!generated_from_option.contains("/* Packages */ = {"));

    let generated_from_package = ProjectWriter::generate(&per_package_group_project).unwrap().pbxproj;
    assert!(generated_from_package.contains("path = ../XcodeGen;"));
    assert!(!generated_from_package.contains("/* Packages */ = {"));
}

#[test]
fn generator_links_multiple_swift_package_products_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Packages",
            "packages": {
                "FooFeature": {
                    "path": "../FooFeature"
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"package": "FooFeature", "products": ["FooDomain", "FooUI"]}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("productName = FooDomain;"));
    assert!(generated.pbxproj.contains("productName = FooUI;"));
    assert!(generated.pbxproj.contains("FooDomain in Frameworks"));
    assert!(generated.pbxproj.contains("FooUI in Frameworks"));
}

#[test]
fn generator_sets_carthage_search_paths_and_copy_phase_like_xcodegen() {
    let static_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "MyStaticFramework", "findFrameworks": true, "linkType": "static"}
                    ]
                }
            }
        }),
    );
    let mixed_project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "MyDynamicFramework", "findFrameworks": true, "linkType": "dynamic"},
                        {"carthage": "MyStaticFramework", "findFrameworks": true, "linkType": "static"}
                    ]
                }
            }
        }),
    );

    let static_generated = ProjectWriter::generate(&static_project).unwrap().pbxproj;
    assert!(static_generated.contains("FRAMEWORK_SEARCH_PATHS = ("));
    assert!(static_generated.contains("Carthage/Build/iOS/Static"));
    assert!(static_generated.contains("MyStaticFramework.framework in Frameworks"));
    assert!(!static_generated.contains("MyStaticFramework.framework in CopyFiles"));

    let mixed_generated = ProjectWriter::generate(&mixed_project).unwrap().pbxproj;
    assert!(mixed_generated.contains("Carthage/Build/iOS"));
    assert!(mixed_generated.contains("Carthage/Build/iOS/Static"));
    assert!(mixed_generated.contains("MyDynamicFramework.framework in Frameworks"));
    assert!(mixed_generated.contains("MyStaticFramework.framework in Frameworks"));
    assert!(mixed_generated.contains("name = Carthage;"));
    assert!(mixed_generated.contains("carthage copy-frameworks"));
    assert!(mixed_generated.contains("$(SRCROOT)/Carthage/Build/iOS/MyDynamicFramework.framework"));
    assert!(mixed_generated
        .contains("$(BUILT_PRODUCTS_DIR)/$(FRAMEWORKS_FOLDER_PATH)/MyDynamicFramework.framework"));
    assert!(!mixed_generated.contains("MyDynamicFramework.framework in CopyFiles"));
    assert!(!mixed_generated.contains("MyStaticFramework.framework in CopyFiles"));
}

#[test]
fn generator_adds_only_matching_platform_carthage_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "targets": {
                "Watch": {
                    "type": "watch2App",
                    "platform": "watchOS",
                    "dependencies": [
                        {"carthage": "Alamofire_watch", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                },
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "Alamofire", "findFrameworks": false, "linkType": "dynamic"},
                        {"target": "Watch"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/Alamofire.framework"));
    assert!(generated.contains("Alamofire_watch.framework in Frameworks"));
    assert!(!generated.contains("$(SRCROOT)/Carthage/Build/iOS/Alamofire_watch.framework"));
}

#[test]
fn generator_emits_frameworks_group_after_source_groups_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for directory in ["A", "P", "S"] {
        fs::create_dir(temp.path().join(directory)).unwrap();
        fs::write(temp.path().join(directory).join("file.swift"), "").unwrap();
    }

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Navigator",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["A", "P", "S"],
                    "dependencies": [
                        {"carthage": "Alamofire", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("/* Frameworks */ = {"));
    assert!(generated.contains("path = Alamofire.framework;"));
    assert!(generated.contains("path = A;"));
    assert!(generated.contains("path = P;"));
    assert!(generated.contains("path = S;"));
}

#[test]
fn generator_sorts_source_groups_and_files_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    for directory in ["A", "B", "Source", "Sources/group", "Sources/group2", "Z/A"] {
        fs::create_dir_all(temp.path().join(directory)).unwrap();
    }
    for file in [
        "A/A.swift",
        "B/file.swift",
        "Source/file.swift",
        "Sources/file3.swift",
        "Sources/file.swift",
        "Sources/10file.a",
        "Sources/1file.a",
        "Sources/file2.swift",
        "Sources/group/file.swift",
        "Sources/group2/file.swift",
        "Z/A/file.swift",
    ] {
        fs::write(temp.path().join(file), "").unwrap();
    }

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SortSources",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        "Sources",
                        {"path": "Source", "name": "S"},
                        "A",
                        {"path": "Z/A", "name": "B"},
                        "B"
                    ],
                    "dependencies": [
                        {"carthage": "Alamofire", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        main_group_children_block(&generated),
        &["/* A */", "/* B */", "/* B */", "/* S */", "/* Sources */"],
    );
    assert_names_in_order(
        group_children_block_with_path(&generated, "Sources"),
        &[
            "group",
            "group2",
            "1file.a",
            "10file.a",
            "file.swift",
            "file2.swift",
            "file3.swift",
        ],
    );
}

#[test]
fn generator_uses_project_level_find_carthage_frameworks_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "options": {
                "findCarthageFrameworks": true
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "MyDynamicFramework", "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("FRAMEWORK_SEARCH_PATHS = ("));
    assert!(generated.contains("$(PROJECT_DIR)/Carthage/Build/iOS"));
}

#[test]
fn generator_resolves_related_carthage_frameworks_from_version_files_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Carthage/Build")).unwrap();
    fs::write(
        temp.path()
            .join("Carthage/Build/.CarthageTestFixture.version"),
        r#"{
            "iOS": [
                {"name": "CarthageTestFixture", "hash": "1"},
                {"name": "DependencyFixtureB", "hash": "2"},
                {"name": "DependencyFixtureA", "hash": "3"}
            ]
        }"#,
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "CarthageRelated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "CarthageTestFixture", "findFrameworks": true, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("CarthageTestFixture.framework in Frameworks"));
    assert!(generated.contains("DependencyFixtureA.framework in Frameworks"));
    assert!(generated.contains("DependencyFixtureB.framework in Frameworks"));
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/CarthageTestFixture.framework"));
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/DependencyFixtureA.framework"));
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/DependencyFixtureB.framework"));
}

#[test]
fn generator_deduplicates_related_carthage_frameworks_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Carthage/Build")).unwrap();
    fs::write(
        temp.path().join("Carthage/Build/.ReactiveSwift.version"),
        r#"{
            "iOS": [
                {"name": "ReactiveSwift", "hash": "1"},
                {"name": "ReactiveSwift", "hash": "1"}
            ]
        }"#,
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "CarthageRelated",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "ReactiveSwift", "findFrameworks": true, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_eq!(
        generated
            .matches("$(SRCROOT)/Carthage/Build/iOS/ReactiveSwift.framework")
            .count(),
        1
    );
}

#[test]
fn generator_sorts_carthage_dependencies_for_copy_frameworks_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CarthageSorted",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "RxSwift", "findFrameworks": false, "linkType": "dynamic"},
                        {"carthage": "RxCocoa", "findFrameworks": false, "linkType": "dynamic"},
                        {"carthage": "RxBlocking", "findFrameworks": false, "linkType": "dynamic"},
                        {"carthage": "RxTest", "findFrameworks": false, "linkType": "dynamic"},
                        {"carthage": "RxAtomic", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert_names_in_order(
        &generated,
        &[
            "$(SRCROOT)/Carthage/Build/iOS/RxAtomic.framework",
            "$(SRCROOT)/Carthage/Build/iOS/RxBlocking.framework",
            "$(SRCROOT)/Carthage/Build/iOS/RxCocoa.framework",
            "$(SRCROOT)/Carthage/Build/iOS/RxSwift.framework",
            "$(SRCROOT)/Carthage/Build/iOS/RxTest.framework",
        ],
    );
}

#[test]
fn generator_resolves_transitive_carthage_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "TransitiveCarthage",
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "NestedFramework", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                },
                "SkippedFramework": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "SkippedNestedFramework", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                },
                "OtherPlatformFramework": {
                    "type": "framework",
                    "platform": "tvOS",
                    "dependencies": [
                        {"carthage": "TvNestedFramework", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                },
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "Framework"},
                        {"target": "SkippedFramework", "embed": false, "link": false},
                        {"target": "OtherPlatformFramework"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/NestedFramework.framework"));
    assert!(!generated.contains("$(SRCROOT)/Carthage/Build/iOS/SkippedNestedFramework.framework"));
    assert!(!generated.contains("$(SRCROOT)/Carthage/Build/iOS/TvNestedFramework.framework"));
}

#[test]
fn generator_resolves_carthage_dependencies_through_aggregate_targets_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "AggregateCarthage",
            "aggregateTargets": {
                "Dependencies": {
                    "targets": ["Framework"]
                }
            },
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "AggregateNestedFramework", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                },
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "Dependencies"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/iOS/AggregateNestedFramework.framework"));
}

#[test]
fn generator_uses_custom_carthage_build_path_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "options": {
                "carthageBuildPath": "Vendor/CarthageBuild"
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "MyStaticFramework", "findFrameworks": true, "linkType": "static"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("$(PROJECT_DIR)/Vendor/CarthageBuild/iOS/Static"));
    assert!(!generated.contains("$(PROJECT_DIR)/Carthage/Build/iOS/Static"));
}

#[test]
fn generator_uses_custom_carthage_executable_path_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "Carthage",
            "options": {
                "carthageExecutablePath": "../bin/carthage"
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"carthage": "MyDynamicFramework", "findFrameworks": false, "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("../bin/carthage copy-frameworks"));
    assert!(!generated.contains("shellScript = \"carthage copy-frameworks"));
}

#[test]
fn generator_directly_embeds_macos_carthage_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CarthageDirectEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"carthage": "frameworkA.framework", "linkType": "dynamic"},
                        {"carthage": "frameworkB.framework", "linkType": "dynamic", "embed": false},
                        {"carthage": "frameworkC.framework", "linkType": "static"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("name = \"Embed Frameworks\";"));
    assert!(generated.contains("dstSubfolderSpec = 10;"));
    assert!(generated.contains("frameworkA.framework in Embed Frameworks"));
    assert!(!generated.contains("frameworkB.framework in Embed Frameworks"));
    assert!(!generated.contains("frameworkC.framework in Embed Frameworks"));
    assert!(!generated.contains("carthage copy-frameworks"));
}

#[test]
fn generator_uses_custom_copy_phase_for_carthage_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CarthageCustomCopy",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {
                            "carthage": "frameworkA.framework",
                            "linkType": "dynamic",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        },
                        {
                            "carthage": "frameworkB.framework",
                            "linkType": "static",
                            "embed": false,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("name = \"Embed Dependencies\";"));
    assert!(generated.contains("dstSubfolderSpec = 13;"));
    assert!(generated.contains("dstPath = test;"));
    assert!(generated.contains("frameworkA.framework in Embed Dependencies"));
    assert!(!generated.contains("frameworkB.framework in Embed Dependencies"));
    assert!(!generated.contains("carthage copy-frameworks"));
}

#[test]
fn generator_honors_directly_embed_carthage_dependencies_override_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CarthageDirectEmbedOverride",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "directlyEmbedCarthageDependencies": false,
                    "dependencies": [
                        {"carthage": "frameworkA.framework", "linkType": "dynamic"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap().pbxproj;
    assert!(generated.contains("carthage copy-frameworks"));
    assert!(generated.contains("$(SRCROOT)/Carthage/Build/Mac/frameworkA.framework"));
    assert!(!generated.contains("frameworkA.framework in Embed Frameworks"));
}

#[test]
fn generator_emits_weak_target_dependency_build_file_settings_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "WeakLinks",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "RequiredFramework"},
                        {"target": "OptionalFramework", "weak": true}
                    ]
                },
                "RequiredFramework": {"type": "framework", "platform": "iOS"},
                "OptionalFramework": {"type": "framework", "platform": "iOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("OptionalFramework.framework in Frameworks"));
    assert_eq!(generated.pbxproj.matches("ATTRIBUTES = (").count(), 3);
    assert!(generated.pbxproj.contains("Weak,"));
}

#[test]
fn generator_emits_source_destination_filters_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    fs::write(temp.path().join("Sources/iOS.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/Mac.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "DestinationFilters",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "auto",
                    "sources": [
                        {
                            "path": "Sources/iOS.swift",
                            "destinationFilters": ["iOS"]
                        },
                        {
                            "path": "Sources/Mac.swift",
                            "destinationFilters": ["macOS", "macCatalyst"]
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("iOS.swift in Sources"));
    assert!(generated.pbxproj.contains("Mac.swift in Sources"));
    assert!(generated.pbxproj.contains("platformFilters = ("));
    assert!(generated.pbxproj.contains("ios,"));
    assert!(generated.pbxproj.contains("macos,"));
    assert!(generated.pbxproj.contains("maccatalyst,"));
}

#[test]
fn generator_infers_source_destination_filters_by_path_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/iOs")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/TVOS")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/macos")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/macCatalyst")).unwrap();
    fs::write(temp.path().join("Sources/iOs/File_A.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/TVOS/File_B.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/macos/File_C.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/macCatalyst/File_D.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/File_ios.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/File_tvOs.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/File_macOS.swift"), "").unwrap();
    fs::write(temp.path().join("Sources/File_MACCATALYST.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "DestinationFilters",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "auto",
                    "sources": [{
                        "path": "Sources",
                        "inferDestinationFiltersByPath": true
                    }]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(generated.pbxproj.matches("platformFilters = (").count(), 8);
    assert_eq!(generated.pbxproj.matches("ios,").count(), 2);
    assert_eq!(generated.pbxproj.matches("tvos,").count(), 2);
    assert_eq!(generated.pbxproj.matches("macos,").count(), 2);
    assert_eq!(generated.pbxproj.matches("maccatalyst,").count(), 2);
}

#[test]
fn generator_emits_dependency_destination_filters_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "DependencyFilters",
            "packages": {
                "RxSwift": {
                    "url": "https://github.com/ReactiveX/RxSwift",
                    "majorVersion": "5.1.1"
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "FrameworkA", "destinationFilters": ["iOS"]},
                        {"framework": "FrameworkB.framework", "destinationFilters": ["iOS", "tvOS"]},
                        {"sdk": "StoreKit.framework", "destinationFilters": ["macOS"]},
                        {"package": "RxSwift", "product": "RxSwift", "destinationFilters": ["iOS"]}
                    ]
                },
                "FrameworkA": {"type": "framework", "platform": "iOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(generated.pbxproj.matches("platformFilters = (").count(), 7);
    assert_eq!(generated.pbxproj.matches("ios,").count(), 6);
    assert_eq!(generated.pbxproj.matches("tvos,").count(), 2);
    assert_eq!(generated.pbxproj.matches("macos,").count(), 1);
    assert!(generated
        .pbxproj
        .contains("FrameworkA.framework in Frameworks"));
    assert!(generated
        .pbxproj
        .contains("FrameworkB.framework in Frameworks"));
    assert!(generated
        .pbxproj
        .contains("StoreKit.framework in Frameworks"));
    assert!(generated.pbxproj.contains("RxSwift in Frameworks"));
}

#[test]
fn generator_copies_bundle_dependencies_into_resources_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "BundleDependencies",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"bundle": "bundleA.bundle", "destinationFilters": ["iOS"]},
                        {"bundle": "bundleB.bundle", "destinationFilters": ["iOS", "tvOS"]}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(generated
        .pbxproj
        .contains("name = \"Copy Bundle Resources\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 7;"));
    assert!(generated
        .pbxproj
        .contains("bundleA.bundle in Copy Bundle Resources"));
    assert!(generated
        .pbxproj
        .contains("bundleB.bundle in Copy Bundle Resources"));
    assert_eq!(generated.pbxproj.matches("platformFilters = (").count(), 2);
    assert_eq!(generated.pbxproj.matches("ios,").count(), 2);
    assert_eq!(generated.pbxproj.matches("tvos,").count(), 1);
}

#[test]
fn generator_ignores_custom_copy_phase_for_bundle_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "BundleCopyPhase",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {
                            "bundle": "bundleA.bundle",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        },
                        {
                            "bundle": "bundleB.bundle",
                            "embed": false,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("name = \"Copy Bundle Resources\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 7;"));
    assert!(generated
        .pbxproj
        .contains("bundleA.bundle in Copy Bundle Resources"));
    assert!(generated
        .pbxproj
        .contains("bundleB.bundle in Copy Bundle Resources"));
    assert!(!generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(!generated.pbxproj.contains("dstPath = test;"));
    assert!(!generated.pbxproj.contains("dstPath = plugins;"));
}

#[test]
fn generator_embeds_target_framework_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "TargetFrameworkEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"target": "FrameworkA", "embed": true},
                        {"target": "FrameworkB", "embed": false}
                    ]
                },
                "FrameworkA": {"type": "framework", "platform": "macOS"},
                "FrameworkB": {"type": "framework", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(generated.pbxproj.contains("name = \"Embed Frameworks\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated
        .pbxproj
        .contains("FrameworkA.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("FrameworkB.framework in Embed Frameworks"));
}

#[test]
fn generator_uses_custom_copy_phase_for_embedded_target_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "TargetCustomCopy",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {
                            "target": "HelperAppA",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        },
                        {
                            "target": "HelperAppB",
                            "embed": false,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        }
                    ]
                },
                "HelperAppA": {"type": "application", "platform": "macOS"},
                "HelperAppB": {"type": "application", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(generated.pbxproj.contains("name = \"Embed Dependencies\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(generated.pbxproj.contains("dstPath = test;"));
    assert!(generated
        .pbxproj
        .contains("HelperAppA.app in Embed Dependencies"));
    assert!(!generated
        .pbxproj
        .contains("HelperAppB.app in Embed Dependencies"));
}

#[test]
fn generator_embeds_extensionkit_dependencies_into_products_directory_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "ExtensionKitEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"target": "ExtensionA", "embed": true},
                        {"target": "ExtensionB", "embed": false}
                    ]
                },
                "ExtensionA": {"type": "extensionKitExtension", "platform": "macOS"},
                "ExtensionB": {"type": "extensionKitExtension", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 16;"));
    assert!(generated
        .pbxproj
        .contains("dstPath = \"$(EXTENSIONS_FOLDER_PATH)\";"));
    assert!(generated
        .pbxproj
        .contains("ExtensionA.appex in Embed Foundation Extensions"));
    assert!(!generated
        .pbxproj
        .contains("ExtensionB.appex in Embed Foundation Extensions"));
}

#[test]
fn generator_embeds_xpc_service_dependencies_into_xpc_services_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "XpcEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"target": "XpcA", "embed": true},
                        {"target": "XpcB", "embed": false}
                    ]
                },
                "XpcA": {"type": "xpcService", "platform": "macOS"},
                "XpcB": {"type": "xpcService", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 16;"));
    assert!(generated
        .pbxproj
        .contains("dstPath = \"$(CONTENTS_FOLDER_PATH)/XPCServices\";"));
    assert!(generated.pbxproj.contains("XpcA.xpc in CopyFiles"));
    assert!(!generated.pbxproj.contains("XpcB.xpc in CopyFiles"));
}

#[test]
fn generator_embeds_app_clip_dependencies_into_app_clips_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "AppClipEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "ClipA", "embed": true},
                        {"target": "ClipB", "embed": false}
                    ]
                },
                "ClipA": {"type": "application.on-demand-install-capable", "platform": "iOS"},
                "ClipB": {"type": "application.on-demand-install-capable", "platform": "iOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 16;"));
    assert!(generated
        .pbxproj
        .contains("dstPath = \"$(CONTENTS_FOLDER_PATH)/AppClips\";"));
    assert!(generated.pbxproj.contains("ClipA.app in Embed App Clips"));
    assert!(!generated.pbxproj.contains("ClipB.app in Embed App Clips"));
}

#[test]
fn generator_embeds_xcode_and_intents_extensions_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "ExtensionEmbeds",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"target": "XcodeExtensionA", "embed": true},
                        {"target": "IntentsExtensionA", "embed": true},
                        {"target": "XcodeExtensionB", "embed": false}
                    ]
                },
                "XcodeExtensionA": {"type": "xcodeExtension", "platform": "macOS"},
                "XcodeExtensionB": {"type": "xcodeExtension", "platform": "macOS"},
                "IntentsExtensionA": {"type": "intentsServiceExtension", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(generated
        .pbxproj
        .contains("XcodeExtensionA.appex in Embed Foundation Extensions"));
    assert!(generated
        .pbxproj
        .contains("IntentsExtensionA.appex in Embed Foundation Extensions"));
    assert!(!generated
        .pbxproj
        .contains("XcodeExtensionB.appex in Embed Foundation Extensions"));
}

#[test]
fn generator_uses_custom_copy_phase_for_unembedded_product_types_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "CustomProductEmbeds",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {
                            "target": "OcUnit",
                            "embed": true,
                            "copy": {"destination": "frameworks", "subpath": "test"}
                        },
                        {
                            "target": "Instruments",
                            "embed": true,
                            "copy": {"destination": "frameworks", "subpath": "test"}
                        },
                        {
                            "target": "Metal",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        }
                    ]
                },
                "OcUnit": {"type": "ocUnitTestBundle", "platform": "macOS"},
                "Instruments": {"type": "instrumentsPackage", "platform": "macOS"},
                "Metal": {"type": "metalLibrary", "platform": "macOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("dstPath = test;"));
    assert!(generated
        .pbxproj
        .contains("OcUnit.octest in Embed Frameworks"));
    assert!(generated
        .pbxproj
        .contains("Instruments.instrpkg in Embed Frameworks"));
    assert!(generated
        .pbxproj
        .contains("Metal.metallib in Embed Dependencies"));
}

#[test]
fn generator_embeds_framework_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "FrameworkDependencyEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"framework": "frameworkA.framework", "embed": true},
                        {"framework": "frameworkB.framework", "embed": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("name = \"Embed Frameworks\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated
        .pbxproj
        .contains("frameworkA.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("frameworkB.framework in Embed Frameworks"));
}

#[test]
fn generator_embeds_framework_dependencies_by_default_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "FrameworkDependencyDefaultEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"framework": "FrameworkA.framework"},
                        {"framework": "FrameworkB.framework", "embed": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("name = \"Embed Frameworks\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated
        .pbxproj
        .contains("FrameworkA.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("FrameworkB.framework in Embed Frameworks"));
}

#[test]
fn generator_uses_single_custom_copy_phase_for_framework_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "FrameworkDependencyCustomCopy",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {
                            "framework": "frameworkA.framework",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        },
                        {
                            "framework": "frameworkB.framework",
                            "embed": true,
                            "copy": {"destination": "plugins", "subpath": "test"}
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(
        generated
            .pbxproj
            .matches("isa = PBXCopyFilesBuildPhase;")
            .count(),
        1
    );
    assert!(generated.pbxproj.contains("name = \"Embed Dependencies\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(generated.pbxproj.contains("dstPath = test;"));
    assert!(generated
        .pbxproj
        .contains("frameworkA.framework in Embed Dependencies"));
    assert!(generated
        .pbxproj
        .contains("frameworkB.framework in Embed Dependencies"));
}

#[test]
fn generator_embeds_sdk_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "SdkDependencyEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"sdk": "sdkA.framework", "embed": true},
                        {"sdk": "sdkB.framework", "embed": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("name = \"Embed Frameworks\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated
        .pbxproj
        .contains("sdkA.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("sdkB.framework in Embed Frameworks"));
}

#[test]
fn generator_embeds_package_dependencies_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "PackageDependencyEmbed",
            "packages": {
                "RxSwift": {
                    "url": "https://github.com/ReactiveX/RxSwift",
                    "majorVersion": "5.1.1"
                }
            },
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "macOS",
                    "dependencies": [
                        {"package": "RxSwift", "products": ["RxSwift"], "embed": true},
                        {"package": "RxSwift", "products": ["RxCocoa"], "embed": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("name = \"Embed Frameworks\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 10;"));
    assert!(generated.pbxproj.contains("RxSwift in Embed Frameworks"));
    assert!(!generated.pbxproj.contains("RxCocoa in Embed Frameworks"));
}

#[test]
fn generator_embeds_app_extension_target_dependencies_by_default_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "AppExtensionDefaultEmbed",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "tvOS",
                    "dependencies": [
                        {"target": "AppExtension"},
                        {"target": "OtherExtension", "embed": false}
                    ]
                },
                "AppExtension": {"type": "appExtension", "platform": "tvOS"},
                "OtherExtension": {"type": "appExtension", "platform": "tvOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("name = \"Embed Foundation Extensions\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(generated
        .pbxproj
        .contains("AppExtension.appex in Embed Foundation Extensions"));
    assert!(!generated
        .pbxproj
        .contains("OtherExtension.appex in Embed Foundation Extensions"));
}

#[test]
fn generator_does_not_embed_static_framework_targets_by_default_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "StaticFrameworks",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [
                        {"target": "DynamicFramework"},
                        {"target": "DynamicFrameworkNotEmbedded", "embed": false},
                        {"target": "StaticFramework"},
                        {"target": "StaticFrameworkExplicitlyEmbedded", "embed": true},
                        {"target": "StaticFramework2"},
                        {"target": "StaticFramework2ExplicitlyEmbedded", "embed": true},
                        {"target": "StaticLibrary"}
                    ]
                },
                "DynamicFramework": {"type": "framework", "platform": "iOS"},
                "DynamicFrameworkNotEmbedded": {"type": "framework", "platform": "iOS"},
                "StaticFramework": {
                    "type": "framework",
                    "platform": "iOS",
                    "settings": {"MACH_O_TYPE": "staticlib"}
                },
                "StaticFrameworkExplicitlyEmbedded": {
                    "type": "framework",
                    "platform": "iOS",
                    "settings": {"MACH_O_TYPE": "staticlib"}
                },
                "StaticFramework2": {"type": "staticFramework", "platform": "iOS"},
                "StaticFramework2ExplicitlyEmbedded": {"type": "staticFramework", "platform": "iOS"},
                "StaticLibrary": {"type": "staticLibrary", "platform": "iOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    for linked in [
        "DynamicFramework.framework in Frameworks",
        "DynamicFrameworkNotEmbedded.framework in Frameworks",
        "StaticFramework.framework in Frameworks",
        "StaticFrameworkExplicitlyEmbedded.framework in Frameworks",
        "StaticFramework2.framework in Frameworks",
        "StaticFramework2ExplicitlyEmbedded.framework in Frameworks",
        "libStaticLibrary.a in Frameworks",
    ] {
        assert!(generated.pbxproj.contains(linked), "{linked} should link");
    }
    assert!(generated
        .pbxproj
        .contains("DynamicFramework.framework in Embed Frameworks"));
    assert!(generated
        .pbxproj
        .contains("StaticFrameworkExplicitlyEmbedded.framework in Embed Frameworks"));
    assert!(generated
        .pbxproj
        .contains("StaticFramework2ExplicitlyEmbedded.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("DynamicFrameworkNotEmbedded.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("StaticFramework.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("StaticFramework2.framework in Embed Frameworks"));
    assert!(!generated
        .pbxproj
        .contains("libStaticLibrary.a in Embed Frameworks"));
}

#[test]
fn generator_sets_objc_linker_flag_for_objc_linking_dependencies_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("StaticLibrary_ObjC")).unwrap();
    fs::write(
        temp.path().join("StaticLibrary_ObjC/StaticLibrary_ObjC.m"),
        "",
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "ObjCLinking",
            "targets": {
                "requiresObjCLinking": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "requiresObjCLinking": true
                },
                "doesntRequireObjCLinking": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "requiresObjCLinking": false
                },
                "implicitlyRequiresObjCLinking": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "sources": ["StaticLibrary_ObjC/StaticLibrary_ObjC.m"]
                },
                "framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "dependencies": [{"target": "requiresObjCLinking", "link": false}]
                },
                "app1": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "requiresObjCLinking"}]
                },
                "app2": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "doesntRequireObjCLinking"}]
                },
                "app3": {
                    "type": "application",
                    "platform": "iOS",
                    "dependencies": [{"target": "implicitlyRequiresObjCLinking"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert_eq!(generated.pbxproj.matches("-ObjC").count(), 4);
    assert!(generated.pbxproj.contains("OTHER_LDFLAGS = ("));
}

#[test]
fn generator_marks_embed_frameworks_copy_phase_only_on_install_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "OnlyCopyFrameworksOnInstall",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "onlyCopyFilesOnInstall": true,
                    "dependencies": [
                        {"framework": "FrameworkA.framework"},
                        {"framework": "FrameworkB.framework", "embed": false}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("FrameworkA.framework in Embed Frameworks"));
    assert!(generated
        .pbxproj
        .contains("runOnlyForDeploymentPostprocessing = 1;"));
}

#[test]
fn generator_marks_embed_app_extensions_copy_phase_only_on_install_like_xcodegen() {
    let project = project_from_json(
        std::path::PathBuf::new(),
        serde_json::json!({
            "name": "OnlyCopyExtensionsOnInstall",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "tvOS",
                    "onlyCopyFilesOnInstall": true,
                    "dependencies": [{"target": "AppExtension"}]
                },
                "AppExtension": {"type": "appExtension", "platform": "tvOS"}
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("AppExtension.appex in Embed Foundation Extensions"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 13;"));
    assert!(!generated
        .pbxproj
        .contains("runOnlyForDeploymentPostprocessing = 1;"));
}

#[test]
fn generator_emits_source_build_file_settings_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("App.swift"), "").unwrap();
    fs::write(temp.path().join("Asset.png"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "SourceSettings",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        {
                            "path": "App.swift",
                            "compilerFlags": ["-DDEBUG", "-warnings-as-errors"],
                            "attributes": ["Public"]
                        },
                        {
                            "path": "Asset.png",
                            "resourceTags": ["on-demand"],
                            "attributes": ["RemoveHeadersOnCopy"]
                        }
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("COMPILER_FLAGS = \"-DDEBUG -warnings-as-errors\";"));
    assert!(generated.pbxproj.contains("ATTRIBUTES = ("));
    assert!(generated.pbxproj.contains("Public,"));
    assert!(generated.pbxproj.contains("RemoveHeadersOnCopy,"));
    assert!(generated.pbxproj.contains("ASSET_TAGS = ("));
    assert!(generated.pbxproj.contains("\"on-demand\","));
}

#[test]
fn generator_emits_headers_build_phase_for_frameworks_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Headers")).unwrap();
    for file in [
        "Public.h",
        "Private.hh",
        "Project.hpp",
        "Inline.ipp",
        "Template.tpp",
        "Other.hxx",
        "Module.def",
    ] {
        fs::write(temp.path().join("Headers").join(file), "").unwrap();
    }

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Headers",
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Headers/Public.h"},
                        {"path": "Headers/Private.hh", "headerVisibility": "private"},
                        {"path": "Headers/Project.hpp", "headerVisibility": "project"},
                        {"path": "Headers/Inline.ipp"},
                        {"path": "Headers/Template.tpp"},
                        {"path": "Headers/Other.hxx"},
                        {"path": "Headers/Module.def"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("isa = PBXHeadersBuildPhase;"));
    assert!(generated.pbxproj.contains("Public.h in Headers"));
    assert!(generated.pbxproj.contains("Private.hh in Headers"));
    assert!(generated.pbxproj.contains("Project.hpp in Headers"));
    assert!(generated.pbxproj.contains("Inline.ipp in Headers"));
    assert!(generated.pbxproj.contains("Template.tpp in Headers"));
    assert!(generated.pbxproj.contains("Other.hxx in Headers"));
    assert!(generated.pbxproj.contains("Module.def in Headers"));
    assert!(generated.pbxproj.contains("Public,"));
    assert!(generated.pbxproj.contains("Private,"));
}

#[test]
fn generator_drops_headers_phase_for_application_targets_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("Public.h"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "Headers",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Public.h"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(!generated.pbxproj.contains("isa = PBXHeadersBuildPhase;"));
    assert!(!generated.pbxproj.contains("Public.h in Headers"));
}

#[test]
fn generator_copies_public_static_library_headers_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("Public.h"), "").unwrap();
    fs::write(temp.path().join("Private.h"), "").unwrap();
    fs::write(temp.path().join("Project.h"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "StaticHeaders",
            "targets": {
                "Library": {
                    "type": "staticLibrary",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Public.h"},
                        {"path": "Private.h", "headerVisibility": "private"},
                        {"path": "Project.h", "headerVisibility": "project"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(!generated.pbxproj.contains("isa = PBXHeadersBuildPhase;"));
    assert!(generated.pbxproj.contains("isa = PBXCopyFilesBuildPhase;"));
    assert!(!generated.pbxproj.contains("name = \"Copy Headers\";"));
    assert!(generated.pbxproj.contains("dstSubfolderSpec = 16;"));
    assert!(generated
        .pbxproj
        .contains("dstPath = \"include/$(PRODUCT_NAME)\";"));
    assert!(generated.pbxproj.contains("Public.h in CopyFiles"));
    assert!(!generated.pbxproj.contains("Private.h in CopyFiles"));
    assert!(!generated.pbxproj.contains("Project.h in CopyFiles"));
}

#[test]
fn generator_respects_source_build_phase_overrides_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Resources")).unwrap();
    fs::create_dir(temp.path().join("Excluded")).unwrap();
    fs::write(temp.path().join("Resources/Forced.swift"), "").unwrap();
    fs::write(temp.path().join("Resources/Forced.h"), "").unwrap();
    fs::write(temp.path().join("Excluded/Skipped.swift"), "").unwrap();
    fs::write(temp.path().join("ForcedResource.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "BuildPhases",
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Resources", "buildPhase": "resources"},
                        {"path": "Excluded", "buildPhase": "none"},
                        {"path": "ForcedResource.swift", "buildPhase": "resources"}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("Forced.swift in Resources"));
    assert!(generated.pbxproj.contains("Forced.h in Resources"));
    assert!(generated
        .pbxproj
        .contains("ForcedResource.swift in Resources"));
    assert!(!generated.pbxproj.contains("Forced.swift in Sources"));
    assert!(!generated.pbxproj.contains("Forced.h in Headers"));
    assert!(!generated.pbxproj.contains("Skipped.swift in Sources"));
    assert!(!generated.pbxproj.contains("Skipped.swift in Resources"));
}

#[test]
fn generator_matches_xcodegen_default_file_type_build_phases() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("Sources")).unwrap();
    for file in [
        "file.S",
        "file.metal",
        "file.mlmodel",
        "file.mlpackage",
        "Intent.intentdefinition",
        "Documentation.docc",
        "Localizable.xcstrings",
        "Configuration.storekit",
        "Compiled.mlmodelc",
        "file.123",
        "file.xcconfig",
        "file.entitlements",
        "file.gpx",
        "file.apns",
        "Plan.xctestplan",
    ] {
        fs::write(temp.path().join("Sources").join(file), "").unwrap();
    }

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "DefaultFileTypes",
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("file.S in Sources"));
    assert!(generated.pbxproj.contains("file.metal in Sources"));
    assert!(generated.pbxproj.contains("file.mlmodel in Sources"));
    assert!(generated.pbxproj.contains("file.mlpackage in Sources"));
    assert!(generated
        .pbxproj
        .contains("Intent.intentdefinition in Sources"));
    assert!(generated.pbxproj.contains("Documentation.docc in Sources"));
    assert!(generated
        .pbxproj
        .contains("Localizable.xcstrings in Resources"));
    assert!(generated
        .pbxproj
        .contains("Configuration.storekit in Resources"));
    assert!(generated.pbxproj.contains("Compiled.mlmodelc in Resources"));
    assert!(generated.pbxproj.contains("file.123 in Resources"));
    assert!(!generated.pbxproj.contains("file.xcconfig in Resources"));
    assert!(!generated.pbxproj.contains("file.entitlements in Resources"));
    assert!(!generated.pbxproj.contains("file.gpx in Resources"));
    assert!(!generated.pbxproj.contains("file.apns in Resources"));
    assert!(!generated.pbxproj.contains("Plan.xctestplan in Resources"));
}

#[test]
fn generator_places_localized_intent_definitions_in_sources_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Base.lproj")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/en.lproj")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/ja.lproj")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/Base.lproj/Intents.intentdefinition"),
        "",
    )
    .unwrap();
    fs::write(temp.path().join("Sources/en.lproj/Intents.strings"), "").unwrap();
    fs::write(temp.path().join("Sources/ja.lproj/Intents.strings"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "IntentDefinitions",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("Intents.intentdefinition in Sources"));
    assert!(!generated
        .pbxproj
        .contains("Intents.intentdefinition in Resources"));
}

#[test]
fn generator_respects_build_phase_for_localized_intent_definitions_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Base.lproj")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/en.lproj")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/Base.lproj/Intents.intentdefinition"),
        "",
    )
    .unwrap();
    fs::write(temp.path().join("Sources/en.lproj/Intents.strings"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "IntentDefinitions",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [{"path": "Sources", "buildPhase": "resources"}]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated
        .pbxproj
        .contains("Intents.intentdefinition in Resources"));
    assert!(!generated
        .pbxproj
        .contains("Intents.intentdefinition in Sources"));
}

#[test]
fn generator_applies_custom_file_type_properties_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir(temp.path().join("A")).unwrap();
    fs::write(temp.path().join("A/file.resource1"), "").unwrap();
    fs::write(temp.path().join("A/file.source1"), "").unwrap();
    fs::create_dir(temp.path().join("A/file.abc")).unwrap();
    fs::write(temp.path().join("A/file.abc/file.a"), "").unwrap();
    fs::write(temp.path().join("A/file.unphased1"), "").unwrap();
    fs::write(temp.path().join("A/ignored.swift"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "CustomFileTypes",
            "options": {
                "fileTypes": {
                    "abc": {"buildPhase": "sources"},
                    "source1": {
                        "buildPhase": "sources",
                        "attributes": ["a1", "a2"],
                        "resourceTags": ["r1", "r2"],
                        "compilerFlags": ["-c1", "-c2"]
                    },
                    "resource1": {
                        "buildPhase": "resources",
                        "attributes": ["a1", "a2"],
                        "resourceTags": ["r1", "r2"],
                        "compilerFlags": ["-c1", "-c2"]
                    },
                    "unphased1": {"buildPhase": "none"},
                    "swift": {"buildPhase": "resources"}
                }
            },
            "targets": {
                "Framework": {
                    "type": "framework",
                    "platform": "iOS",
                    "sources": ["A"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("file.abc in Sources"));
    assert!(!generated.pbxproj.contains("file.a in Frameworks"));
    assert!(generated.pbxproj.contains("file.source1 in Sources"));
    assert!(generated.pbxproj.contains("file.resource1 in Resources"));
    assert!(!generated.pbxproj.contains("file.unphased1 in Sources"));
    assert!(!generated.pbxproj.contains("file.unphased1 in Resources"));
    assert!(generated.pbxproj.contains("ignored.swift in Resources"));
    assert!(generated.pbxproj.contains("COMPILER_FLAGS = \"-c1 -c2\";"));
    assert!(generated.pbxproj.contains("ASSET_TAGS = ("));
    assert!(generated.pbxproj.contains("r1,"));
    assert!(generated.pbxproj.contains("r2,"));
    assert!(generated.pbxproj.contains("a1,"));
    assert!(generated.pbxproj.contains("a2,"));
}

#[test]
fn generator_detects_known_regions_from_lproj_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join("Sources/Base.lproj")).unwrap();
    fs::create_dir_all(temp.path().join("Sources/en-CA.lproj")).unwrap();
    fs::write(
        temp.path()
            .join("Sources/Base.lproj/LocalizedStoryboard.storyboard"),
        "",
    )
    .unwrap();
    fs::write(
        temp.path().join("Sources/en-CA.lproj/Localizable.strings"),
        "",
    )
    .unwrap();
    fs::write(
        temp.path().join("Sources/Localizable.xcstrings"),
        r#"{
            "sourceLanguage": "en",
            "strings": {
                "foo": {"localizations": {"en": {}, "es": {}}},
                "bar": {"localizations": {"it": {}}}
            },
            "version": "1.0"
        }"#,
    )
    .unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "KnownRegions",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": ["Sources"]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("knownRegions = ("));
    for region in ["Base", "en", "en-CA"] {
        let expected = if region.contains('-') {
            format!("\t\t\t\t\"{region}\",")
        } else {
            format!("\t\t\t\t{region},")
        };
        assert!(
            generated.pbxproj.contains(&expected),
            "missing known region {region}"
        );
    }
}

#[test]
fn generator_emits_known_asset_tags_like_xcodegen() {
    let temp = tempfile::TempDir::new().unwrap();
    fs::write(temp.path().join("Asset1.png"), "").unwrap();
    fs::write(temp.path().join("Asset2.png"), "").unwrap();

    let project = project_from_json(
        temp.path().to_path_buf(),
        serde_json::json!({
            "name": "KnownAssetTags",
            "targets": {
                "App": {
                    "type": "application",
                    "platform": "iOS",
                    "sources": [
                        {"path": "Asset1.png", "resourceTags": ["tag2", "tag1"]},
                        {"path": "Asset2.png", "resourceTags": ["tag3", "tag2"]}
                    ]
                }
            }
        }),
    );

    let generated = ProjectWriter::generate(&project).unwrap();
    assert!(generated.pbxproj.contains("knownAssetTags = ("));
    assert!(generated.pbxproj.contains("tag1,"));
    assert!(generated.pbxproj.contains("tag2,"));
    assert!(generated.pbxproj.contains("tag3,"));
}
