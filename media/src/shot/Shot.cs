using System.Collections;
using System.IO;
using UnityEngine;
using Vitrail;

// Paints README pictures with Vitrail UI over a real scene: a background picture on a quad fills an orthographic
// camera that renders into a texture of the picture's size; the page is drawn over that camera, so its glass blurs
// the scene as in a game. Jobs come from show/jobs.txt: "page background scale out" per line.
public sealed class Shot : MonoBehaviour
{
    public Camera cam;
    public MeshRenderer quad;

    private IEnumerator Start()
    {
        var outDir = Arg("-out", Path.Combine(Application.dataPath, "..", "out"));
        Directory.CreateDirectory(outDir);
        var log = new System.Text.StringBuilder();
        foreach (var line in File.ReadAllLines(Path.Combine(Application.streamingAssetsPath, "show", "jobs.txt")))
        {
            var p = line.Split(' ');
            if (p.Length < 4) continue;
            yield return Job(p[0], p[1], float.Parse(p[2], System.Globalization.CultureInfo.InvariantCulture), Path.Combine(outDir, p[3]), log);
        }
        File.WriteAllText(Path.Combine(outDir, "log.txt"), log.ToString());
        Application.Quit();
    }

    private IEnumerator Job(string page, string bg, float scale, string outPath, System.Text.StringBuilder log)
    {
        var tex = new Texture2D(2, 2, TextureFormat.RGBA32, false, false);
        tex.LoadImage(File.ReadAllBytes(Path.Combine(Application.streamingAssetsPath, bg)));
        tex.filterMode = FilterMode.Point;
        tex.wrapMode = TextureWrapMode.Clamp;
        int w = tex.width, h = tex.height;
        quad.material.SetTexture("_BaseMap", tex);
        var rt = new RenderTexture(w, h, 24, RenderTextureFormat.ARGB32, RenderTextureReadWrite.sRGB);
        cam.targetTexture = rt;
        cam.aspect = w / (float)h;
        cam.orthographicSize = 5;
        quad.transform.localScale = new Vector3(10f * w / h, 10f, 1f);
        var panel = new WebPanel(page, 0) { TargetCamera = cam };
        for (var i = 0; i < 300; i++) { panel.Update(scale); yield return null; }
        yield return new WaitForEndOfFrame();
        var prev = RenderTexture.active;
        RenderTexture.active = rt;
        var shot = new Texture2D(w, h, TextureFormat.RGB24, false, false);
        shot.ReadPixels(new Rect(0, 0, w, h), 0, 0);
        shot.Apply();
        RenderTexture.active = prev;
        File.WriteAllBytes(outPath, shot.EncodeToPNG());
        log.AppendLine($"{page} {w}x{h} scale {scale} paints {panel.PaintsStarted}");
        panel.Dispose();
        cam.targetTexture = null;
        rt.Release();
    }

    private static string Arg(string name, string fallback)
    {
        var a = System.Environment.GetCommandLineArgs();
        for (var i = 0; i < a.Length - 1; i++) if (a[i] == name) return a[i + 1];
        return fallback;
    }
}
