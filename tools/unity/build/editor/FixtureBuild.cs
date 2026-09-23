// The oldest editors compile this with their C# 4.0 compiler and report a
// build as an error string rather than a report, so the file stays inside
// what every supported editor accepts.
using System;
using System.IO;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
#if UNITY_2018_1_OR_NEWER
using UnityEditor.Build.Reporting;
#endif

public static class FixtureBuild
{
    /// <summary>
    ///     Builds one fixture player, pointed here through <c>-executeMethod</c>.
    ///     The <c>unity-fixtures</c> tool copies this file into the throwaway project, then starts the editor in batch mode.
    /// </summary>
    /// <remarks>
    ///     <c>-fixtureOut</c> says where the player goes and <c>-fixtureVariant</c> what to build, by the
    ///     variant's name; Unity's own <c>-buildTarget</c> selects the platform.
    ///     The editor writes that command line into its log to document how a player was built.
    /// </remarks>
    public static void Build()
    {
        string outDir = Word("-fixtureOut");
        var variant = Variant.FromCommandLine();

        var target = EditorUserBuildSettings.activeBuildTarget;
        var group = BuildPipeline.GetBuildTargetGroup(target);
        PlayerSettings.SetScriptingBackend(group, variant.Backend);

#if UNITY_2018_1_OR_NEWER
        // The configuration reaches the C++ compile of the runtime itself, so
        // a master player is not the binary a release signature matched.
        if (variant.Backend == ScriptingImplementation.IL2CPP)
        {
            PlayerSettings.SetIl2CppCompilerConfiguration(group, variant.Configuration);
        }
#endif

        var options = new BuildPlayerOptions
        {
            scenes = Scenes(),
            locationPathName = Path.Combine(outDir, PlayerName(target)),
            target = target,
        };

#if UNITY_2018_1_OR_NEWER
        var report = BuildPipeline.BuildPlayer(options);
        EditorApplication.Exit(report.summary.result == BuildResult.Succeeded ? 0 : 1);
#else
        string error = BuildPipeline.BuildPlayer(options);
        EditorApplication.Exit(string.IsNullOrEmpty(error) ? 0 : 1);
#endif
    }

    const string BootScene = "Assets/Scenes/Boot.unity";

    // This path is too long to fit inside the scene's own record. That way a
    // reader sees a short path and a long one.
    const string SecondScene = "Assets/Scenes/Deeply/Nested/Second.unity";

    static string[] Scenes()
    {
        if (!File.Exists(BootScene) || !File.Exists(SecondScene))
        {
            throw new FileNotFoundException("the fixture scenes are not in the project");
        }

        return new[] { BootScene, SecondScene };
    }

#if UNITY_2017_1_OR_NEWER && !UNITY_2019_1_OR_NEWER
    /// <summary>
    ///     Sets the scripting runtime the variant in <c>-fixtureVariant</c> wants: legacy ships
    ///     <c>mono.dll</c>, bdwgc ships <c>mono-2.0-bdwgc.dll</c>. The switch only takes effect in a
    ///     fresh session, so the tool runs this on its own and builds in the next run.
    /// </summary>
    public static void SetRuntime()
    {
        PlayerSettings.scriptingRuntimeVersion = Variant.FromCommandLine().Runtime;
        AssetDatabase.SaveAssets();
        EditorApplication.Exit(0);
    }
#endif

    /// <summary>
    ///     Sets the project up once before any variant builds. It turns on
    ///     text serialization so the tool can patch the settings, and turns
    ///     audio off because there isn't a player flag for it.
    /// </summary>
    public static void Prepare()
    {
        EditorSettings.serializationMode = SerializationMode.ForceText;
        SilenceAudio();
        AssetDatabase.SaveAssets();
        EditorApplication.Exit(0);
    }

    /// <summary>
    ///     Writes the two fixture scenes the tool ships. Run this once from
    ///     the oldest editor: a newer editor upgrades an old scene when it
    ///     imports it, but an older editor can't read a new one. The scenes
    ///     hold what unity-fixture.json describes. Nothing in them moves, so
    ///     every run reads the same.
    /// </summary>
    public static void CreateScenes()
    {
        EditorSettings.serializationMode = SerializationMode.ForceText;
        Directory.CreateDirectory(Path.GetDirectoryName(SecondScene));

        var second = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
        new GameObject("SecondRoot");
        EditorSceneManager.SaveScene(second, SecondScene);

        var boot = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
        new GameObject("Fixture").AddComponent<FixtureData>();

        var foo = new GameObject("Foo");
        var bar = new GameObject("Bar");
        var baz = new GameObject("Baz");
        var qux = new GameObject("Qux");
        var hidden = new GameObject("Hidden");
        var quux = new GameObject("Quux");
        bar.transform.SetParent(foo.transform);
        baz.transform.SetParent(bar.transform);
        qux.transform.SetParent(baz.transform);
        hidden.transform.SetParent(baz.transform);
        quux.transform.SetParent(hidden.transform);

        // A second root after Foo, so a reader can tell the order the roots
        // come back in. Its name carries on the series the tree uses.
        new GameObject("Corge");

        qux.AddComponent<Marker>();

        // Quux stays active under Hidden, which is inactive. That way there is
        // an object whose own flag differs from its effective one.
        hidden.SetActive(false);

        // Every component can be stored exactly in a float. That way a reader
        // can check for equality instead of a tolerance.
        qux.transform.localPosition = new Vector3(1.25f, -2.5f, 3.75f);
        qux.transform.localRotation = new Quaternion(0.5f, 0.5f, 0.5f, 0.5f);
        qux.transform.localScale = new Vector3(2f, 4f, 8f);

        EditorSceneManager.SaveScene(boot, BootScene);
        AssetDatabase.SaveAssets();
        EditorApplication.Exit(0);
    }

    // Turns audio off in the project settings, because there isn't a player
    // flag for it. It goes through the serialized object because 5.6 writes
    // the asset as binary.
    static void SilenceAudio()
    {
        var loaded = AssetDatabase.LoadAllAssetsAtPath("ProjectSettings/AudioManager.asset");
        if (loaded == null || loaded.Length == 0 || loaded[0] == null)
        {
            return;
        }

        var audio = new SerializedObject(loaded[0]);
        var disabled = audio.FindProperty("m_DisableAudio");
        if (disabled == null)
        {
            return;
        }

        disabled.boolValue = true;
        audio.ApplyModifiedProperties();
        AssetDatabase.SaveAssets();
    }

    static string PlayerName(BuildTarget target)
    {
        switch (target)
        {
#if UNITY_2017_3_OR_NEWER
            case BuildTarget.StandaloneOSX:
#else
            case BuildTarget.StandaloneOSXUniversal:
#endif
                return "fixture.app";
            case BuildTarget.StandaloneLinux64:
                return "fixture";
            default:
                return "fixture.exe";
        }
    }

    /// <summary>
    ///     What the last word of a variant name says, such as <c>win-x64-mono-bdwgc</c> or
    ///     <c>linux-x64-il2cpp-master</c>: the backend, and its Mono runtime or IL2CPP configuration.
    ///     Unity's own <c>-buildTarget</c> already selected the platform.
    /// </summary>
    sealed class Variant
    {
        public ScriptingImplementation Backend;
#if UNITY_2018_1_OR_NEWER
        public Il2CppCompilerConfiguration Configuration;
#endif
#if UNITY_2017_1_OR_NEWER && !UNITY_2019_1_OR_NEWER
        public ScriptingRuntimeVersion Runtime;
#endif

        public static Variant FromCommandLine()
        {
            string name = Word("-fixtureVariant");
            string flavor = name.Substring(name.LastIndexOf('-') + 1);
            var variant = new Variant();
            switch (flavor)
            {
                case "legacy":
                case "bdwgc":
                    variant.Backend = ScriptingImplementation.Mono2x;
#if UNITY_2017_1_OR_NEWER && !UNITY_2019_1_OR_NEWER
                    variant.Runtime = flavor == "legacy" ? ScriptingRuntimeVersion.Legacy : ScriptingRuntimeVersion.Latest;
#endif
                    return variant;
                case "release":
                case "master":
                    variant.Backend = ScriptingImplementation.IL2CPP;
#if UNITY_2018_1_OR_NEWER
                    variant.Configuration = flavor == "master" ? Il2CppCompilerConfiguration.Master : Il2CppCompilerConfiguration.Release;
#endif
                    return variant;
                default:
                    throw new ArgumentException("unknown variant " + name);
            }
        }
    }

    // The word after a flag on the editor's command line.
    static string Word(string flag)
    {
        string[] args = Environment.GetCommandLineArgs();
        int at = Array.IndexOf(args, flag);
        if (at < 0 || at + 1 >= args.Length)
        {
            throw new ArgumentException("missing " + flag);
        }

        return args[at + 1];
    }
}
