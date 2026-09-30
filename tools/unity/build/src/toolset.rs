//! Keeps IL2CPP builds on an MSVC toolset older than 14.30. Newer ones build
//! x86 players that crash before the game loads, and on 2022.2 they don't
//! compile libil2cpp at all. Editors from 2021.2 on take the newest toolset
//! of every Visual Studio install they find, including the one
//! `VS160COMNTOOLS` points at, so the tool points it at a folder shaped like
//! an install that holds only the right toolset.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{fail, major_minor, Platform, Variant};

/// Answers the folder `VS160COMNTOOLS` takes for a Windows IL2CPP build on
/// an editor from 2021.2 on, or none for any other build.
pub(crate) fn for_variant(
    version: &str,
    variant: Variant,
    workspace: &Path,
    project: &Path,
) -> Option<PathBuf> {
    let windows = matches!(variant.platform, Platform::WinX64 | Platform::WinX86);
    if !windows || variant.backend.is_mono() || major_minor(version) < (2021, 2) {
        return None;
    }
    let (tools, toolset) = toolset_shim(workspace);
    forget_other_toolset(project, &toolset);
    Some(tools)
}

/// The version in a toolset directory's name, such as `14.29.30133`.
fn toolset_version(toolset: &Path) -> Option<Vec<u32>> {
    toolset
        .file_name()?
        .to_str()?
        .split('.')
        .map(|part| part.parse().ok())
        .collect()
}

/// Picks the newest MSVC toolset whose major.minor is below the ceiling.
fn newest_toolset_below(ceiling: (u32, u32), toolsets: &[PathBuf]) -> Option<PathBuf> {
    toolsets
        .iter()
        .filter_map(|toolset| Some((toolset_version(toolset)?, toolset)))
        .filter(|(version, _)| version.len() >= 2 && (version[0], version[1]) < ceiling)
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, toolset)| toolset.clone())
}

/// What to install when no usable toolset is on the machine.
const INSTALL_TOOLSET: &str = "In the Visual Studio Installer, modify any install (Build Tools \
    is enough), open Individual components and add \"MSVC v142 - VS 2019 C++ x64/x86 build \
    tools (Latest)\". Newer toolsets can stay installed.";

/// Every MSVC toolset in every Visual Studio install vswhere knows about.
fn installed_toolsets() -> Vec<PathBuf> {
    let program_files = std::env::var_os("ProgramFiles(x86)").unwrap_or_default();
    let vswhere = Path::new(&program_files).join("Microsoft Visual Studio/Installer/vswhere.exe");
    let Ok(output) = Command::new(&vswhere)
        .args(["-all", "-products", "*", "-property", "installationPath"])
        .output()
    else {
        fail(&format!(
            "IL2CPP builds for Windows need Visual Studio with the C++ tools, and {} isn't \
             there, so no Visual Studio seems to be installed. {INSTALL_TOOLSET}",
            vswhere.display()
        ));
    };
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|install| fs::read_dir(Path::new(install.trim()).join("VC/Tools/MSVC")).ok())
        .flatten()
        .filter_map(|entry| Some(entry.ok()?.path()))
        .collect()
}

/// Links `link` to `target` as a junction, which needs no admin rights.
/// mklink takes backslashes only.
fn junction(link: &Path, target: &Path) {
    let backslashed = |path: &Path| path.to_string_lossy().replace('/', "\\");
    let made = Command::new("cmd")
        .arg("/c")
        .arg("mklink")
        .arg("/J")
        .arg(backslashed(link))
        .arg(backslashed(target))
        .output()
        .unwrap_or_else(|error| fail(&format!("mklink would not start: {error}")));
    if !made.status.success() {
        fail(&format!(
            "could not link {} to {}: {}",
            link.display(),
            target.display(),
            String::from_utf8_lossy(&made.stdout).trim()
        ));
    }
}

/// The newest MSVC toolset that builds working IL2CPP players.
const TOOLSET_CEILING: (u32, u32) = (14, 30);

/// Makes a folder shaped like a Visual Studio install that holds only the
/// newest toolset below 14.30, and answers the path `VS160COMNTOOLS` takes.
/// Editors from 2021.2 on take the newest toolset of every install they
/// find, so the one in this folder goes in as 14.99.0 to win. The build also
/// asks the install for the C runtime it ships beside the player, so the
/// redist goes in too, under the toolset's real version.
fn toolset_shim(workspace: &Path) -> (PathBuf, PathBuf) {
    let toolsets = installed_toolsets();
    let toolset = newest_toolset_below(TOOLSET_CEILING, &toolsets).unwrap_or_else(|| {
        let found: Vec<_> = toolsets
            .iter()
            .filter_map(|toolset| Some(toolset.file_name()?.to_string_lossy().into_owned()))
            .collect();
        let found = match found.is_empty() {
            true => "none".to_string(),
            false => found.join(", "),
        };
        fail(&format!(
            "IL2CPP builds for Windows need an MSVC toolset older than 14.30, because newer \
             ones build players that crash or don't compile. Installed toolsets: {found}. \
             {INSTALL_TOOLSET}"
        ))
    });
    let name = toolset.file_name().expect("a toolset name");

    let shim = workspace.join("toolset");
    if shim.exists() {
        fs::remove_dir_all(&shim).expect("removing the old toolset folder");
    }
    fs::create_dir_all(shim.join("VC/Tools/MSVC")).expect("creating the toolset folder");
    fs::create_dir_all(shim.join("Common7/Tools")).expect("creating the toolset folder");
    junction(&shim.join("VC/Tools/MSVC/14.99.0"), &toolset);

    let vc = toolset
        .ancestors()
        .nth(3)
        .expect("a toolset under VC/Tools/MSVC");
    let redist = vc.join("Redist/MSVC").join(name);
    if redist.exists() {
        fs::create_dir_all(shim.join("VC/Redist/MSVC")).expect("creating the redist folder");
        junction(&shim.join("VC/Redist/MSVC").join(name), &redist);
        fs::create_dir_all(shim.join("VC/Auxiliary/Build")).expect("creating the auxiliary folder");
        fs::write(
            shim.join("VC/Auxiliary/Build/Microsoft.VCRedistVersion.default.txt"),
            name.to_string_lossy().as_bytes(),
        )
        .expect("writing the redist version");
    }

    println!("compiling IL2CPP with MSVC {}", name.to_string_lossy());
    (shim.join("Common7/Tools/"), toolset)
}

/// Clears what Bee built with another toolset. Bee keeps the compiler's
/// path in its cache, so a project built once would keep using it.
fn forget_other_toolset(project: &Path, toolset: &Path) {
    let bee = project.join("Library/Bee");
    let note = project.join("Library/toolset.txt");
    let built = fs::read_to_string(&note).unwrap_or_default();
    if bee.exists() && built.trim() != toolset.to_string_lossy() {
        fs::remove_dir_all(&bee).expect("clearing Library/Bee");
    }
    fs::write(&note, toolset.to_string_lossy().as_bytes()).expect("writing Library/toolset.txt");
}

#[cfg(test)]
mod tests {
    use super::newest_toolset_below;
    use std::path::PathBuf;

    #[test]
    fn the_toolset_is_the_newest_below_the_ceiling() {
        let install = |version: &str| PathBuf::from(format!("VS/VC/Tools/MSVC/{version}"));
        let toolsets = [
            install("14.29.30133"),
            install("14.51.36231"),
            install("14.16.27023"),
            install("not-a-version"),
        ];
        assert_eq!(
            newest_toolset_below((14, 30), &toolsets),
            Some(install("14.29.30133"))
        );
        assert_eq!(newest_toolset_below((14, 16), &toolsets), None);
    }
}
