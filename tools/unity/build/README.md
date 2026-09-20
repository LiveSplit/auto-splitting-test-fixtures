# unity-fixtures

Builds the fixture assets for one Unity version: one whole player per variant, zipped and hashed. It prints the manifest entries to paste into `manifest.json`.

> [!IMPORTANT]  
> Mac IL2CPP is missing.  
> It needs a host with the module installed; every other variant is verified.

## Setup

1. Install the editor version through Unity Hub.
2. Add the build support module each variant needs. A Windows host builds its own Mono variants with the editor alone and needs a module for everything else: `Windows Build Support (IL2CPP)`, `Linux Build Support (Mono)`, `Linux Build Support (IL2CPP)`, `Mac Build Support (Mono)`, and `Mac Build Support (IL2CPP)`.
3. Have a Rust toolchain; the tool builds with stable cargo.
4. Keep about 4 GB free per version. A run of every variant on 6000.5.10f1 leaves 3.2 GB in the workspace: 1.9 GB of project, 1.1 GB of players, and 223 MB of assets, the only part worth keeping once a release is up.

The x86 variants need an editor that still ships a 32-bit player; narrow `-v` on versions that dropped it.

## Usage

From `tools/unity/build`:

```
cargo run --release -- -e "/path/to/Unity/Hub/Editor/6000.5.10f1/Editor/Unity.exe" -o /path/to/destination
```

The editor binary is `Editor/Unity.exe` under a Hub install on Windows, `Editor/Unity` on Linux, and `Unity.app/Contents/MacOS/Unity` on macOS.

| | | | |
|---|---|---|---|
| `-e` | `--editor` | **required** | Path to the editor binary |
| | `--editor-version` | optional | Editor version, when the editor path does not name it |
| `-o` | `--out` | **required** | Workspace to build into |
| `-v` | `--variant` | optional | Build only these variants. Every variant builds without it |

Variants are `win-x64-mono`, `win-x64-il2cpp`, `win-x86-mono`, `win-x86-il2cpp`, `linux-x64-mono`, `linux-x64-il2cpp`, `mac-mono`, and `mac-il2cpp`, passed together or one flag at a time:

```
cargo run --release -- -e "/path/to/Unity.exe" -o /path/to/destination -v win-x64-mono linux-x64-mono
```

The workspace holds one directory per version. Inside it, `project/` is a throwaway project reused across runs, each variant builds into its own directory, and the assets are written next to them:

```
<out>/6000.5.10f1/
├── project/
├── win-x64-mono/
├── unity-6000.5.10f1-win-x64-mono.zip
└── unity-6000.5.10f1-win-x64-mono.log
```

Every editor run keeps its log beside what it produced. The log opens with the command line it ran, so it records what produced the asset. The run prints how long each build took and ends with the manifest entries.

The editor does the building: `editor/FixtureBuild.cs` is copied into the throwaway project and invoked through `-executeMethod`, taking the output path and scripting backend as arguments.

The tool sets up a fresh project once by copying `assets/` into it: the two scenes, the scripts and their `.meta` files, so every asset keeps the GUID it was written with. Then `FixtureBuild.Prepare` turns on text serialization and turns off audio. The scenes were written by 5.6.7f1, and every later editor upgrades them when it imports them. If they ever need to change, `FixtureBuild.CreateScenes` writes them again from the oldest editor.

## The fixture

[`unity-fixture.json`](../../../unity-fixture.json) at the root of the repo lists what the player holds: two scenes, the objects in them with their names, parents and active flags, and the values that the `FixtureData` script sets once in `Awake`. Nothing moves or ticks, so a run reads the same at any point.

## What goes in an asset

The whole build, at the paths a shipped game uses: the executable, its data directory and the runtime. A build also leaves a `BackUpThisFolder_ButDontShipItWithYourGame` directory next to the player. It holds a second copy of the metadata and other things a game does not ship, and it stays out of the asset.

The tool fails instead of shipping an incomplete asset when a build lacks a binary its backend needs. Every build needs `UnityPlayer`, or the executable itself on players from before the engine was split out. Mono builds need `mono-2.0-bdwgc`, or `mono` on older editors. IL2CPP builds need `GameAssembly` and `global-metadata.dat`.

The symbols come out of that backup directory: `GameAssembly.pdb` on Windows, `GameAssembly.debug` and `UnityPlayer_s.debug` on Linux. IL2CPP compiles the runtime's own structures into the game assembly, so those symbols say where the members of that build's structures sit. They roughly double an IL2CPP asset, but that is better than keeping them somewhere a build can drift away from.

Mono builds have no symbols. The editor keeps a `mono-2.0-bdwgc.pdb` at its root and the `UnityPlayer` symbols under its player variations. Neither is attached to the copy a player ships, so you pair them by matching debug IDs, not paths.

## Cutting a release

Releases are cut by hand, since building needs installed editors:

1. Build the variants for the version.
2. Create the release tagged `unity-<version>` and upload the zips.
3. PR the printed manifest entries into `manifest.json`.
