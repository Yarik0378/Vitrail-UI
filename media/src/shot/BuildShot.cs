using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

public static class BuildShot
{
    public static void Build()
    {
        PlayerSettings.colorSpace = ColorSpace.Linear;
        var ps = AssetDatabase.LoadAllAssetsAtPath("ProjectSettings/ProjectSettings.asset")[0];
        var so = new SerializedObject(ps);
        so.FindProperty("activeInputHandler").intValue = 2;
        so.ApplyModifiedProperties();
        PlayerSettings.fullScreenMode = FullScreenMode.Windowed;
        PlayerSettings.defaultScreenWidth = 640;
        PlayerSettings.defaultScreenHeight = 360;
        PlayerSettings.resizableWindow = true;
        PlayerSettings.runInBackground = true;

        System.IO.Directory.CreateDirectory("Assets/Settings");
        var rd = ScriptableObject.CreateInstance<UniversalRendererData>();
        rd.intermediateTextureMode = IntermediateTextureMode.Always;
        AssetDatabase.CreateAsset(rd, "Assets/Settings/Renderer.asset");
        var urp = UniversalRenderPipelineAsset.Create(rd);
        urp.renderScale = 1f;
        urp.msaaSampleCount = 1;
        AssetDatabase.CreateAsset(urp, "Assets/Settings/URP.asset");
        GraphicsSettings.defaultRenderPipeline = urp;
        for (var i = 0; i < QualitySettings.names.Length; i++) { QualitySettings.SetQualityLevel(i, false); QualitySettings.renderPipeline = urp; }

        var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
        var cam = new GameObject("Camera").AddComponent<Camera>();
        cam.orthographic = true;
        cam.orthographicSize = 5;
        cam.transform.position = new Vector3(0, 0, -10);
        cam.clearFlags = CameraClearFlags.SolidColor;
        cam.backgroundColor = Color.black;
        cam.tag = "MainCamera";
        var data = cam.GetUniversalAdditionalCameraData();
        data.renderPostProcessing = false;
        data.antialiasing = AntialiasingMode.None;
        var quad = GameObject.CreatePrimitive(PrimitiveType.Quad);
        quad.name = "Background";
        var mat = new Material(Shader.Find("Universal Render Pipeline/Unlit"));
        AssetDatabase.CreateAsset(mat, "Assets/Settings/Background.mat");
        quad.GetComponent<MeshRenderer>().sharedMaterial = mat;
        var shot = new GameObject("Shot").AddComponent<Shot>();
        shot.cam = cam;
        shot.quad = quad.GetComponent<MeshRenderer>();
        EditorSceneManager.SaveScene(scene, "Assets/Shot/Shot.unity");
        AssetDatabase.SaveAssets();

        var r = BuildPipeline.BuildPlayer(new[] { "Assets/Shot/Shot.unity" }, "Build/shot.exe", BuildTarget.StandaloneWindows64, BuildOptions.None);
        Debug.Log("BuildShot: " + r.summary.result + " " + r.summary.totalErrors + " errors");
        if (r.summary.result != UnityEditor.Build.Reporting.BuildResult.Succeeded) EditorApplication.Exit(1);
    }
}
