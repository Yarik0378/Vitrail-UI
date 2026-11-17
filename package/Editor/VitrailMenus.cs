using System.IO;
using System.Linq;
using UnityEditor;
using UnityEngine;

namespace Vitrail.Editor
{
    internal static class VitrailMenus
    {
        [MenuItem("GameObject/UI/Vitrail UI Screen", false, 2100)]
        private static void Screen(MenuCommand cmd)
        {
            var go = new GameObject("Vitrail UI", typeof(VitrailScreen));
            GameObjectUtility.SetParentAndAlign(go, cmd.context as GameObject);
            go.GetComponent<VitrailScreen>().page = FirstPage() ?? "ui/index.html";
            Undo.RegisterCreatedObjectUndo(go, "Create Vitrail UI Screen");
            Selection.activeObject = go;
        }

        [MenuItem("GameObject/UI/Vitrail UI World Panel", false, 2101)]
        private static void World(MenuCommand cmd)
        {
            var go = GameObject.CreatePrimitive(PrimitiveType.Quad);
            go.name = "Vitrail UI World Panel";
            Object.DestroyImmediate(go.GetComponent<MeshCollider>());
            go.AddComponent<MeshCollider>();
            go.transform.localScale = new Vector3(1.6f, 1.2f, 1);
            go.AddComponent<VitrailWorld>().page = FirstPage() ?? "ui/index.html";
            GameObjectUtility.SetParentAndAlign(go, cmd.context as GameObject);
            Undo.RegisterCreatedObjectUndo(go, "Create Vitrail UI World Panel");
            Selection.activeObject = go;
        }

        /// <summary>The HTML pages under StreamingAssets, as paths relative to it.</summary>
        internal static string[] Pages()
        {
            var root = Application.streamingAssetsPath;
            if (!Directory.Exists(root)) return new string[0];
            return Directory.GetFiles(root, "*.html", SearchOption.AllDirectories)
                .Select(f => Path.GetRelativePath(root, f).Replace('\\', '/')).OrderBy(f => f).ToArray();
        }

        private static string FirstPage() => Pages().FirstOrDefault();
    }

    [CustomEditor(typeof(VitrailScreen))]
    internal sealed class VitrailScreenEditor : UnityEditor.Editor
    {
        public override void OnInspectorGUI()
        {
            var screen = (VitrailScreen)target;
            serializedObject.Update();
            PagePicker(serializedObject.FindProperty("page"));
            DrawPropertiesExcluding(serializedObject, "m_Script", "page");
            serializedObject.ApplyModifiedProperties();
            Live(screen.Panel, screen.Reload);
        }

        internal static void PagePicker(SerializedProperty page)
        {
            var pages = VitrailMenus.Pages();
            using (new EditorGUILayout.HorizontalScope())
            {
                EditorGUILayout.PropertyField(page);
                if (pages.Length > 0)
                {
                    var i = EditorGUILayout.Popup(System.Array.IndexOf(pages, page.stringValue), pages, GUILayout.Width(140));
                    if (i >= 0) page.stringValue = pages[i];
                }
            }
            if (!File.Exists(Path.Combine(Application.streamingAssetsPath, page.stringValue)))
                EditorGUILayout.HelpBox($"No page at StreamingAssets/{page.stringValue}.", MessageType.Warning);
        }

        /// <summary>In play mode: what the page costs, and a reload.</summary>
        internal static void Live(WebPanel panel, System.Action reload)
        {
            if (!Application.isPlaying || panel == null) return;
            EditorGUILayout.Space();
            EditorGUILayout.HelpBox(panel.Stats, MessageType.None);
            using (new EditorGUILayout.HorizontalScope())
            {
                if (GUILayout.Button("Reload page")) reload();
                if (GUILayout.Button("Inspect")) VitrailInspector.Open(panel);
            }
        }

        public override bool RequiresConstantRepaint() => Application.isPlaying;
    }

    [CustomEditor(typeof(VitrailWorld))]
    internal sealed class VitrailWorldEditor : UnityEditor.Editor
    {
        public override void OnInspectorGUI()
        {
            var world = (VitrailWorld)target;
            serializedObject.Update();
            VitrailScreenEditor.PagePicker(serializedObject.FindProperty("page"));
            DrawPropertiesExcluding(serializedObject, "m_Script", "page");
            serializedObject.ApplyModifiedProperties();
            VitrailScreenEditor.Live(world.Panel, () => { world.enabled = false; world.enabled = true; });
        }

        public override bool RequiresConstantRepaint() => Application.isPlaying;
    }
}
