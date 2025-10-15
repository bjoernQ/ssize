use std::{
    collections::{BTreeMap, HashSet},
    env,
    io::Cursor,
    path::{Component, PathBuf},
    process::Command,
};

use anyhow::bail;
use byteorder::{ReadBytesExt, LE};
use cargo_project::{Artifact, Profile, Project};
use clap::Parser;
use toml::Value;
use xmas_elf::{
    sections::SectionData,
    symbol_table::{Entry, Type},
    ElfFile,
};

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
    let functions = analyze_executable(&elf)?;

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

// ----from https://github.com/japaric/stack-sizes

/// Functions found after analyzing an executable
#[derive(Clone, Debug)]
pub struct Functions<'a> {
    /// Whether the addresses of these functions are 32-bit or 64-bit
    pub have_32_bit_addresses: bool,

    /// "undefined" symbols, symbols that need to be dynamically loaded
    pub undefined: HashSet<&'a str>,

    /// "defined" symbols, symbols with known locations (addresses)
    pub defined: BTreeMap<u64, Function<'a>>,
}

/// A symbol that represents a function (subroutine)
#[derive(Clone, Debug)]
pub struct Function<'a> {
    names: Vec<&'a str>,
    size: u64,
    stack: Option<u64>,
}

impl<'a> Function<'a> {
    /// Returns the (mangled) name of the function and its aliases
    pub fn names(&self) -> &[&'a str] {
        &self.names
    }

    /// Returns the size of this subroutine in bytes
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Returns the stack usage of the function in bytes
    pub fn stack(&self) -> Option<u64> {
        self.stack
    }
}

// is this symbol a tag used to delimit code / data sections within a subroutine?
fn is_tag(name: &str) -> bool {
    name == "$a" || name == "$t" || name == "$d" || {
        (name.starts_with("$a.") || name.starts_with("$d.") || name.starts_with("$t."))
            && name.splitn(2, '.').nth(1).unwrap().parse::<u64>().is_ok()
    }
}

/// Parses an executable ELF file and returns a list of functions and their stack usage
pub fn analyze_executable(elf: &[u8]) -> anyhow::Result<Functions<'_>> {
    let elf = &ElfFile::new(elf).map_err(anyhow::Error::msg)?;

    let mut have_32_bit_addresses = false;
    let (undefined, mut defined) = if let Some(section) = elf.find_section_by_name(".symtab") {
        match section.get_data(elf).map_err(anyhow::Error::msg)? {
            SectionData::SymbolTable32(entries) => {
                have_32_bit_addresses = true;

                process_symtab_exec(entries, elf)?
            }

            SectionData::SymbolTable64(entries) => process_symtab_exec(entries, elf)?,
            _ => bail!("malformed .symtab section"),
        }
    } else {
        (HashSet::new(), BTreeMap::new())
    };

    if let Some(stack_sizes) = elf.find_section_by_name(".stack_sizes") {
        let data = stack_sizes.raw_data(elf);
        let end = data.len() as u64;
        let mut cursor = Cursor::new(data);

        while cursor.position() < end {
            let address = if have_32_bit_addresses {
                u64::from(cursor.read_u32::<LE>()?)
            } else {
                cursor.read_u64::<LE>()?
            };
            let stack = leb128::read::unsigned(&mut cursor)?;

            // NOTE try with the thumb bit both set and clear
            if let Some(sym) = defined.get_mut(&(address | 1)) {
                sym.stack = Some(stack);
            } else if let Some(sym) = defined.get_mut(&(address & !1)) {
                sym.stack = Some(stack);
            } else {
                // ignore this
                // unreachable!()
            }
        }
    }

    Ok(Functions {
        have_32_bit_addresses,
        defined,
        undefined,
    })
}

fn process_symtab_exec<'a, E>(
    entries: &'a [E],
    elf: &ElfFile<'a>,
) -> anyhow::Result<(HashSet<&'a str>, BTreeMap<u64, Function<'a>>)>
where
    E: Entry + core::fmt::Debug,
{
    let mut defined = BTreeMap::new();
    let mut maybe_aliases = BTreeMap::new();
    let mut undefined = HashSet::new();

    for entry in entries {
        let ty = entry.get_type();
        let value = entry.value();
        let size = entry.size();
        let name = entry.get_name(&elf);

        if ty == Ok(Type::Func) {
            let name = name.map_err(anyhow::Error::msg)?;

            if value == 0 && size == 0 {
                undefined.insert(name);
            } else {
                defined
                    .entry(value)
                    .or_insert(Function {
                        names: vec![],
                        size,
                        stack: None,
                    })
                    .names
                    .push(name);
            }
        } else if ty == Ok(Type::NoType) {
            if let Ok(name) = name {
                if !is_tag(name) {
                    maybe_aliases.entry(value).or_insert(vec![]).push(name);
                }
            }
        }
    }

    for (value, alias) in maybe_aliases {
        // try with the thumb bit both set and clear
        if let Some(sym) = defined.get_mut(&(value | 1)) {
            sym.names.extend(alias);
        } else if let Some(sym) = defined.get_mut(&(value & !1)) {
            sym.names.extend(alias);
        }
    }

    Ok((undefined, defined))
}
