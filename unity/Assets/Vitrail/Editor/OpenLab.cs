using UnityEditor;
using UnityEditor.SceneManagement;

namespace VitrailLab.Editor
{
    /// <summary>Opens the lab scene when the editor starts on an empty one.</summary>
    [InitializeOnLoad]
    internal static class OpenLab
    {
        static OpenLab()
        {
            EditorApplication.delayCall += () =>
            {
                if (EditorApplication.isPlayingOrWillChangePlaymode) return;
                if (string.IsNullOrEmpty(EditorSceneManager.GetActiveScene().path))
                    EditorSceneManager.OpenScene("Assets/Vitrail/Lab.unity");
            };
        }
    }
}
