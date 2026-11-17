using System.IO;
using UnityEditor;
using UnityEditor.SceneManagement;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

namespace VitrailLab.Editor
{
    /// <summary>
    /// Batch-mode tools for the lab: Setup makes the project match the game (URP, linear colour) and builds a test scene
    /// (an orthographic camera over a picture of the map); Capture renders a page over it and saves the scene alone and
    /// the scene with the page, so Chrome can render the same page over the very same background for comparison.
    /// Args after -executeMethod: -page <html> -out <dir> -w <px> -h <px>
    /// </summary>
    public static class Lab
    {
        public static void Setup()
        {
            PlayerSettings.colorSpace = ColorSpace.Linear;
            Directory.CreateDirectory("Assets/Vitrail/Settings");
            var rendererData = ScriptableObject.CreateInstance<UniversalRendererData>();
            // The page is mixed over a copy of the camera picture, so the camera must draw into its own texture first.
            rendererData.intermediateTextureMode = IntermediateTextureMode.Always;
            AssetDatabase.CreateAsset(rendererData, "Assets/Vitrail/Settings/Renderer.asset");
            var urp = UniversalRenderPipelineAsset.Create(rendererData);
            AssetDatabase.CreateAsset(urp, "Assets/Vitrail/Settings/URP.asset");
            GraphicsSettings.defaultRenderPipeline = urp;
            for (var i = 0; i < QualitySettings.names.Length; i++)
            {
                QualitySettings.SetQualityLevel(i, false);
                QualitySettings.renderPipeline = urp;
            }
            var scene = EditorSceneManager.NewScene(NewSceneSetup.EmptyScene, NewSceneMode.Single);
            var cam = new GameObject("Camera").AddComponent<Camera>();
            cam.orthographic = true;
            cam.orthographicSize = 5;
            cam.transform.position = new Vector3(0, 0, -10);
            cam.clearFlags = CameraClearFlags.SolidColor;
            cam.backgroundColor = Color.black;
            cam.GetUniversalAdditionalCameraData().renderPostProcessing = false;
            var quad = GameObject.CreatePrimitive(PrimitiveType.Quad);
            quad.name = "Map";
            var tex = AssetDatabase.LoadAssetAtPath<Texture2D>("Assets/Vitrail/Resources/map.jpg");
            var mat = new Material(Shader.Find("Universal Render Pipeline/Unlit")) { mainTexture = tex };
            AssetDatabase.CreateAsset(mat, "Assets/Vitrail/Settings/Map.mat");
            quad.GetComponent<MeshRenderer>().sharedMaterial = mat;
            quad.transform.localScale = new Vector3(40, 40 * tex.height / (float)tex.width, 1);
            new GameObject("Vitrail").AddComponent<VitrailPanel>();
            EditorSceneManager.SaveScene(scene, "Assets/Vitrail/Lab.unity");
            AssetDatabase.SaveAssets();
        }

        /// <summary>Sets the lab scene up for looking at a page in play mode: the map follows the view, the page loads.</summary>
        public static void Show()
        {
            var scene = EditorSceneManager.OpenScene("Assets/Vitrail/Lab.unity");
            var map = GameObject.Find("Map");
            if (map.GetComponent<FitMap>() == null) map.AddComponent<FitMap>();
            var cam = Object.FindFirstObjectByType<Camera>();
            cam.tag = "MainCamera";
            var panel = Object.FindFirstObjectByType<VitrailPanel>();
            panel.HtmlPath = Arg("-page", panel.HtmlPath);
            EditorSceneManager.SaveScene(scene);
        }

        private static string Arg(string name, string fallback)
        {
            var a = System.Environment.GetCommandLineArgs();
            for (var i = 0; i < a.Length - 1; i++) if (a[i] == name) return a[i + 1];
            return fallback;
        }

        public static void Capture()
        {
            EditorSceneManager.OpenScene("Assets/Vitrail/Lab.unity");
            var rd = AssetDatabase.LoadAssetAtPath<UniversalRendererData>("Assets/Vitrail/Settings/Renderer.asset");
            if (rd.intermediateTextureMode != IntermediateTextureMode.Always) { rd.intermediateTextureMode = IntermediateTextureMode.Always; EditorUtility.SetDirty(rd); AssetDatabase.SaveAssets(); }
            var page = Arg("-page", "");
            var outDir = Arg("-out", "Captures");
            int w = int.Parse(Arg("-w", "1422")), h = int.Parse(Arg("-h", "118"));
            Directory.CreateDirectory(outDir);
            var cam = Object.FindFirstObjectByType<Camera>();
            var panel = Object.FindFirstObjectByType<VitrailPanel>();
            var name = Path.GetFileNameWithoutExtension(page);
            // The map picture covers the whole view, like CSS "background-size: cover".
            var map = GameObject.Find("Map").transform;
            var tex = (Texture2D)map.GetComponent<MeshRenderer>().sharedMaterial.mainTexture;
            float viewH = cam.orthographicSize * 2, viewW = viewH * w / h, ta = tex.width / (float)tex.height;
            map.localScale = viewW / viewH > ta ? new Vector3(viewW, viewW / ta, 1) : new Vector3(viewH * ta, viewH, 1);
            panel.enabled = false;
            Save(cam, w, h, Path.Combine(outDir, name + "-bg.png"));
            panel.enabled = true;
            panel.Load(page, w, h, 1);
            var tx = panel.Texture;
            Debug.Log($"Vitrail texture {(tx == null ? "null" : tx.width + "x" + tx.height)} shader {(Shader.Find("Hidden/Vitrail/Composite") != null)} pipeline {GraphicsSettings.currentRenderPipeline}");
            if (tx != null)
            {
                var px = tx.GetPixels32(); int opaque = 0; foreach (var c in px) if (c.a > 0) opaque++;
                Debug.Log($"Vitrail painted pixels {opaque}/{px.Length}");
                var copy = new Texture2D(tx.width, tx.height, TextureFormat.RGBA32, false, true);
                copy.SetPixels32(px); copy.Apply();
                File.WriteAllBytes(Path.Combine(outDir, name + "-tex.png"), copy.EncodeToPNG());
            }
            var t0 = Time.realtimeSinceStartup;
            Save(cam, w, h, Path.Combine(outDir, name + "-unity.png"));
            Debug.Log($"Vitrail captured {name} ({w}x{h}); render+composite {(Time.realtimeSinceStartup - t0) * 1000:F1} ms");
            panel.enabled = false;
        }

        /// <summary>The same page painted by the CPU painter and by the GPU painter, saved side by side for comparison.
        /// Args: -page, -out, -w, -h, -scale.</summary>
        public static void GpuCapture()
        {
            GpuCaptureOne(Arg("-page", ""), Arg("-out", "Captures"), int.Parse(Arg("-w", "600")), int.Parse(Arg("-h", "130")),
                float.Parse(Arg("-scale", "1"), System.Globalization.CultureInfo.InvariantCulture), Arg("-aa", "0") == "1");
        }

        /// <summary>Several pages in one run. -list: "page.html w h scale" per line; -out; -aa.</summary>
        public static void GpuSuite()
        {
            foreach (var line in File.ReadAllLines(Arg("-list", "")))
            {
                var f = line.Split(' ');
                if (f.Length < 4) continue;
                GpuCaptureOne(f[0], Arg("-out", "Captures"), int.Parse(f[1]), int.Parse(f[2]),
                    float.Parse(f[3], System.Globalization.CultureInfo.InvariantCulture), Arg("-aa", "0") == "1");
            }
        }

        private static unsafe void GpuCaptureOne(string page, string outDir, int w, int h, float scale, bool aa)
        {
            Directory.CreateDirectory(outDir);
            var name = Path.GetFileNameWithoutExtension(page);
            var ui = VitrailNative.vitrail_create(File.ReadAllText(page), new System.Uri(Path.GetFullPath(page)).AbsoluteUri, (uint)w, (uint)h, scale);
            // Pages named "*-scrolled": the first scroll container scrolled 900 px down (the GPU moves its content).
            if (name.EndsWith("-scrolled"))
                for (var k = 0; k < 9; k++) VitrailNative.vitrail_scroll(ui, w / 2f, h / 2f, 0, 100 * scale);
            var cpu = new byte[w * h * 4];
            fixed (byte* p = cpu) VitrailNative.vitrail_render(ui, p, (System.UIntPtr)cpu.Length);
            var tex = new Texture2D(w, h, TextureFormat.RGBA32, false, true);
            tex.SetPixelData(cpu, 0); tex.Apply();
            File.WriteAllBytes(Path.Combine(outDir, name + "-cpu.png"), Flip(tex).EncodeToPNG());
            var gpu = new GpuPainter();
            // A game texture for pages that show texture:test: 64×32, sRGB, top half red, bottom half blue, a white
            // square at the top left (so a flip or a colour-space slip shows).
            var test = new Texture2D(64, 32, TextureFormat.RGBA32, false, false) { filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
            for (var y = 0; y < 32; y++)
                for (var x = 0; x < 64; x++)
                {
                    var top = y >= 16; // Unity rows run bottom up
                    var c = top ? new Color32(200, 60, 50, 255) : new Color32(50, 90, 200, 255);
                    if (top && x < 16 && y >= 24) c = new Color32(255, 255, 255, 255);
                    test.SetPixel(x, y, c);
                }
            test.Apply();
            gpu.SetTexture("test", test);
            // The game's picture under the page (backdrop-filter reads it): a plain #20302c, as the -overbg references.
            var scene = new Texture2D(1, 1, TextureFormat.RGBA32, false, false);
            // Pages compared with the CPU painter (which leaves glass see-through) get a clear scene.
            scene.SetPixel(0, 0, name.Contains("backdrop") ? new Color32(0x20, 0x30, 0x2c, 255) : new Color32(0, 0, 0, 0));
            scene.Apply();
            Shader.SetGlobalTexture("_VitrailScene", scene);
            // aa: edges antialiased in the triangles, no multisampling.
            if (aa) VitrailNative.vitrail_set_gpu(ui, 2);
            var t0 = Time.realtimeSinceStartup;
            gpu.Paint(ui, w, h, aa ? 1 : 8);
            var ms = (Time.realtimeSinceStartup - t0) * 1000;
            var resolved = new RenderTexture(w, h, 0, RenderTextureFormat.ARGB32, RenderTextureReadWrite.Linear);
            Graphics.Blit(gpu.Target, resolved);
            var prev = RenderTexture.active;
            RenderTexture.active = resolved;
            var g = new Texture2D(w, h, TextureFormat.RGBA32, false, true);
            g.ReadPixels(new Rect(0, 0, w, h), 0, 0); g.Apply();
            RenderTexture.active = prev;
            File.WriteAllBytes(Path.Combine(outDir, name + (aa ? "-gpuaa.png" : "-gpu.png")), g.EncodeToPNG());
            Debug.Log($"Vitrail GPU capture {name}: build+draw {ms:F1} ms");
            gpu.Dispose();
            VitrailNative.vitrail_destroy(ui);
        }

        /// <summary>A copy turned upside down (the native buffer's first row is the page's top).</summary>
        private static Texture2D Flip(Texture2D t)
        {
            var px = t.GetPixels32();
            var o = new Color32[px.Length];
            for (var y = 0; y < t.height; y++) System.Array.Copy(px, y * t.width, o, (t.height - 1 - y) * t.width, t.width);
            var r = new Texture2D(t.width, t.height, TextureFormat.RGBA32, false, true);
            r.SetPixels32(o); r.Apply();
            return r;
        }

        private static void Save(Camera cam, int w, int h, string path)
        {
            var rt = new RenderTexture(w, h, 24, RenderTextureFormat.ARGB32, RenderTextureReadWrite.sRGB);
            cam.targetTexture = rt;
            cam.aspect = w / (float)h;
            cam.Render();
            var prev = RenderTexture.active;
            RenderTexture.active = rt;
            var tex = new Texture2D(w, h, TextureFormat.RGBA32, false, false);
            tex.ReadPixels(new Rect(0, 0, w, h), 0, 0);
            tex.Apply();
            RenderTexture.active = prev;
            cam.targetTexture = null;
            File.WriteAllBytes(path, tex.EncodeToPNG());
            Object.DestroyImmediate(tex);
            rt.Release();
        }
    }
}
