# auto-splitting-test-fixtures

Real engine binaries for auto splitting runtimes to test against, one GitHub release per engine version.

Binaries this size don't belong in git history, so each engine version is a release whose assets hold just the files the tests read.

## Layout

One release per engine version, tagged `<engine>-<version>`. Each asset is one variant of that version: a platform, plus whatever else that engine varies by.

```
unity-6000.5.8f1-win-x64-mono.zip
unity-6000.5.8f1-win-x64-il2cpp.zip
unity-6000.5.8f1-win-x86-mono.zip
unity-6000.5.8f1-win-x86-il2cpp.zip
```

The program inside is the emptiest one the engine builds, made the way a shipped one is. An asset holds the engine's own runtime and metadata files, and their symbols where the build produces any. Each engine's build tool decides which files those are.

We keep everything that varies by machine out of an asset, so building the same engine version and variant somewhere else gives the same archive byte for byte. The build log is the one file a build stamps itself onto, so we leave it beside the asset instead of inside it.

[`manifest.json`](manifest.json) maps (engine, version, variant) to the asset URL, its size, its sha256, and the archive's files, each with a sha256 of its own. The archive hash checks the download and the build together. The per-file hashes check a single extracted file, and compare one binary across two versions without fetching both. Consumers fetch through it and cache locally.

Engines separate by data, not structure: the tag prefix, the asset names, and the manifest's `engine` field. Tooling lives per engine under `tools/`.

## Building

- [`Unity`](tools/unity/build)

## Licensing

- Source: CC0
- Release assets carry whatever terms the engine ships under
