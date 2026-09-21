//! Builds one Unity version's fixture assets: the fixture player per
//! variant, whole, zipped and hashed, with the manifest entries printed to
//! paste into manifest.json.

use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{self, Command},
    time::{Duration, Instant},
};

use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use zip::{write::SimpleFileOptions, ZipWriter};

const RELEASES: &str =
    "https://github.com/LiveSplit/auto-splitting-test-fixtures/releases/download";

/// A platform a player is built for.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Platform {
    WinX64,
    WinX86,
    LinuxX64,
    Mac,
}

impl Platform {
    /// The editor's name for the build target. Mac's was renamed in 2017.3.
    fn target(self, version: &str) -> &'static str {
        match self {
            Platform::WinX64 => "StandaloneWindows64",
            Platform::WinX86 => "StandaloneWindows",
            Platform::LinuxX64 => "StandaloneLinux64",
            Platform::Mac if major_minor(version) < (2017, 3) => "StandaloneOSXUniversal",
            Platform::Mac => "StandaloneOSX",
        }
    }
}

/// The scripting backend with what varies inside it. A Mono player ships
/// one of two runtimes: the legacy one, `mono.dll`, which is all there is
/// before 2017.1, or the newer one built with the Boehm collector,
/// `mono-2.0-bdwgc.dll`, which is all there is from 2019.1 on. An IL2CPP
/// player's C++ configuration changes its compiled code, which matters to
/// anyone matching signatures in it.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Flavor {
    MonoLegacy,
    MonoBdwgc,
    Il2cppRelease,
    Il2cppMaster,
}

impl Flavor {
    fn is_mono(self) -> bool {
        matches!(self, Flavor::MonoLegacy | Flavor::MonoBdwgc)
    }

    /// Whether the editor can build this flavor at all.
    fn offered_by(self, version: &str) -> bool {
        match self {
            Flavor::MonoLegacy => major_minor(version) < (2019, 1),
            Flavor::MonoBdwgc => major_minor(version) >= (2017, 1),
            _ => true,
        }
    }

    /// Whether the editor has the Mono runtime toggle, which it does from
    /// 2017.1 through 2018.4.
    fn toggles_runtime(version: &str) -> bool {
        ((2017, 1)..(2019, 1)).contains(&major_minor(version))
    }
}

/// One variant of a version, named `<platform>-<flavor>`, such as
/// `win-x64-mono-bdwgc` or `linux-x64-il2cpp-release`. The build script
/// gets the name and reads the flavor back out of it.
#[derive(Copy, Clone, PartialEq, Eq)]
struct Variant {
    platform: Platform,
    flavor: Flavor,
}

fn name_of<T: ValueEnum>(value: T) -> String {
    value
        .to_possible_value()
        .expect("a named value")
        .get_name()
        .to_string()
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}-{}", name_of(self.platform), name_of(self.flavor))
    }
}

fn fail(message: &str) -> ! {
    eprintln!("{message}");
    process::exit(1);
}

/// The name with the extension its platform gave it taken off.
fn stem(name: &str) -> &str {
    name.strip_suffix(".dll")
        .or_else(|| name.strip_suffix(".so"))
        .or_else(|| name.strip_suffix(".dylib"))
        .or_else(|| name.strip_suffix(".exe"))
        .unwrap_or(name)
}

/// The names of the files a walk reads, without the extension each platform
/// gives them: the player binary always, the runtime library on Mono, the
/// game assembly and the metadata file on IL2CPP. An asset holds the whole
/// build, and it must hold these.
fn wanted_stem(flavor: Flavor, stem: &str) -> bool {
    if flavor.is_mono() {
        matches!(
            stem,
            "UnityPlayer"
                | "mono"
                | "mono-2.0-bdwgc"
                | "libmono"
                | "libmono.0"
                | "libmonobdwgc-2.0"
        )
    } else {
        matches!(stem, "UnityPlayer" | "GameAssembly" | "global-metadata.dat")
    }
}

/// The symbols belonging to a file the tests read, as a PDB on Windows or
/// separated debug info elsewhere. IL2CPP compiles the runtime's own
/// structures into the game assembly, so its symbols name the layouts a
/// walk over that build has to know.
fn wanted_symbols(flavor: Flavor, name: &str) -> bool {
    name.strip_suffix(".pdb")
        .or_else(|| name.strip_suffix(".debug"))
        .map(|stem| stem.strip_suffix("_s").unwrap_or(stem))
        .is_some_and(|stem| wanted_stem(flavor, stem))
}

/// The directory a build puts beside the player for the files a game does
/// not ship, symbols among them.
const NOT_SHIPPED: &str = "_BackUpThisFolder_ButDontShipItWithYourGame";

/// Every file under the build, each with the path a shipped game would
/// hold it at.
fn walk(root: &Path, at: &str, into: &mut Vec<(String, PathBuf)>) {
    for entry in fs::read_dir(root).unwrap_or_else(|_| fail("build directory unreadable")) {
        let path = entry.expect("directory entry").path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let relative = match at {
            "" => name.to_string(),
            _ => format!("{at}/{name}"),
        };

        if path.is_dir() {
            walk(&path, &relative, into);
        } else {
            into.push((relative, path));
        }
    }
}

/// Copies a directory tree, keeping the relative paths.
fn copy_tree(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap_or_else(|_| fail("assets directory unreadable")) {
        let path = entry.expect("directory entry").path();
        let target = to.join(path.file_name().expect("a name"));
        if path.is_dir() {
            fs::create_dir_all(&target).expect("creating a directory");
            copy_tree(&path, &target);
        } else {
            fs::copy(&path, &target)
                .unwrap_or_else(|_| fail(&format!("copying {}", path.display())));
        }
    }
}

/// Sets up a fresh project: copies in the shipped scenes, scripts and meta
/// files, then has the editor turn on text serialization and turn off audio.
fn prepare(editor: &Path, workspace: &Path, project: &Path, tool: &Path, version: &str) {
    let assets = project.join("Assets");
    copy_tree(&tool.join("assets"), &assets);

    run_editor(
        editor,
        &workspace.join("prepare.log"),
        &Run::Prepare.args(project, version),
    );
}

/// Zips the files at their in-build relative paths and answers the
/// archive's size and digest.
fn archive(files: &[(String, PathBuf)], asset: &Path) -> (u64, String) {
    let mut zip = ZipWriter::new(fs::File::create(asset).expect("creating the asset"));
    for (relative, file) in files {
        let bytes = fs::read(file).expect("reading a built file");
        zip.start_file(relative, SimpleFileOptions::default())
            .expect("starting the zip entry");
        zip.write_all(&bytes).expect("writing the zip entry");
    }
    zip.finish().expect("finishing the asset");

    let bytes = fs::read(asset).expect("re-reading the asset");
    (bytes.len() as u64, format!("{:x}", Sha256::digest(&bytes)))
}

/// What one run of the editor does. All but the first go through the build
/// script's `-executeMethod` entry points.
enum Run<'a> {
    CreateProject,
    Prepare,
    SetRuntime(Variant),
    Build { variant: Variant, out: &'a Path },
}

impl Run<'_> {
    fn args(&self, project: &Path, version: &str) -> Vec<String> {
        let project = project.to_string_lossy().into_owned();
        let mut args: Vec<String> = match self {
            Run::CreateProject => return vec!["-createProject".into(), project],
            Run::Prepare => vec!["FixtureBuild.Prepare".into()],
            Run::SetRuntime(variant) => vec![
                "FixtureBuild.SetRuntime".into(),
                "-fixtureVariant".into(),
                variant.to_string(),
            ],
            Run::Build { variant, out } => vec![
                "FixtureBuild.Build".into(),
                "-buildTarget".into(),
                variant.platform.target(version).into(),
                "-fixtureOut".into(),
                out.to_string_lossy().into_owned(),
                "-fixtureVariant".into(),
                variant.to_string(),
            ],
        };
        args.splice(
            0..0,
            ["-projectPath".into(), project, "-executeMethod".into()],
        );
        args
    }
}

/// Runs the editor with its log kept beside whatever it produces. The log
/// carries the build report and the engine's own version lines, which is
/// what says how an asset came to be.
fn run_editor(editor: &Path, log: &Path, args: &[String]) -> Duration {
    let mut command = Command::new(editor);
    command
        .args(["-batchmode", "-nographics", "-quit"])
        .args(["-logFile", &log.to_string_lossy()])
        .args(args);
    println!("> {command:?}");

    let started = Instant::now();
    match command.status() {
        Ok(status) if status.success() => started.elapsed(),
        Ok(_) => fail(&format!("editor exited nonzero, see {}", log.display())),
        Err(error) => fail(&format!("editor would not start: {error}")),
    }
}

/// The version out of a Hub-style editor path, such as
/// `.../Hub/Editor/6000.5.8f1/Editor/Unity.exe`.
fn version_from(editor: &Path) -> Option<String> {
    editor.components().rev().find_map(|component| {
        let text = component.as_os_str().to_str()?;
        let looks_like_version = text.starts_with(|c: char| c.is_ascii_digit())
            && text.matches('.').count() == 2
            && text.contains(['a', 'b', 'f', 'p']);
        looks_like_version.then(|| text.to_string())
    })
}

/// Builds one Unity version's fixture assets: the fixture player per
/// variant, whole, zipped and hashed, with the manifest entries printed to
/// paste into manifest.json.
#[derive(Parser)]
struct Args {
    /// Path to the editor binary
    #[arg(short, long)]
    editor: PathBuf,

    /// Workspace the projects and players build into
    #[arg(short, long)]
    out: PathBuf,

    /// Editor version, when the editor path does not name it
    #[arg(long)]
    editor_version: Option<String>,

    /// Platforms to build for. Every platform without it, which needs
    /// every module installed
    #[arg(short, long = "platform", num_args = 1..)]
    platforms: Vec<Platform>,

    /// Flavors to build. Every flavor the editor offers without it
    #[arg(short, long = "flavor", num_args = 1..)]
    flavors: Vec<Flavor>,
}

/// The major and minor of a Unity version such as `2019.4.41f2`.
fn major_minor(version: &str) -> (u32, u32) {
    let mut parts = version.split(['.', 'a', 'b', 'f', 'p']);
    let mut next = || {
        parts
            .next()
            .and_then(|part| part.parse().ok())
            .unwrap_or(0u32)
    };
    (next(), next())
}

fn main() {
    let args = Args::parse();
    let version = args
        .editor_version
        .or_else(|| version_from(&args.editor))
        .unwrap_or_else(|| fail("editor path names no version, pass --editor-version"));

    let platforms = match args.platforms.is_empty() {
        true => Platform::value_variants().to_vec(),
        false => args.platforms,
    };
    let flavors: Vec<_> = match args.flavors.is_empty() {
        true => Flavor::value_variants()
            .iter()
            .copied()
            .filter(|flavor| flavor.offered_by(&version))
            .collect(),
        false => args.flavors,
    };
    if let Some(flavor) = flavors.iter().find(|flavor| !flavor.offered_by(&version)) {
        fail(&format!("{version} can't build {}", name_of(*flavor)));
    }
    let variants = platforms.iter().flat_map(|&platform| {
        flavors
            .iter()
            .map(move |&flavor| Variant { platform, flavor })
    });

    let workspace = args.out.join(&version);
    let project = workspace.join("project");
    let tool = Path::new(env!("CARGO_MANIFEST_DIR"));
    let fresh = !project.exists();
    if fresh {
        fs::create_dir_all(&workspace).expect("creating the workspace");
        run_editor(
            &args.editor,
            &workspace.join("create-project.log"),
            &Run::CreateProject.args(&project, &version),
        );
    }

    // The build script is copied on every run, so a change to it works
    // without a fresh project.
    let editor_scripts = project.join("Assets").join("Editor");
    fs::create_dir_all(&editor_scripts).expect("creating Assets/Editor");
    fs::copy(
        tool.join("editor/FixtureBuild.cs"),
        editor_scripts.join("FixtureBuild.cs"),
    )
    .expect("copying FixtureBuild.cs");

    if fresh {
        prepare(&args.editor, &workspace, &project, tool, &version);
    }

    let mut manifest = Vec::new();
    for variant in variants {
        let name = variant.to_string();
        let flavor = variant.flavor;

        // The runtime switch takes effect in a fresh editor session, so it
        // gets a run of its own before the build.
        if flavor.is_mono() && Flavor::toggles_runtime(&version) {
            run_editor(
                &args.editor,
                &workspace.join(format!("runtime-{}.log", name_of(flavor))),
                &Run::SetRuntime(variant).args(&project, &version),
            );
        }

        let build_dir = workspace.join(&name);
        let log = workspace.join(format!("unity-{version}-{name}.log"));
        let took = run_editor(
            &args.editor,
            &log,
            &Run::Build {
                variant,
                out: &build_dir,
            }
            .args(&project, &version),
        );

        let mut built = Vec::new();
        walk(&build_dir, "", &mut built);
        built.sort();

        let last = |relative: &str| relative.rsplit('/').next().unwrap_or(relative).to_string();

        // Everything a shipped game holds, at the paths a shipped game uses.
        // The build puts the files a game does not ship in a directory of
        // its own beside the player, and only the symbols come from there.
        let shipped: Vec<_> = built
            .iter()
            .filter(|(relative, _)| !relative.contains(NOT_SHIPPED))
            .cloned()
            .collect();

        // Players from before the engine split one out are monolithic, on
        // Windows and Mac until 2017 and on Linux until 2019: no UnityPlayer
        // library exists, and the executable itself is the player.
        let names_a_player = |stem: &str| stem == "UnityPlayer" || stem == "fixture";

        // By role, not by count: a duplicate match for one role must not
        // stand in for another role's absence.
        let holds = |role: &dyn Fn(&str) -> bool| {
            shipped.iter().any(|(relative, _)| {
                let name = last(relative);
                role(stem(&name))
            })
        };
        let complete = holds(&names_a_player)
            && if flavor.is_mono() {
                holds(&|stem| wanted_stem(flavor, stem) && !names_a_player(stem))
            } else {
                holds(&|stem| stem == "GameAssembly")
                    && holds(&|stem| stem == "global-metadata.dat")
            };
        if !complete {
            fail(&format!(
                "{name}: build under {} is missing binaries the asset needs",
                build_dir.display(),
            ));
        }

        // We leave the log out of the asset because it names the machine that
        // ran the build and when.
        let mut files = shipped;
        files.extend(
            built
                .iter()
                .filter(|(relative, _)| wanted_symbols(flavor, &last(relative)))
                .cloned(),
        );

        let asset = workspace.join(format!("unity-{version}-{name}.zip"));
        let (size, digest) = archive(&files, &asset);

        let entry = serde_json::json!({
            "engine": "unity",
            "version": version,
            "variant": name,
            "asset": format!("{RELEASES}/unity-{version}/unity-{version}-{name}.zip"),
            "size": size,
            "sha256": digest,
        });

        manifest.push(entry);
        println!("built {} in {}s", asset.display(), took.as_secs());
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&manifest).expect("rendering entries")
    );
}
