# unity-fixtures

Builds the fixture assets for one Unity version: one whole player per variant, zipped and hashed, plus the manifest entries to paste into `manifest.json`.

> [!IMPORTANT]
> Mac IL2CPP is missing. It needs a Mac host with the module installed. Every other variant is verified.

## Quick start

From `tools/unity/build`:

```
cargo run --release -- -e "C:/Unity/6000.5.10f1/Editor/Unity.exe" -o D:/fixtures
```

That builds every variant the editor has modules for and writes the assets to `D:/fixtures/6000.5.10f1/`. It prints which variants it skipped and which module each one needs. The last thing it prints is the manifest entries.

To build only some variants, list the platforms and backends:

```
cargo run --release -- -e "C:/Unity/6000.5.10f1/Editor/Unity.exe" -o D:/fixtures -p win-x64 linux-x64 -b mono-bdwgc il2cpp-release
```

That builds 4 players: every platform given with every backend given.

## What you need

| To build | Install |
|---|---|
| Anything | The editor version through Unity Hub, and a stable Rust toolchain |
| Windows Mono | Nothing else, a Windows host has it |
| Windows IL2CPP | The "Windows Build Support (IL2CPP)" module, and "MSVC v142 - VS 2019 C++ x64/x86 build tools (Latest)" in any Visual Studio install |
| Linux | The "Linux Build Support (Mono)" or "(IL2CPP)" module |
| Mac Mono | The "Mac Build Support (Mono)" module |
| Mac IL2CPP | A Mac host |

Keep about 4 GB free per version. A run of every variant on 6000.5.10f1 leaves 3.2 GB: 1.9 GB of project, 1.1 GB of players, and 223 MB of assets, the only part worth keeping once a release is up.

## Options

| Short | Long | | What it does |
|---|---|---|---|
| `-e` | `--editor` | **required** | Path to the editor binary: `Editor/Unity.exe` under a Hub install on Windows, `Editor/Unity` on Linux, `Unity.app/Contents/MacOS/Unity` on macOS |
| `-o` | `--out` | **required** | Folder to build into. Each version gets its own folder inside |
| `-p` | `--platform` | optional | `win-x64`, `win-x86`, `linux-x64`, `mac`. Without it, every platform the editor has the module for |
| `-b` | `--backend` | optional | `mono-legacy`, `mono-bdwgc`, `il2cpp-release`, `il2cpp-master`. Without it, every backend the editor offers |
| | `--editor-version` | optional | The editor version, for when the editor path doesn't show it |

## Variants

A variant is a platform and a backend, like `win-x64-mono-bdwgc`. The backend is Unity's scripting backend plus what varies inside it:

| Backend | What it is | Editors |
|---|---|---|
| `mono-legacy` | The old Mono runtime, `mono.dll` | before 2019.1 |
| `mono-bdwgc` | The newer Mono runtime built with the Boehm collector, `mono-2.0-bdwgc.dll` | 2017.1 on |
| `il2cpp-release` | IL2CPP with the Release C++ configuration | every editor with the module |
| `il2cpp-master` | IL2CPP with the Master C++ configuration, which compiles the runtime differently, so a signature matched in one isn't matched in the other | 2018.3 on |

The x86 variants need an editor that still ships a 32-bit player. The tool refuses a backend the editor doesn't offer, and a variant given with `-p` or `-b` whose module isn't installed.

## MSVC toolsets

Toolsets from 14.30 on build x86 IL2CPP players that crash before the game loads, and on 2022.2 they don't compile at all. 14.29 is the newest toolset known to work.

- Editors from 2021.2 on: the tool picks the newest toolset below 14.30 by itself, so newer toolsets can stay installed.
- Editors before 2021.2: the editor takes the toolset listed in the Visual Studio install's `VC\Auxiliary\Build\Microsoft.VCToolsVersion.default.txt`, so that file has to hold the 14.29 toolset's version.

The tool also reads the linker version out of every x86 `GameAssembly.dll` and fails the variant on 14.30 or newer.

## The workspace

Each version gets one folder. `project/` is a throwaway project reused across runs, each variant builds into its own folder, and the assets and logs sit next to them:

```
<out>/6000.5.10f1/
├── project/
├── win-x64-mono-bdwgc/
├── unity-6000.5.10f1-win-x64-mono-bdwgc.zip
└── unity-6000.5.10f1-win-x64-mono-bdwgc.log
```

Every editor run keeps its log beside what it produced. The log opens with the command line the editor ran, so it shows what produced the asset.

## How it builds

The editor does the building. The tool copies `editor/FixtureBuild.cs` into the project and runs it through `-executeMethod`.

A fresh project gets `assets/` copied in once: the two scenes, the scripts and their `.meta` files, so every asset keeps the GUID it was written with. `FixtureBuild.Prepare` then turns on text serialization and turns off audio. The scenes were written by 5.0.0f4, and every later editor upgrades them when it imports them. If they ever need to change, `FixtureBuild.CreateScenes` writes them again from the oldest editor.

Switching the Mono runtime only takes effect in a fresh editor session, so on editors from 2017.1 through 2018.4 the tool runs the editor once to switch it and once more to build.

## The fixture

[`unity-fixture.json`](../../../unity-fixture.json) at the root of the repo lists what the player holds: two scenes, the objects in them with their names, parents and active flags, and the values that the `FixtureData` script sets once in `Awake`. The roots of a scene are listed in the order the scene keeps them. Nothing moves or ticks, so a run reads the same at any point.

## What goes in an asset

The whole build, at the paths a shipped game uses: the executable, its data directory and the runtime. A build also leaves a `BackUpThisFolder_ButDontShipItWithYourGame` directory next to the player. It holds a second copy of the metadata and other things a game does not ship, and it stays out of the asset.

The tool fails instead of shipping an incomplete asset when a build lacks a binary its backend needs. Every build needs `UnityPlayer`, or the executable itself on players from before the engine was split out. Mono builds need `mono-2.0-bdwgc`, or `mono` on older editors. IL2CPP builds need `GameAssembly` and `global-metadata.dat`.

The symbols come out of that backup directory: `GameAssembly.pdb` on Windows, `GameAssembly.debug` and `UnityPlayer_s.debug` on Linux. IL2CPP compiles the runtime's own structures into the game assembly, so those symbols hold where the members of that build's structures sit. They roughly double an IL2CPP asset, but that is better than keeping them somewhere a build can drift away from.

Mono builds have no symbols. The editor keeps a `mono-2.0-bdwgc.pdb` at its root and the `UnityPlayer` symbols under its player variations. Neither is attached to the copy a player ships, so you pair them by matching debug IDs, not paths.

## Cutting a release

Releases are cut by hand, since building needs installed editors:

1. Build the variants for the version.
2. Create the release tagged `unity-<version>` and upload the zips.
3. PR the printed manifest entries into `manifest.json`.
