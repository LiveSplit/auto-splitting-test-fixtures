# auto-splitting-test-fixtures

Real engine binaries for auto splitting runtimes to test against, one GitHub release per engine version.

Binaries this size don't belong in git history, so each engine version is a release whose assets hold the players.

## Layout

One release per engine version, tagged `<engine>-<version>`. Each asset is one variant of that version: a platform, plus whatever else that engine varies by.

```
unity-6000.5.10f1-win-x64-mono-bdwgc.zip
unity-6000.5.10f1-win-x64-il2cpp-release.zip
unity-6000.5.10f1-win-x64-il2cpp-master.zip
unity-6000.5.10f1-win-x86-mono-bdwgc.zip
```

The program inside is a small one built by the engine, the same way a shipped game is built. An asset holds the whole build, so you can run it: the executable, its data and the runtime, at the paths a shipped game uses, plus the symbols when the build produces any. It runs headless with `-batchmode -nographics`, and audio is turned off in its settings. Nothing in it moves, so a run reads the same at any point. [`unity-fixture.json`](unity-fixture.json) lists what a Unity player holds, object by object and value by value.

An asset is what one build produced. If you build the same version again, you get the same files except the ones the engine stamps on every build, like the build GUID and the compiled code. The build log stays next to the asset instead of inside it.

[`manifest.json`](manifest.json) maps (engine, version, variant) to the asset URL, its size and its sha256. The hash checks that the download is intact. Consumers fetch through the manifest and cache locally, so a harness never has to ask GitHub what exists.

Engines separate by data, not structure: the tag prefix, the asset names, and the manifest's `engine` field. Tooling lives per engine under `tools/`.

## Building

- [`Unity`](tools/unity/build)

## Licensing

- Source: CC0
- Release assets carry whatever terms the engine ships under
