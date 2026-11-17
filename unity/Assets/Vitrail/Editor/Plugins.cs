using UnityEditor;
using UnityEngine;

namespace VitrailLab.Editor
{
    /// <summary>
    /// The native library per platform: libvitrail.dylib for macOS (and the Mac editor), vitrail.dll for 64-bit Windows (and
    /// the Windows editor). Set once on load; also builds the lab for Windows from the Vitrail menu.
    /// </summary>
    [InitializeOnLoad]
    internal static class Plugins
    {
        private const string Mac = "Assets/Vitrail/Plugins/macOS/libvitrail.dylib";
        private const string Win = "Assets/Vitrail/Plugins/x86_64/vitrail.dll";

        static Plugins() => EditorApplication.delayCall += Configure;

        [MenuItem("Vitrail/Configure native plugins")]
        public static void Configure()
        {
            Set(Mac, BuildTarget.StandaloneOSX, "OSX", "AnyCPU");
            Set(Win, BuildTarget.StandaloneWindows64, "Windows", "x86_64");
        }

        private static void Set(string path, BuildTarget target, string editorOs, string cpu)
        {
            if (AssetImporter.GetAtPath(path) is not PluginImporter p) return;
            if (!p.GetCompatibleWithAnyPlatform() && p.GetCompatibleWithPlatform(target) && p.GetEditorData("OS") == editorOs) return;
            p.SetCompatibleWithAnyPlatform(false);
            foreach (BuildTarget t in System.Enum.GetValues(typeof(BuildTarget)))
                if (t > 0 && t != target) try { p.SetCompatibleWithPlatform(t, false); } catch { }
            p.SetCompatibleWithPlatform(target, true);
            p.SetPlatformData(target, "CPU", cpu);
            p.SetCompatibleWithEditor(true);
            p.SetEditorData("OS", editorOs);
            p.SetEditorData("CPU", cpu);
            p.SaveAndReimport();
        }

        [MenuItem("Vitrail/Build Windows lab")]
        public static void BuildWindows()
        {
            Configure();
            var report = BuildPipeline.BuildPlayer(new BuildPlayerOptions
            {
                scenes = new[] { "Assets/Vitrail/Lab.unity" },
                locationPathName = "Build/Windows/VitrailLab.exe",
                target = BuildTarget.StandaloneWindows64,
                options = BuildOptions.None,
            });
            Debug.Log($"Vitrail Windows build: {report.summary.result}, {report.summary.totalSize / 1e6:F0} MB");
        }
    }
}
