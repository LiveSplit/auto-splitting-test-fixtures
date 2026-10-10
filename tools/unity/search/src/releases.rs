//! Asks Unity's release API which versions exist and where their packages
//! are.

use std::io;

use serde_json::Value;

use crate::version;

const RELEASES: &str = "https://services.api.unity.com/unity/editor/release/v1/releases";

/// Fetches one page of the API.
fn page(query: &str) -> io::Result<Value> {
    let text = ureq::get(format!("{RELEASES}?{query}"))
        .header("User-Agent", "unity-search")
        .call()
        .map_err(io::Error::other)?
        .into_body()
        .read_to_string()
        .map_err(io::Error::other)?;
    serde_json::from_str(&text).map_err(io::Error::other)
}

/// Lists every full release of one minor such as `2019.4`, oldest first.
pub(crate) fn of_minor(minor: &str) -> io::Result<Vec<String>> {
    let mut found = Vec::new();
    let mut offset = 0;
    loop {
        let answer = page(&format!("version={minor}.&limit=25&offset={offset}"))?;
        let results = answer["results"].as_array().cloned().unwrap_or_default();
        for release in &results {
            let Some(name) = release["version"].as_str() else {
                continue;
            };
            if name.starts_with(&format!("{minor}.")) && version::is_release(name) {
                found.push(name.to_string());
            }
        }
        offset += 25;
        if offset >= answer["total"].as_u64().unwrap_or(0) {
            break;
        }
    }
    found.sort_by(|a, b| version::order(a, b));
    found.dedup();
    Ok(found)
}

/// Fetches the record of one exact version. Returns `None` when the API
/// has no such version.
pub(crate) fn record(version: &str) -> io::Result<Option<Value>> {
    let answer = page(&format!("version={version}&limit=5"))?;
    Ok(answer["results"]
        .as_array()
        .and_then(|results| {
            results
                .iter()
                .find(|release| release["version"].as_str() == Some(version))
        })
        .cloned())
}

/// Picks which package of a release to stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Package {
    /// The Mac editor itself, which holds the IL2CPP headers
    Editor,
    /// The Mac package of Windows Build Support, which holds the Windows
    /// players and their PDBs
    Windows,
    /// The Mac package of Linux Build Support, which holds the Linux players
    Linux,
}

/// Reads the URL of one package out of a release's record.
pub(crate) fn package_url(release: &Value, package: Package) -> Option<String> {
    let downloads = release["downloads"].as_array()?;
    let mac = downloads.iter().find(|d| {
        d["platform"].as_str() == Some("MAC_OS")
            && d["url"].as_str().is_some_and(|url| url.ends_with(".pkg"))
    })?;
    if package == Package::Editor {
        return mac["url"].as_str().map(String::from);
    }
    // Unity calls the Windows module "Windows Build Support (Mono)" from
    // 2018 on and "Windows Build Support" before that. Linux is "Linux Build
    // Support (Mono)" from 2019.3 on and "Linux Build Support" before.
    let names: &[&str] = match package {
        Package::Windows => &["Windows Build Support (Mono)", "Windows Build Support"],
        Package::Linux => &["Linux Build Support (Mono)", "Linux Build Support"],
        Package::Editor => unreachable!(),
    };
    let modules = mac["modules"].as_array()?;
    names.iter().find_map(|name| {
        modules.iter().find_map(|module| {
            let url = module["url"].as_str()?;
            (module["name"].as_str() == Some(name) && url.ends_with(".pkg"))
                .then(|| url.to_string())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release() -> Value {
        json!({
            "version": "2018.2.0f1",
            "downloads": [
                {"platform": "WINDOWS", "url": "https://x/UnitySetup64-2018.2.0f1.exe", "modules": []},
                {"platform": "MAC_OS", "url": "https://x/Unity-2018.2.0f1.pkg", "modules": [
                    {"name": "Windows Build Support (Mono)", "url": "https://x/UnitySetup-Windows-Mono-Support-for-Editor-2018.2.0f1.pkg"},
                    {"name": "Linux Build Support", "url": "https://x/UnitySetup-Linux-Support-for-Editor-2018.2.0f1.pkg"},
                    {"name": "Android Build Support", "url": "https://x/android.pkg"}
                ]}
            ]
        })
    }

    #[test]
    fn editor_package_is_mac_pkg() {
        assert_eq!(
            package_url(&release(), Package::Editor).as_deref(),
            Some("https://x/Unity-2018.2.0f1.pkg")
        );
    }

    #[test]
    fn modules_are_found_by_either_of_their_names() {
        assert_eq!(
            package_url(&release(), Package::Windows).as_deref(),
            Some("https://x/UnitySetup-Windows-Mono-Support-for-Editor-2018.2.0f1.pkg")
        );
        assert_eq!(
            package_url(&release(), Package::Linux).as_deref(),
            Some("https://x/UnitySetup-Linux-Support-for-Editor-2018.2.0f1.pkg")
        );
    }

    #[test]
    fn no_packages_without_mac_downloads() {
        let release = json!({"version": "x", "downloads": []});
        assert_eq!(package_url(&release, Package::Editor), None);
        assert_eq!(package_url(&release, Package::Linux), None);
    }
}
