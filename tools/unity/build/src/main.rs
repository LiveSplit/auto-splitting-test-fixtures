//! Builds one Unity version's fixture assets: the fixture player per
//! variant, whole, zipped and hashed, with the manifest entries printed to
//! paste into manifest.json.

use std::{
    fmt, fs, io,
    io::Write,
    path::{Path, PathBuf},
    process::{self, Command},
    time::{Duration, Instant},
};

use clap::{Parser, ValueEnum};
use sha2::{Digest, Sha256};
use zip::{write::SimpleFileOptions, ZipWriter};

mod modules;
mod toolset;

const RELEASES: &str =
    "https://github.com/LiveSplit/auto-splitting-test-fixtures/releases/download";

/// A platform a player is built for.
#[derive(Copy, Clone, PartialEq, Eq, ValueEnum)]
enum Platform {
    /// Windows, 64-bit
    WinX64,
    /// Windows, 32-bit. Needs an editor that still ships a 32-bit player
    WinX86,
    /// Linux, 64-bit. Needs the Linux Build Support module
    LinuxX64,
    /// macOS. Needs the Mac Build Support module
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
enum Backend {
    /// The old Mono runtime, mono.dll. Editors before 2019.1
    MonoLegacy,
    /// The Mono runtime built with the Boehm collector, mono-2.0-bdwgc.dll. Editors from 2017.1 on
    MonoBdwgc,
    /// IL2CPP, Release C++ configuration. Needs the IL2CPP module for the platform
    Il2cppRelease,
    /// IL2CPP, Master C++ configuration. Editors from 2018.3 on
    Il2cppMaster,
}

impl Backend {
    fn is_mono(self) -> bool {
        matches!(self, Backend::MonoLegacy | Backend::MonoBdwgc)
    }

    /// Whether the editor can build this backend at all.
    fn offered_by(self, version: &str) -> bool {
        match self {
            Backend::MonoLegacy => major_minor(version) < (2019, 1),
            Backend::MonoBdwgc => major_minor(version) >= (2017, 1),
            // The Master configuration arrives in 2018.3.
            Backend::Il2cppMaster => major_minor(version) >= (2018, 3),
            Backend::Il2cppRelease => true,
        }
    }

    /// Whether the editor has the Mono runtime toggle, which it does from
    /// 2017.1 through 2018.4.
    fn toggles_runtime(version: &str) -> bool {
        ((2017, 1)..(2019, 1)).contains(&major_minor(version))
    }
}

/// One variant of a version, written `<platform>-<backend>`, such as
/// `win-x64-mono-bdwgc` or `linux-x64-il2cpp-release`. The build script
/// gets the name and reads the backend back out of it.
#[derive(Copy, Clone, PartialEq, Eq)]
struct Variant {
    platform: Platform,
    backend: Backend,
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
        write!(f, "{}-{}", name_of(self.platform), name_of(self.backend))
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
fn wanted_stem(backend: Backend, stem: &str) -> bool {
    if backend.is_mono() {
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
/// structures into the game assembly, so its symbols hold the layouts a
/// walk over that build has to know.
fn wanted_symbols(backend: Backend, name: &str) -> bool {
    name.strip_suffix(".pdb")
        .or_else(|| name.strip_suffix(".debug"))
        .map(|stem| stem.strip_suffix("_s").unwrap_or(stem))
        .is_some_and(|stem| wanted_stem(backend, stem))
}

/// Reads the version of the linker that wrote a PE file out of its
/// optional header.
fn linker_version(pe: &[u8]) -> Option<(u8, u8)> {
    let at = u32::from_le_bytes(pe.get(0x3C..0x40)?.try_into().ok()?) as usize;
    let optional = pe.get(at + 24..at + 28)?;
    Some((optional[2], optional[3]))
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
        &[],
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

    // We open the zip we just wrote and read every file in it. If a run
    // dies while it writes, we end up with half a zip, and the size and
    // hash we print for that half zip look just like the size and hash of
    // a whole zip.
    let mut written = zip::ZipArchive::new(io::Cursor::new(&bytes))
        .unwrap_or_else(|error| fail(&format!("{} does not read back: {error}", asset.display())));
    if written.len() != files.len() {
        fail(&format!(
            "{} holds {} files, the build had {}",
            asset.display(),
            written.len(),
            files.len(),
        ));
    }
    for index in 0..written.len() {
        let mut entry = written.by_index(index).expect("a zip entry");
        io::copy(&mut entry, &mut io::sink()).unwrap_or_else(|error| {
            fail(&format!("{} in {}: {error}", entry.name(), asset.display()))
        });
    }

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
/// carries the build report and the engine's own version lines, which
/// show what produced an asset.
fn run_editor(editor: &Path, log: &Path, args: &[String], env: &[(&str, &Path)]) -> Duration {
    let mut command = Command::new(editor);
    command
        .envs(env.iter().copied())
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
#[command(after_help = "Examples:
  Every variant the editor has modules for:
    unity-fixtures -e C:/Unity/6000.5.10f1/Editor/Unity.exe -o D:/fixtures

  4 players, every platform given with every backend given:
    unity-fixtures -e C:/Unity/6000.5.10f1/Editor/Unity.exe -o D:/fixtures -p win-x64 linux-x64 -b mono-bdwgc il2cpp-release")]
struct Args {
    /// Path to the editor binary: Editor/Unity.exe under a Hub install on
    /// Windows, Editor/Unity on Linux, Unity.app/Contents/MacOS/Unity on macOS
    #[arg(short, long, value_name = "PATH")]
    editor: PathBuf,

    /// Folder to build into. Each version gets its own folder inside, with
    /// the project, the players, the assets and the logs
    #[arg(short, long, value_name = "FOLDER")]
    out: PathBuf,

    /// The editor version, such as 6000.5.10f1, for when the editor path
    /// doesn't show it
    #[arg(long, value_name = "VERSION")]
    editor_version: Option<String>,

    /// Platforms to build for. Without it, every platform the editor has the
    /// module for
    #[arg(short, long = "platform", num_args = 1.., value_name = "PLATFORM")]
    platforms: Vec<Platform>,

    /// Backends to build. Without it, every backend the editor offers
    #[arg(short, long = "backend", num_args = 1.., value_name = "BACKEND")]
    backends: Vec<Backend>,
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
        .unwrap_or_else(|| fail("the editor path doesn't hold a version, pass --editor-version"));

    // Variants given with -p or -b have to build. Without either, a variant
    // whose module is missing is skipped with the reason.
    let given = !args.platforms.is_empty() || !args.backends.is_empty();
    let platforms = match args.platforms.is_empty() {
        true => Platform::value_variants().to_vec(),
        false => args.platforms,
    };
    let backends: Vec<_> = match args.backends.is_empty() {
        true => Backend::value_variants()
            .iter()
            .copied()
            .filter(|backend| backend.offered_by(&version))
            .collect(),
        false => args.backends,
    };
    if let Some(backend) = backends
        .iter()
        .find(|backend| !backend.offered_by(&version))
    {
        fail(&format!("{version} can't build {}", name_of(*backend)));
    }
    let mut variants = Vec::new();
    for &platform in &platforms {
        for &backend in &backends {
            let variant = Variant { platform, backend };
            match modules::missing(&args.editor, platform, backend) {
                None => variants.push(variant),
                Some(reason) if given => fail(&format!("can't build {variant}: {reason}")),
                Some(reason) => println!("skipping {variant}: {reason}"),
            }
        }
    }
    if variants.is_empty() {
        fail("nothing to build: this editor has none of the modules the variants need");
    }

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
            &[],
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
        let backend = variant.backend;

        // The runtime switch takes effect in a fresh editor session, so it
        // gets a run of its own before the build.
        if backend.is_mono() && Backend::toggles_runtime(&version) {
            run_editor(
                &args.editor,
                &workspace.join(format!("runtime-{}.log", name_of(backend))),
                &Run::SetRuntime(variant).args(&project, &version),
                &[],
            );
        }

        let build_dir = workspace.join(&name);
        let log = workspace.join(format!("unity-{version}-{name}.log"));
        let tools = toolset::for_variant(&version, variant, &workspace, &project);
        let env: Vec<(&str, &Path)> = tools
            .iter()
            .map(|tools| ("VS160COMNTOOLS", tools.as_path()))
            .collect();
        let took = run_editor(
            &args.editor,
            &log,
            &Run::Build {
                variant,
                out: &build_dir,
            }
            .args(&project, &version),
            &env,
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
        let is_player = |stem: &str| stem == "UnityPlayer" || stem == "fixture";

        // By role, not by count: a duplicate match for one role must not
        // stand in for another role's absence.
        let holds = |role: &dyn Fn(&str) -> bool| {
            shipped.iter().any(|(relative, _)| {
                let name = last(relative);
                role(stem(&name))
            })
        };
        let complete = holds(&is_player)
            && if backend.is_mono() {
                holds(&|stem| wanted_stem(backend, stem) && !is_player(stem))
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

        // MSVC 14.51 drops a guard in the garbage collector's table walk
        // when it compiles for x86, so a player it links crashes before
        // the game loads. 14.29 is the newest toolset measured good.
        if variant.platform == Platform::WinX86 && !backend.is_mono() {
            let assembly = fs::read(build_dir.join("GameAssembly.dll"))
                .unwrap_or_else(|_| fail(&format!("{name}: GameAssembly.dll unreadable")));
            let linker = linker_version(&assembly)
                .unwrap_or_else(|| fail(&format!("{name}: GameAssembly.dll is no PE file")));
            if linker >= (14, 30) {
                fail(&format!(
                    "{name}: GameAssembly.dll was linked by MSVC {}.{}; use a toolset below 14.30",
                    linker.0, linker.1
                ));
            }
        }

        // We leave the log out of the asset because it holds the machine that
        // ran the build and when.
        let mut files = shipped;
        files.extend(
            built
                .iter()
                .filter(|(relative, _)| relative.contains(NOT_SHIPPED))
                .filter(|(relative, _)| wanted_symbols(backend, &last(relative)))
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

#[cfg(test)]
mod tests {
    use super::{linker_version, modules::has_players, Backend, Platform};

    #[test]
    fn il2cpp_master_starts_at_2018_3() {
        assert!(!Backend::Il2cppMaster.offered_by("2018.1.0f1"));
        assert!(!Backend::Il2cppMaster.offered_by("2018.2.21f1"));
        assert!(Backend::Il2cppMaster.offered_by("2018.3.0f2"));
        assert!(Backend::Il2cppRelease.offered_by("2018.1.0f1"));
    }

    // Variations folders as the 2018.1.0f1 and 2022.2.0f1 editors ship them.
    #[test]
    fn players_are_found_by_their_variations_folder() {
        let old = ["win32_nondevelopment_mono", "win64_nondevelopment_il2cpp"];
        let new = [
            "il2cpp",
            "win64_player_nondevelopment_mono",
            "linux64_player_nondevelopment_il2cpp",
        ];
        let old = old.map(String::from);
        let new = new.map(String::from);
        assert!(has_players(&old, Platform::WinX86, Backend::MonoLegacy));
        assert!(has_players(&old, Platform::WinX64, Backend::Il2cppRelease));
        assert!(!has_players(&old, Platform::WinX86, Backend::Il2cppRelease));
        assert!(has_players(&new, Platform::WinX64, Backend::MonoBdwgc));
        assert!(has_players(&new, Platform::LinuxX64, Backend::Il2cppMaster));
        assert!(!has_players(&new, Platform::LinuxX64, Backend::MonoBdwgc));
        assert!(!has_players(&new, Platform::WinX86, Backend::MonoBdwgc));
    }

    #[test]
    fn the_linker_version_comes_from_the_optional_header() {
        // A DOS header pointing at a PE signature, a COFF header, then the
        // optional header with the linker version in its third and fourth
        // bytes.
        let mut pe = vec![0; 0x80];
        pe[0x3C..0x40].copy_from_slice(&0x40u32.to_le_bytes());
        pe[0x40..0x44].copy_from_slice(b"PE\0\0");
        pe[0x40 + 24..0x40 + 28].copy_from_slice(&[0x0B, 0x01, 14, 29]);
        assert_eq!(linker_version(&pe), Some((14, 29)));
        assert_eq!(linker_version(&pe[..0x50]), None);
    }
}
