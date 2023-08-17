use std::{
    env,
    path::{Component, PathBuf},
    process::Command,
};

use anyhow::bail;
use cargo_project::{Artifact, Profile, Project};
use clap::Parser;
use toml::Value;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Build only the specified binary
    #[arg(long, value_name = "BIN")]
    bin: Option<String>,

    /// Build only the specified example
    #[arg(long, value_name = "NAME")]
    example: Option<String>,

    /// Space-separated list of features to activate
    #[arg(long, value_name = "FEATURES")]
    features: Option<String>,

    /// Activate all available features
    #[arg(long)]
    all_features: bool,

    /// Only show functions whose stack size is greater or equals to this
    #[arg(long)]
    min_stack: Option<u64>,

    /// Override the path of the resulting ELF - use if for some reason it's not found
    #[arg(long)]
    out_override: Option<PathBuf>,
}

fn main() -> anyhow::Result<()> {
    env_logger::init();

    let args = Args::parse();

    let meta = rustc_version::version_meta()?;
    let host = meta.host;
    let cwd = env::current_dir()?;
    let project = Project::query(cwd)?;
    let target = project.target().unwrap_or(&host);

    let config = std::fs::read_to_string(".cargo/config.toml");
    let rustflags = if let Ok(content) = config {
        let value = content.parse::<Value>()?;
        if let Some(build) = value["build"].as_table() {
            if build.contains_key("rustflags") {
                let rf = build["rustflags"].as_array();
                if let Some(rf) = rf {
                    let mut rf_str = String::new();
                    for v in rf {
                        rf_str.push_str("\"");
                        rf_str.push_str(&v.as_str().unwrap().replace("\"", "\\\""));
                        rf_str.push_str("\",");
                    }
                    rf_str
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            }
        } else {
            "".to_string()
        }
    } else {
        "".to_string()
    };

    let mut tmp_file = std::env::temp_dir();
    let tmp_dir = tmp_file.to_owned();
    let tmp_dir = tmp_dir.to_str().unwrap().replace("\\", "/");
    tmp_file.push("2374972342390lnk.x");
    std::fs::write(
        &tmp_file,
        "
    SECTIONS
    {
      /* `INFO` makes the section not allocatable so it won't be loaded into memory */
      .stack_sizes (INFO) :
      {
        KEEP(*(.stack_sizes));
      }
    }    
    ",
    )?;

    let mut cargo_args: Vec<String> = Vec::new();
    cargo_args.push(String::from("--config"));
    cargo_args.push(String::from(&format!("target.{target}.rustflags=[{rustflags} \"-Z\", \"emit-stack-sizes\",\"-C\", \"link-arg=-T2374972342390lnk.x\",  \"-C\", \"link-arg=-L{}\"]", tmp_dir)));
    cargo_args.push(String::from("build"));
    cargo_args.push(String::from("--release"));

    if args.all_features {
        cargo_args.push(String::from("--all-features"));
    } else if let Some(features) = &args.features {
        cargo_args.push(format!("--features={}", features));
    }

    let file = match (&args.example, &args.bin) {
        (Some(f), None) => f,
        (None, Some(f)) => f,
        _ => bail!("Please specify either --example <NAME> or --bin <NAME>."),
    };

    if args.example.is_some() {
        cargo_args.push(format!("--example={}", file));
    }

    if args.bin.is_some() {
        cargo_args.push(format!("--bin={}", file));
    }

    let cargo_res = Command::new("cargo").args(&cargo_args[..]).status();

    std::fs::remove_file(&tmp_file)?;

    cargo_res?;

    let mut path: PathBuf = if let Some(binary) = args.out_override {
        binary
    } else if args.example.is_some() {
        project.path(
            Artifact::Example(&file),
            Profile::Release,
            Some(target),
            &host,
        )?
    } else {
        project.path(Artifact::Bin(&file), Profile::Release, Some(target), &host)?
    };

    // the project crate seems to have problems with workspaces (at least on Windows) ... if the file isn't there let's guess one level up
    // otherwise the user can still specify `out_override`
    if !path.exists() {
        let mut parts: Vec<Component> = path.components().collect();
        let target_index = parts
            .iter()
            .position(|c| match c {
                Component::Normal(name) if name.to_str() == Some("target") => true,
                _ => false,
            })
            .unwrap_or(usize::MAX);

        if target_index != usize::MAX && target_index > 0 {
            parts.remove(target_index - 1);

            let mut tmp = PathBuf::new();
            for c in parts {
                tmp.push(c);
            }

            path = tmp;
        }
    }

    let elf = std::fs::read(path)?;
    let functions = stack_sizes::analyze_executable(&elf)?;

    let mut functions: Vec<(String, u64, u64)> = functions
        .defined
        .iter()
        .map(|(_, f)| {
            let mut fname = String::new();
            for name in f.names() {
                if name.len() > 0 {
                    fname.push_str(&format!("{} ", rustc_demangle::demangle(name)));
                }
            }
            (fname, f.size(), f.stack().unwrap_or(0))
        })
        .collect();

    functions.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

    let min_stack = args.min_stack.unwrap_or(0);

    println!("Code  Stack Name");
    for (name, code_size, stack_size) in functions
        .iter()
        .filter(|(_name, _code_size, stack_size)| stack_size >= &min_stack)
    {
        println!("{:5} {:5} {}", code_size, stack_size, name);
    }

    Ok(())
}
