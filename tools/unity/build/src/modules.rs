//! Checks whether an editor has the players a variant builds from. Every
//! build support module Unity Hub installs is a folder of player variations,
//! so the tool can report a missing module before it starts the editor.

use std::{fs, path::Path};

use crate::{Backend, Platform};

/// Checks whether a list of player variations holds one for the platform
/// and the backend.
pub(crate) fn has_players(variations: &[String], platform: Platform, backend: Backend) -> bool {
    let prefixes: &[&str] = match platform {
        Platform::WinX64 => &["win64"],
        Platform::WinX86 => &["win32"],
        Platform::LinuxX64 => &["linux64"],
        Platform::Mac => &["mac", "universal"],
    };
    let runtime = if backend.is_mono() { "mono" } else { "il2cpp" };
    variations.iter().any(|variation| {
        prefixes.iter().any(|prefix| variation.starts_with(prefix))
            && variation.contains("nondevelopment")
            && variation.ends_with(runtime)
    })
}

/// Finds the Unity Hub module a variant needs.
fn module(platform: Platform, backend: Backend) -> &'static str {
    match (platform, backend.is_mono()) {
        (Platform::WinX64 | Platform::WinX86, true) => "Windows Build Support (Mono)",
        (Platform::WinX64 | Platform::WinX86, false) => "Windows Build Support (IL2CPP)",
        (Platform::LinuxX64, true) => "Linux Build Support (Mono)",
        (Platform::LinuxX64, false) => "Linux Build Support (IL2CPP)",
        (Platform::Mac, true) => "Mac Build Support (Mono)",
        (Platform::Mac, false) => "Mac Build Support (IL2CPP)",
    }
}

/// Returns why the editor can't build a variant, or none when it can.
/// An editor laid out in a way this doesn't know is taken at its word.
pub(crate) fn missing(editor: &Path, platform: Platform, backend: Backend) -> Option<String> {
    if platform == Platform::Mac && !backend.is_mono() && !cfg!(target_os = "macos") {
        return Some("Mac IL2CPP players only build on a Mac".into());
    }

    let folder = match platform {
        Platform::WinX64 | Platform::WinX86 => "windowsstandalonesupport",
        Platform::LinuxX64 => "linuxstandalonesupport",
        Platform::Mac => "macstandalonesupport",
    };
    // Data/PlaybackEngines beside Unity.exe on Windows and Linux, and
    // Contents/PlaybackEngines two levels up from the binary on macOS.
    let dir = editor.parent()?;
    let engines = [
        dir.join("Data/PlaybackEngines"),
        dir.parent()?.join("PlaybackEngines"),
    ]
    .into_iter()
    .find(|engines| engines.is_dir())?;

    let installed = fs::read_dir(&engines)
        .ok()?
        .filter_map(|entry| entry.ok())
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(folder)
        })
        .and_then(|entry| fs::read_dir(entry.path().join("Variations")).ok())
        .map(|variations| {
            let names: Vec<String> = variations
                .filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned()))
                .collect();
            has_players(&names, platform, backend)
        })
        .unwrap_or(false);

    (!installed).then(|| {
        format!(
            "the \"{}\" module isn't installed for this editor; add it in Unity Hub under the \
             editor's Add modules",
            module(platform, backend)
        )
    })
}
