using System;
using System.Runtime.InteropServices;
using System.IO;
using Unity.Collections;
using Unity.Collections.LowLevel.Unsafe;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.RenderGraphModule;
using UnityEngine.Rendering.Universal;

namespace VitrailLab
{
    /// <summary>
    /// A full-screen HTML page over the game camera. The page is painted by Blitz into premultiplied sRGB bytes; the
    /// composite pass mixes it over the camera picture the way a browser does (in sRGB), so see-through colours look as
    /// they do in Chrome and don't turn grey as UI Toolkit's linear blending made them.
    /// </summary>
    [ExecuteAlways]
    public sealed class VitrailPanel : MonoBehaviour
    {
        public string HtmlPath = "";
        public float Scale = 1;
        /// <summary>Scale the page with the screen (CSS px = device px × height / 1080), as a browser zooms on a big screen.</summary>
        public bool AutoScale = true;
        private float EffectiveScale => AutoScale ? Mathf.Max(0.5f, Screen.height / 1080f) : Scale;
        /// <summary>The page's width in CSS pixels.</summary>
        public float CssWidth => Screen.width / EffectiveScale;
        private static readonly int TexId = Shader.PropertyToID("_VitrailTex");
        private static readonly int MaskId = Shader.PropertyToID("_VitrailMask");
        private static readonly int BlurId = Shader.PropertyToID("_VitrailBlur");
        private static readonly int MatrixId = Shader.PropertyToID("_VitrailRegionMatrix");
        private static readonly int OffsetId = Shader.PropertyToID("_VitrailRegionOffset");
        private static readonly int InfoId = Shader.PropertyToID("_VitrailRegionInfo");
        private static readonly int SigmaId = Shader.PropertyToID("_VitrailSigma");
        private static readonly int[] BlurredIds = { Shader.PropertyToID("_VitrailBlurred0"), Shader.PropertyToID("_VitrailBlurred1"), Shader.PropertyToID("_VitrailBlurred2"), Shader.PropertyToID("_VitrailBlurred3") };
        private const int MaxRegions = 8, MaxBlurs = 4;
        private readonly Matrix4x4[] _matrices = new Matrix4x4[MaxRegions];
        private readonly Vector4[] _offsets = new Vector4[MaxRegions];
        private readonly Vector4[] _infos = new Vector4[MaxRegions];
        private readonly System.Collections.Generic.List<float> _sigmas = new();
        private static readonly int TexelId = Shader.PropertyToID("_VitrailTexel");
        private Texture2D _mask;
        private readonly float[] _backdrop = new float[21 * MaxRegions];
        private bool _hasBackdrop;
        private IntPtr _ui;
        private Texture2D _tex;
        private int _w, _h;
        private bool _dirty = true;
        private CompositePass _pass;

        public Texture2D Texture => _tex;

        public void Load(string path, int width, int height, float scale)
        {
            Release();
            HtmlPath = path;
            Scale = scale;
            var html = File.ReadAllText(path);
            var baseUrl = new Uri(Path.GetFullPath(path)).AbsoluteUri;
            _w = width; _h = height;
            _ui = VitrailNative.vitrail_create(html, baseUrl, (uint)width, (uint)height, scale);
            _tex = new Texture2D(width, height, TextureFormat.RGBA32, false, true) { name = "Vitrail", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
            _dirty = true;
            Paint();
        }

        /// <summary>Paints the page into the texture (only when something changed).</summary>
        public unsafe float Paint()
        {
            if (_ui == IntPtr.Zero || (!_dirty && VitrailNative.vitrail_needs_paint(_ui) == 0)) return 0;
            var t0 = Time.realtimeSinceStartup;
            var data = _tex.GetRawTextureData<byte>();
            VitrailNative.vitrail_render(_ui, (byte*)NativeArrayUnsafeUtility.GetUnsafePtr(data), (UIntPtr)data.Length);
            _tex.Apply(false, false);
            // The backdrop mask: where the page asks to blur the game picture under it.
            if (_mask == null) _mask = new Texture2D(_w, _h, TextureFormat.RGBA32, false, true) { name = "Vitrail mask", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
            var maskData = _mask.GetRawTextureData<byte>();
            uint regions;
            fixed (float* p = _backdrop)
                regions = VitrailNative.vitrail_backdrop(_ui, (byte*)NativeArrayUnsafeUtility.GetUnsafePtr(maskData), (UIntPtr)maskData.Length, p, MaxRegions);
            _hasBackdrop = regions > 0;
            // Each element's colour matrix, and which blurred copy of the scene it reads: one copy per distinct blur.
            _sigmas.Clear();
            for (var i = 0; i < MaxRegions; i++)
            {
                if (i >= regions) { _matrices[i] = Matrix4x4.identity; _offsets[i] = Vector4.zero; _infos[i] = new Vector4(-1, 0, 0, 0); continue; }
                var o = i * 21;
                var m = new Matrix4x4();
                for (var r = 0; r < 4; r++) for (var c = 0; c < 4; c++) m[r, c] = _backdrop[o + 1 + r * 5 + c];
                _matrices[i] = m;
                _offsets[i] = new Vector4(_backdrop[o + 5], _backdrop[o + 10], _backdrop[o + 15], _backdrop[o + 20]);
                var sigma = _backdrop[o];
                var g = -1;
                if (sigma > 0.01f)
                {
                    g = _sigmas.FindIndex(s => Mathf.Abs(s - sigma) < 0.01f);
                    if (g < 0 && _sigmas.Count < MaxBlurs) { _sigmas.Add(sigma); g = _sigmas.Count - 1; }
                    if (g < 0) g = MaxBlurs - 1;
                }
                _infos[i] = new Vector4(g, 0, 0, 0);
            }
            if (_hasBackdrop) _mask.Apply(false, false);
            _dirty = false;
            SetGlobals();
            return (Time.realtimeSinceStartup - t0) * 1000;
        }

        private void SetGlobals()
        {
            Shader.SetGlobalTexture(TexId, _tex);
            Shader.SetGlobalTexture(MaskId, _mask);
            Shader.SetGlobalVector(BlurId, new Vector4(0, _hasBackdrop ? 1 : 0, 0, 0));
            Shader.SetGlobalMatrixArray(MatrixId, _matrices);
            Shader.SetGlobalVectorArray(OffsetId, _offsets);
            Shader.SetGlobalVectorArray(InfoId, _infos);
            Shader.SetGlobalVector(TexelId, new Vector4(1f / _w, 1f / _h, 0, 0));
            if (_pass != null) { _pass.Backdrop = _hasBackdrop; _pass.Sigmas.Clear(); _pass.Sigmas.AddRange(_sigmas); }
        }

        /// <summary>A button or other element with `data-action` (or an `id`) was clicked: (action, id).</summary>
        public event Action<string, string> Clicked;

        // What the game set on the page, replayed after the page reloads (its file changed).
        private readonly System.Collections.Generic.Dictionary<string, Action> _state = new();
        private FileSystemWatcher _watcher;
        private volatile bool _fileChanged;
        private Vector2 _lastPointer = new(-1, -1);
        private float _loadedScale;

        /// <summary>Replaces the text of every element matching the CSS selector.</summary>
        public void SetText(string selector, string value) => Apply("text|" + selector, () => VitrailNative.vitrail_set_text(_ui, selector, value));

        /// <summary>Sets an attribute (null removes it) on every element matching the selector.</summary>
        public void SetAttr(string selector, string name, string value) => Apply("attr|" + selector + "|" + name, () => VitrailNative.vitrail_set_attr(_ui, selector, name, value));

        /// <summary>Adds or removes a class on every element matching the selector.</summary>
        public void SetClass(string selector, string cls, bool on) => Apply("class|" + selector + "|" + cls, () => VitrailNative.vitrail_set_class(_ui, selector, cls, on ? 1 : 0));

        /// <summary>Sets an inline style property (also custom properties like --progress); empty removes it.</summary>
        public void SetStyle(string selector, string property, string value) => Apply("style|" + selector + "|" + property, () => VitrailNative.vitrail_set_style(_ui, selector, property, value));

        /// <summary>Whether a screen point (Unity's, y up) is over a part of the page that takes the pointer.</summary>
        public bool IsOverUi(Vector2 screen) => _ui != IntPtr.Zero && VitrailNative.vitrail_hit(_ui, screen.x, Screen.height - screen.y) != 0;

        private void Apply(string key, Action op)
        {
            _state[key] = op;
            if (_ui != IntPtr.Zero) op();
        }

        /// <summary>In play mode the page in HtmlPath fills the game view, follows its size, takes the mouse and reloads
        /// when its folder's files change (save the CSS, see it on the next frame).</summary>
        private unsafe void Update()
        {
            if (!Application.isPlaying || string.IsNullOrEmpty(HtmlPath)) return;
            // In a built player the pages come from StreamingAssets/pages (the editor reads them where they are written).
            if (!File.Exists(HtmlPath))
            {
                var shipped = Path.Combine(Application.streamingAssetsPath, "pages", Path.GetFileName(HtmlPath.Replace('\\', '/')));
                if (!File.Exists(shipped)) return;
                HtmlPath = shipped;
            }
            if (_ui == IntPtr.Zero || _w != Screen.width || _h != Screen.height || _fileChanged || !Mathf.Approximately(_loadedScale, EffectiveScale))
            {
                _fileChanged = false;
                _loadedScale = EffectiveScale;
                Load(HtmlPath, Screen.width, Screen.height, _loadedScale);
                foreach (var op in _state.Values) op();
                Watch();
            }
            var m = (Vector2)Input.mousePosition;
            var p = new Vector2(m.x, Screen.height - m.y);
            if (p != _lastPointer) { VitrailNative.vitrail_pointer_move(_ui, p.x, p.y); _lastPointer = p; }
            if (Input.GetMouseButtonDown(0)) VitrailNative.vitrail_pointer_down(_ui, p.x, p.y);
            if (Input.GetMouseButtonUp(0))
            {
                var action = stackalloc byte[256];
                var id = stackalloc byte[256];
                if (VitrailNative.vitrail_pointer_up(_ui, p.x, p.y, action, id, (UIntPtr)256) != 0)
                    Clicked?.Invoke(Marshal.PtrToStringUTF8((IntPtr)action), Marshal.PtrToStringUTF8((IntPtr)id));
            }
            var ms = Paint();
            if (ms > 0) LastPaintMs = ms;
        }

        /// <summary>How long the last repaint took (style, layout, painting and upload), in ms.</summary>
        public float LastPaintMs { get; private set; }

        private void Watch()
        {
            var dir = Path.GetDirectoryName(Path.GetFullPath(HtmlPath));
            if (_watcher != null && _watcher.Path == dir) return;
            _watcher?.Dispose();
            _watcher = new FileSystemWatcher(dir) { IncludeSubdirectories = true, NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName | NotifyFilters.Size };
            FileSystemEventHandler on = (_, e) =>
            {
                var ext = Path.GetExtension(e.FullPath).ToLowerInvariant();
                if (ext == ".html" || ext == ".css" || ext == ".svg" || ext == ".png") _fileChanged = true;
            };
            _watcher.Changed += on;
            _watcher.Created += on;
            _watcher.Renamed += (s, e) => on(s, e);
            _watcher.EnableRaisingEvents = true;
        }

        private void OnEnable()
        {
            _pass ??= new CompositePass();
            RenderPipelineManager.beginCameraRendering += Enqueue;
        }

        private void OnDisable()
        {
            _watcher?.Dispose();
            _watcher = null;
            RenderPipelineManager.beginCameraRendering -= Enqueue;
            Release();
        }

        private void Release()
        {
            if (_ui != IntPtr.Zero) VitrailNative.vitrail_destroy(_ui);
            _ui = IntPtr.Zero;
            if (_tex != null) DestroyImmediate(_tex);
            if (_mask != null) DestroyImmediate(_mask);
            _tex = null;
            _mask = null;
        }

        private void Enqueue(ScriptableRenderContext context, Camera camera)
        {
            if (_tex == null || _pass.Material == null || camera.cameraType != CameraType.Game)
            {
                Debug.Log($"Vitrail skip: tex {_tex != null} mat {_pass.Material != null} type {camera.cameraType}");
                return;
            }
            if (camera.GetUniversalAdditionalCameraData() is not { } data || data.scriptableRenderer is not { } renderer) return;
            SetGlobals();
            renderer.EnqueuePass(_pass);
        }

        private sealed class CompositePass : ScriptableRenderPass
        {
            public readonly Material Material;
            public bool Backdrop;
            public readonly System.Collections.Generic.List<float> Sigmas = new();

            public CompositePass()
            {
                var shader = Shader.Find("Hidden/Vitrail/Composite");
                if (shader != null) Material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
                renderPassEvent = RenderPassEvent.AfterRenderingPostProcessing;
                // The page is mixed over a copy of the camera picture, which needs the camera's own colour texture.
                requiresIntermediateTexture = true;
                profilingSampler = new ProfilingSampler("Vitrail composite");
            }

            private sealed class Data { public TextureHandle Source; public TextureHandle[] Blurred; public Material Material; public int Pass; public float Sigma; }

            public override void RecordRenderGraph(RenderGraph graph, ContextContainer frame)
            {
                var resources = frame.Get<UniversalResourceData>();
                if (resources.isActiveTargetBackBuffer) return;
                var src = resources.activeColorTexture;
                var desc = graph.GetTextureDesc(src);
                desc.name = "Vitrail scene copy";
                desc.clearBuffer = false;
                var copy = graph.CreateTexture(desc);
                using (var b = graph.AddRasterRenderPass<Data>("Vitrail copy", out var d))
                {
                    d.Source = src;
                    b.UseTexture(src, AccessFlags.Read);
                    b.SetRenderAttachment(copy, 0, AccessFlags.Write);
                    b.SetRenderFunc(static (Data d, RasterGraphContext ctx) => Blitter.BlitTexture(ctx.cmd, d.Source, new Vector4(1, 1, 0, 0), 0, false));
                }
                var blurred = new TextureHandle[4];
                if (Backdrop && Sigmas.Count > 0)
                {
                    // The scene in sRGB (CSS filters work on sRGB values), then one blurred copy per distinct blur.
                    var bdesc = desc;
                    bdesc.colorFormat = UnityEngine.Experimental.Rendering.GraphicsFormat.R8G8B8A8_UNorm;
                    bdesc.name = "Vitrail backdrop";
                    var srgb = graph.CreateTexture(bdesc);
                    var tmp = graph.CreateTexture(bdesc);
                    Blit(graph, "Vitrail to sRGB", copy, srgb, 1, 0);
                    for (var g = 0; g < Sigmas.Count && g < 4; g++)
                    {
                        blurred[g] = graph.CreateTexture(bdesc);
                        Blit(graph, "Vitrail blur x", srgb, tmp, 2, Sigmas[g]);
                        Blit(graph, "Vitrail blur y", tmp, blurred[g], 3, Sigmas[g]);
                    }
                }
                using (var b = graph.AddRasterRenderPass<Data>("Vitrail composite", out var d))
                {
                    d.Source = copy;
                    d.Blurred = blurred;
                    d.Material = Material;
                    b.UseTexture(copy, AccessFlags.Read);
                    foreach (var h in blurred) if (h.IsValid()) b.UseTexture(h, AccessFlags.Read);
                    b.SetRenderAttachment(src, 0, AccessFlags.Write);
                    b.AllowGlobalStateModification(true);
                    b.SetRenderFunc(static (Data d, RasterGraphContext ctx) =>
                    {
                        for (var g = 0; g < 4; g++) if (d.Blurred[g].IsValid()) ctx.cmd.SetGlobalTexture(BlurredIds[g], d.Blurred[g]);
                        Blitter.BlitTexture(ctx.cmd, d.Source, new Vector4(1, 1, 0, 0), d.Material, 0);
                    });
                }
            }

            private void Blit(RenderGraph graph, string name, TextureHandle from, TextureHandle to, int pass, float sigma)
            {
                using var b = graph.AddRasterRenderPass<Data>(name, out var d);
                d.Source = from;
                d.Material = Material;
                d.Pass = pass;
                d.Sigma = sigma;
                b.UseTexture(from, AccessFlags.Read);
                b.SetRenderAttachment(to, 0, AccessFlags.Write);
                b.AllowGlobalStateModification(true);
                b.SetRenderFunc(static (Data d, RasterGraphContext ctx) =>
                {
                    ctx.cmd.SetGlobalFloat(SigmaId, d.Sigma);
                    Blitter.BlitTexture(ctx.cmd, d.Source, new Vector4(1, 1, 0, 0), d.Material, d.Pass);
                });
            }
        }
    }
}
