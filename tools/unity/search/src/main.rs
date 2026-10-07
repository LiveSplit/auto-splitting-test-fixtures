//! Finds the Unity patches where a runtime layout or the code around it
//! changes. No editor gets installed. The tool streams each patch's files
//! out of the packages Unity's release API lists and keeps only the files a
//! measurement needs.

use std::{path::PathBuf, process};

use clap::{Parser, Subcommand};

mod il2cpp;
mod package;
mod releases;
mod version;

#[derive(Parser)]
#[command(after_help = "Examples:
  Every release of a minor:
    unity-search releases 2019.4

  The IL2CPP headers of one release, out of its editor package:
    unity-search fetch 2019.4.41f2 editor -o D:/headers libil2cpp/il2cpp-class-internals.h

  The 64-bit Linux player of one release. A package holds one player per
  variation, so the path names the variation:
    unity-search fetch 2018.2.0f1 linux -o D:/players linux64_withgfx_nondevelopment_mono/LinuxPlayer

  The IL2CPP layout of one release, as entries for asr's il2cpp/builds.rs:
    unity-search il2cpp 2019.4.41f2 -w D:/work")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Lists every full release of a minor, oldest first
    Releases {
        /// The major and minor, such as 2019.4
        minor: String,
    },
    /// Streams the files asked for out of one release's package
    Fetch {
        /// The release, such as 2019.4.41f2
        version: String,
        /// Which of the release's packages to read
        package: releases::Package,
        /// Folder the files land in, each at the path it was asked for
        #[arg(short, long, value_name = "FOLDER")]
        out: PathBuf,
        /// Ends of the paths inside the package to keep, such as
        /// libil2cpp/il2cpp-class-internals.h. The tool keeps the first
        /// entry that ends with a path. When a package holds the same file
        /// name in several folders, put the folder in the path. A path
        /// ending in / keeps every .h file under that folder
        #[arg(required = true, value_name = "PATH")]
        files: Vec<String>,
    },
    /// Measures the IL2CPP struct layout of one release from its editor's
    /// libil2cpp headers, for x64 and x86, and prints both as entries for
    /// asr's il2cpp/builds.rs
    Il2cpp {
        /// The release, such as 2019.4.41f2
        version: String,
        /// Folder for the headers and the probe. Each release gets its own
        /// folder inside
        #[arg(short, long, value_name = "FOLDER")]
        work: PathBuf,
    },
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(1)
}

fn main() {
    match Args::parse().command {
        Command::Releases { minor } => {
            let found = releases::of_minor(&minor).unwrap_or_else(|e| fail(&e.to_string()));
            for version in found {
                println!("{version}");
            }
        }
        Command::Fetch {
            version,
            package,
            out,
            files,
        } => {
            let record = releases::record(&version)
                .unwrap_or_else(|e| fail(&e.to_string()))
                .unwrap_or_else(|| fail(&format!("{version} is not a release the API knows")));
            let url = releases::package_url(&record, package)
                .unwrap_or_else(|| fail(&format!("{version} has no {package:?} package")));
            let taken = package::take(&url, &files, &out).unwrap_or_else(|e| fail(&e.to_string()));
            println!(
                "read {} MB of {}",
                taken.read / 1_000_000,
                url.rsplit('/').next().unwrap_or(&url)
            );
            if let Some(missing) = taken.missing.first() {
                fail(&format!("{missing} is not in the package"));
            }
        }
        Command::Il2cpp { version, work } => {
            let toolset = il2cpp::Toolset::find().unwrap_or_else(|e| fail(&e.to_string()));
            let record = releases::record(&version)
                .unwrap_or_else(|e| fail(&e.to_string()))
                .unwrap_or_else(|| fail(&format!("{version} is not a release the API knows")));
            let url = releases::package_url(&record, releases::Package::Editor)
                .unwrap_or_else(|| fail(&format!("{version} has no editor package")));
            let folder = work.join(&version);
            // Every header of libil2cpp, since the struct header's includes
            // grow over the releases, plus the version checks.
            let files: Vec<String> = [il2cpp::HEADERS]
                .iter()
                .chain(il2cpp::VERSION_CHECKS)
                .map(|f| f.to_string())
                .collect();
            package::take(&url, &files, &folder).unwrap_or_else(|e| fail(&e.to_string()));
            let headers = folder.join(il2cpp::HEADERS);
            if !headers.is_dir() {
                fail(&format!("{version} has no libil2cpp folder in its editor"));
            }
            let metadata = il2cpp::metadata_version(&headers);
            let (x64, x86) = il2cpp::measure(&toolset, &headers, &folder)
                .unwrap_or_else(|e| fail(&e.to_string()));
            println!("{}", il2cpp::entry(&version, metadata, 64, &x64));
            println!("{}", il2cpp::entry(&version, metadata, 32, &x86));
        }
    }
}
