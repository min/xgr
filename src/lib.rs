//! Rust port of [XcodeGen](https://github.com/yonaskolb/XcodeGen)'s
//! `project.yml` spec loader and `.xcodeproj` generator.
//!
//! The crate exposes two main flows:
//!
//! - Load a project spec with [`SpecLoader::load_project`], which resolves
//!   includes, expands variables, and merges templates.
//! - Generate or write the resulting project with [`ProjectWriter::generate`]
//!   or [`ProjectWriter::write`].
//!
//! ```no_run
//! use std::collections::HashMap;
//! use xcodegenrust::{ProjectWriter, SpecLoader};
//!
//! let project = SpecLoader::load_project("project.yml", None, HashMap::new())?;
//! ProjectWriter::write(&project, None)?;
//! # Ok::<(), xcodegenrust::ProjectWriteError>(())
//! ```
//!
//! See `README.md` and `TEST_PARITY.md` for the upstream-XcodeGen
//! compatibility story.

#[allow(dead_code)]
mod core;
#[doc(hidden)]
pub mod pbxproj;
#[doc(hidden)]
pub mod spec;

pub use pbxproj::{GeneratedProject, ProjectWriteError, ProjectWriter};
pub use spec::{
    AggregateTarget, Breakpoint, BreakpointAction, BreakpointField, BreakpointLogConveyanceType,
    BreakpointScope, BreakpointSound, BreakpointStopOnStyle, BreakpointType, BuildRule,
    BuildRuleAction, BuildRuleFileType, BuildScript, BuildScriptKind, BuildToolPlugin, BuildType,
    CarthageLinkType, Dependency, DependencyType, DeploymentTarget, EnvironmentVariable,
    FileBuildPhase, FileType, GroupOrdering, GroupSortPosition, PackageVersionRequirement,
    Platform, PlatformFilter, Plist, ProductType, Project, Scheme, SchemeAction, SchemeBuild,
    SchemeBuildTarget, SchemeExecutionAction, SchemeManagement, SchemeRun,
    SchemeSimulateLocation, SchemeTest, SchemeTestTarget, Settings, SourceType, SpecError,
    SpecFile, SpecLoader, SpecOptions, SpecValidationError, SwiftPackage, Target, TargetScheme,
    TestPlan, ValidationError,
};
