use std::{env, path::PathBuf, process::Command};

use anyhow::bail;
use cargo_project::{Artifact, Profile, Project};
use clap::Parser;

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

    #[arg(long)]
    min_stack: Option<u64>,

    /// binary - can be used if it's not found
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
    cargo_args.push(String::from(&format!("target.{target}.rustflags=[\"-Z\", \"emit-stack-sizes\",\"-C\", \"link-arg=-T2374972342390lnk.x\",  \"-C\", \"link-arg=-L{}\"]", tmp_dir)));
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

    let path: PathBuf = if let Some(binary) = args.out_override {
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

    // TODO doesn't match the real path in the esp-wifi workspace? it seems it doesn't match the name in the workspace-members
    // out-override is a workaround

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
