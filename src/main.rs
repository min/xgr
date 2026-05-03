use clap::{Parser, Subcommand};
use std::collections::HashMap;
use std::path::PathBuf;
use xcodegenrust::{Project, ProjectWriter, SpecFile, SpecLoader};

#[derive(Debug, Parser)]
#[command(name = "xgr")]
#[command(about = "Rust implementation of XcodeGen-compatible project.yml loading")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Generate an .xcodeproj from a project.yml spec.
    Generate {
        #[arg(short, long, default_value = "project.yml")]
        spec: PathBuf,
        #[arg(short, long)]
        project_root: Option<PathBuf>,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(long = "var", value_parser = parse_key_value)]
        variables: Vec<(String, String)>,
    },
    /// Print the resolved JSON form of a spec after includes/templates are applied.
    Dump {
        #[arg(short, long, default_value = "project.yml")]
        spec: PathBuf,
    },
    /// Validate that a spec can be loaded as an XcodeGen project.
    Validate {
        #[arg(short, long, default_value = "project.yml")]
        spec: PathBuf,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    match cli.command {
        Command::Generate {
            spec,
            project_root,
            output,
            variables,
        } => {
            let mut loader = SpecLoader::default();
            let project =
                loader.load_project(spec, project_root, variables.into_iter().collect())?;
            let generated = ProjectWriter::write(&project, output.as_deref())?;
            println!("{}", generated.project_path.display());
        }
        Command::Dump { spec } => {
            let spec = SpecFile::load(spec)?;
            let project = Project::from_spec(&spec)?;
            println!("{}", serde_json::to_string_pretty(&project.raw)?);
        }
        Command::Validate { spec } => {
            let mut loader = SpecLoader::default();
            let project = loader.load_project(spec, None, HashMap::new())?;
            println!(
                "{}: {} targets, {} schemes",
                project.name,
                project.targets.len(),
                project.scheme_specs.len()
            );
        }
    }
    Ok(())
}

fn parse_key_value(value: &str) -> Result<(String, String), String> {
    let Some((key, value)) = value.split_once('=') else {
        return Err("expected KEY=VALUE".to_owned());
    };
    Ok((key.to_owned(), value.to_owned()))
}
