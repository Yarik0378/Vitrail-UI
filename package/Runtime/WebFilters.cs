using System;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Rendering;

namespace Vitrail
{
    /// <summary>
    /// CSS `filter` for the GPU painter: a group's texture runs through its steps (Gaussian blur, drop
    /// shadow, colour matrix: grayscale, brightness, saturate, hue-rotate, sepia, invert, contrast, opacity) before it is
    /// composited. The passes cover only the group's bounds and ping-pong between three scratch textures. Shader:
    /// Resources/Vitrail/WebFilter.shader. backdrop-filter runs the same steps on the page so far over the game's picture. Against Chrome the colour matrices match exactly and blur/shadow profiles within 1–3/255.
    /// </summary>
    public sealed class WebFilters : IDisposable
    {
        /// <summary>One step as the library hands it over (gpu::FilterStep, 128 bytes): kind 1 blur, 2 shadow, 3 matrix;
        /// the matrix is 4×5 row-major in M0..M4 (20 floats in a row).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct Step
        {
            public uint Kind;
            public float Sigma, Dx, Dy;
            public Vector4 Color;
            public Vector4 M0, M1, M2, M3, M4;
            public Vector4 Bounds;
        }

        private const int MatrixPass = 0, BlurPass = 1, SeedPass = 2, UnderPass = 3, BackdropPass = 4, OverPass = 5, LayerPass = 6, ClearPass = 7;
        private static readonly int SrcId = Shader.PropertyToID("_Src"), ShadowId = Shader.PropertyToID("_Shadow");
        private static readonly int BoundsId = Shader.PropertyToID("_Bounds"), BlurDirId = Shader.PropertyToID("_BlurDir");
        private static readonly int OffsetId = Shader.PropertyToID("_Offset"), ShadowColorId = Shader.PropertyToID("_ShadowColor");
        private static readonly int[] RowIds = { Shader.PropertyToID("_M0"), Shader.PropertyToID("_M1"), Shader.PropertyToID("_M2"), Shader.PropertyToID("_M3") };
        private static readonly int MOffId = Shader.PropertyToID("_MOff");

        private readonly Material _material;
        // Scratch textures: page-sized, and a half-sized set for glass with a wide blur. _scratch is the set in use.
        private readonly RenderTexture[] _full = new RenderTexture[3], _half = new RenderTexture[3];
        private RenderTexture[] _scratch;
        private static readonly int TargetSizeId = Shader.PropertyToID("_TargetSize");
        private readonly MaterialPropertyBlock _props = new();
        private readonly float[] _m = new float[20];
        private Step[] _steps = new Step[0];

        public WebFilters()
        {
            var shader = UnityEngine.Resources.Load<Shader>("Vitrail/WebFilter");
            if (shader != null && shader.isSupported) _material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
        }

        /// <summary>Reads the frame's filter steps from the library.</summary>
        public void Load(IntPtr ui)
        {
            var n = WebNative.vitrail_gpu_filters(ui, IntPtr.Zero, UIntPtr.Zero);
            if (n <= 0) return;
            if (_steps.Length < n) _steps = new Step[Mathf.NextPowerOfTwo(n)];
            var h = GCHandle.Alloc(_steps, GCHandleType.Pinned);
            try { WebNative.vitrail_gpu_filters(ui, h.AddrOfPinnedObject(), (UIntPtr)(_steps.Length * Marshal.SizeOf<Step>())); }
            finally { h.Free(); }
        }

        /// <summary>Records steps first..first+count on `source` into `cmd` and returns the texture with the result. The
        /// painter's view-projection (device px, y down) must be set.</summary>
        public RenderTexture Run(CommandBuffer cmd, RenderTexture source, int first, int count, int width, int height, bool backdrop = false)
        {
            if (_material == null || count <= 0) return source;
            // Glass with a blur of 6 px or more blurs at half size: the result is just as smooth, at a quarter of the cost.
            // Glass with a wide blur runs smaller, as Skia does for big blurs: a quarter of the size from σ 6 px, a half
            // from σ 3 px (the blur hides the lost detail; the cost falls with the pixel count).
            var down = backdrop && _steps[first].Kind == 1 ? (_steps[first].Sigma >= 6 ? 4 : _steps[first].Sigma >= 3 ? 2 : 1) : 1;
            var half = down > 1;
            _scratch = half ? _half : _full;
            if (half) { width = Mathf.Max(1, width / down); height = Mathf.Max(1, height / down); }
            for (var i = 0; i < 3; i++)
            {
                var t = _scratch[i];
                if (t != null && t.width == width && t.height == height) continue;
                if (t != null) UnityEngine.Object.Destroy(t);
                _scratch[i] = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 0) { sRGB = false }) { name = half ? "Web HUD glass (small)" : "Web HUD filter" };
                _scratch[i].filterMode = FilterMode.Bilinear;
                _scratch[i].Create();
            }
            var cur = -1; // -1: the source, else a scratch texture
            if (backdrop)
            {
                // backdrop-filter's source: the page so far over the game's picture (_VitrailScene), within the bounds.
                _props.Clear();
                _props.SetVector(BoundsId, _steps[first].Bounds);
                Pass(cmd, BackdropPass, source, 0);
                cur = 0;
            }
            for (var s = first; s < first + count; s++)
            {
                var st = _steps[s];
                _props.Clear();
                _props.SetVector(BoundsId, st.Bounds);
                switch (st.Kind)
                {
                    case 1:
                    {
                        var dst = Other(cur);
                        Blur(cmd, Tex(source, cur), dst, st.Sigma, cur);
                        cur = dst;
                        break;
                    }
                    case 2:
                    {
                        // The seed (the alpha moved and tinted), blurred, then the input drawn over it.
                        var seed = Other(cur);
                        _props.SetVector(OffsetId, new Vector4(st.Dx, st.Dy, 0, 0));
                        _props.SetVector(ShadowColorId, st.Color);
                        Pass(cmd, SeedPass, Tex(source, cur), seed);
                        if (st.Sigma > 0.01f) Blur(cmd, _scratch[seed], seed, st.Sigma, cur);
                        var dst = Other(cur, seed);
                        _props.SetTexture(ShadowId, _scratch[seed]);
                        Pass(cmd, UnderPass, Tex(source, cur), dst);
                        cur = dst;
                        break;
                    }
                    case 3:
                    {
                        Copy(st.M0, 0); Copy(st.M1, 4); Copy(st.M2, 8); Copy(st.M3, 12); Copy(st.M4, 16);
                        for (var r = 0; r < 4; r++) _props.SetVector(RowIds[r], new Vector4(_m[r * 5], _m[r * 5 + 1], _m[r * 5 + 2], _m[r * 5 + 3]));
                        _props.SetVector(MOffId, new Vector4(_m[4], _m[9], _m[14], _m[19]));
                        var dst = Other(cur);
                        Pass(cmd, MatrixPass, Tex(source, cur), dst);
                        cur = dst;
                        break;
                    }
                }
            }
            return Tex(source, cur);
        }

        private static readonly int SrcRectId = Shader.PropertyToID("_SrcRect");

        /// <summary>Draws `src`, which holds page rectangle `rect` (device px), over `dst` (the whole page, premultiplied).</summary>
        public void Over(CommandBuffer cmd, RenderTexture src, RenderTexture dst, RectInt rect)
        {
            if (_material == null) return;
            _props.Clear();
            _props.SetVector(BoundsId, new Vector4(rect.xMin, rect.yMin, rect.xMax, rect.yMax));
            _props.SetVector(SrcRectId, new Vector4(rect.x, rect.y, rect.width, rect.height));
            _props.SetTexture(SrcId, src);
            cmd.SetRenderTarget(dst);
            cmd.DrawProcedural(Matrix4x4.identity, _material, OverPass, MeshTopology.Triangles, 6, 1, _props);
        }

        private static readonly int LayerM0Id = Shader.PropertyToID("_LayerM0"), LayerM1Id = Shader.PropertyToID("_LayerM1");

        /// <summary>Lays composited layer `src` (holding page rectangle `rect`, as painted) on `dst`, moved by matrix
        /// (m.x m.y m.z m.w, e f in t.xy) and faded by t.z.</summary>
        public void Layer(CommandBuffer cmd, RenderTexture src, RenderTexture dst, RectInt rect, Vector4 m, Vector3 t)
        {
            if (_material == null) return;
            _props.Clear();
            _props.SetVector(BoundsId, new Vector4(rect.xMin, rect.yMin, rect.xMax, rect.yMax));
            _props.SetVector(SrcRectId, new Vector4(rect.x, rect.y, rect.width, rect.height));
            _props.SetTexture(SrcId, src);
            _props.SetVector(LayerM0Id, m);
            _props.SetVector(LayerM1Id, new Vector4(t.x, t.y, t.z, 0));
            cmd.SetRenderTarget(dst);
            cmd.DrawProcedural(Matrix4x4.identity, _material, LayerPass, MeshTopology.Triangles, 6, 1, _props);
        }

        /// <summary>Clears page rectangle `bounds` (x0 y0 x1 y1) of the bound target (through the painter's view-projection).</summary>
        public void Clear(CommandBuffer cmd, Vector4 bounds)
        {
            if (_material == null) return;
            _props.Clear();
            _props.SetVector(BoundsId, bounds);
            cmd.DrawProcedural(Matrix4x4.identity, _material, ClearPass, MeshTopology.Triangles, 6, 1, _props);
        }

        private void Copy(Vector4 v, int at) { _m[at] = v.x; _m[at + 1] = v.y; _m[at + 2] = v.z; _m[at + 3] = v.w; }
        private RenderTexture Tex(RenderTexture source, int k) => k < 0 ? source : _scratch[k];
        private static int Other(int a, int b = -2) { var k = 0; while (k == a || k == b) k++; return k; }

        private void Pass(CommandBuffer cmd, int pass, RenderTexture src, int dst)
        {
            cmd.SetRenderTarget(_scratch[dst]);
            _props.SetTexture(SrcId, src);
            _props.SetVector(TargetSizeId, new Vector4(_scratch[dst].width, _scratch[dst].height, 0, 0));
            cmd.DrawProcedural(Matrix4x4.identity, _material, pass, MeshTopology.Triangles, 6, 1, _props);
        }

        /// <summary>A Gaussian blur of `src` into scratch `dst`: horizontal into a scratch texture that is neither `dst`
        /// nor `keep` (nor `src`), then vertical.</summary>
        private void Blur(CommandBuffer cmd, RenderTexture src, int dst, float sigma, int keep)
        {
            var mid = 0;
            while (mid == dst || mid == keep || _scratch[mid] == src) mid++;
            _props.SetVector(BlurDirId, new Vector4(1, 0, sigma, 0));
            Pass(cmd, BlurPass, src, mid);
            _props.SetVector(BlurDirId, new Vector4(0, 1, sigma, 0));
            Pass(cmd, BlurPass, _scratch[mid], dst);
        }

        public void Dispose()
        {
            foreach (var t in _full) if (t != null) UnityEngine.Object.Destroy(t);
            foreach (var t in _half) if (t != null) UnityEngine.Object.Destroy(t);
            if (_material != null) UnityEngine.Object.Destroy(_material);
        }
    }
}
