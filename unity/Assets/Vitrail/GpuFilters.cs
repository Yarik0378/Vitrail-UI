using System;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Rendering;

namespace VitrailLab
{
    /// <summary>
    /// CSS `filter` on the GPU: a group's texture run through its steps (blur, drop shadow, colour matrix) before it is
    /// composited. The passes cover only the group's bounds and ping-pong between three scratch textures.
    /// </summary>
    public sealed class GpuFilters : IDisposable
    {
        /// <summary>One step as the native library hands it over (gpu::FilterStep, 128 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        public unsafe struct Step
        {
            public uint Kind;
            public float Sigma, Dx, Dy;
            public Vector4 Color;
            public fixed float M[20];
            public Vector4 Bounds;
        }

        private const int MatrixPass = 0, BlurPass = 1, SeedPass = 2, UnderPass = 3, BackdropPass = 4;
        private static readonly int SrcId = Shader.PropertyToID("_Src");
        private static readonly int ShadowId = Shader.PropertyToID("_Shadow");
        private static readonly int BoundsId = Shader.PropertyToID("_Bounds");
        private static readonly int BlurDirId = Shader.PropertyToID("_BlurDir");
        private static readonly int OffsetId = Shader.PropertyToID("_Offset");
        private static readonly int ShadowColorId = Shader.PropertyToID("_ShadowColor");
        private static readonly int[] MatrixIds = { Shader.PropertyToID("_M0"), Shader.PropertyToID("_M1"), Shader.PropertyToID("_M2"), Shader.PropertyToID("_M3") };
        private static readonly int MOffId = Shader.PropertyToID("_MOff");

        private readonly Material _material;
        private readonly RenderTexture[] _scratch = new RenderTexture[3];
        private readonly MaterialPropertyBlock _props = new();
        public Step[] Steps = new Step[0];

        public GpuFilters()
        {
            var shader = Shader.Find("Hidden/Vitrail/Filter");
            if (shader != null) _material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
        }

        public bool Ready => _material != null;

        /// <summary>Reads the frame's filter steps from the library.</summary>
        public unsafe void Load(IntPtr ui)
        {
            var n = VitrailNative.vitrail_gpu_filters(ui, null, UIntPtr.Zero);
            if (n <= 0) return;
            if (Steps.Length < n) Steps = new Step[Mathf.NextPowerOfTwo(n)];
            fixed (Step* s = Steps) VitrailNative.vitrail_gpu_filters(ui, (byte*)s, (UIntPtr)(Steps.Length * sizeof(Step)));
        }

        /// <summary>Records the steps first..first+count on `source` into `cmd`; returns the texture holding the result.
        /// The caller's view-projection (device px, y down) must be set.</summary>
        public unsafe RenderTexture Run(CommandBuffer cmd, RenderTexture source, int first, int count, int width, int height, bool backdrop = false)
        {
            if (!Ready || count <= 0) return source;
            // As the game's painter: glass with a wide blur runs at a quarter (σ ≥ 6) or a half (σ ≥ 3) of the size.
            var down = backdrop && Steps[first].Kind == 1 && System.Environment.GetEnvironmentVariable("VITRAIL_FULL_GLASS") == null
                ? (Steps[first].Sigma >= 6 ? 4 : Steps[first].Sigma >= 3 ? 2 : 1) : 1;
            if (down > 1) { width = Mathf.Max(1, width / down); height = Mathf.Max(1, height / down); }
            for (var i = 0; i < 3; i++)
            {
                var t = _scratch[i];
                if (t != null && t.width == width && t.height == height) continue;
                if (t != null) t.Release();
                _scratch[i] = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 0) { sRGB = false }) { name = "Vitrail filter" };
                _scratch[i].Create();
            }
            var cur = -1; // -1: the source itself, else a scratch index
            if (backdrop)
            {
                // backdrop-filter's source: the page so far over the game's picture, within the steps' bounds.
                _props.Clear();
                _props.SetVector(BoundsId, Steps[first].Bounds);
                cmd.SetRenderTarget(_scratch[0]);
                _props.SetTexture(SrcId, source);
                _props.SetVector("_TargetSize", new Vector4(_scratch[0].width, _scratch[0].height, 0, 0));
                cmd.DrawProcedural(Matrix4x4.identity, _material, BackdropPass, MeshTopology.Triangles, 6, 1, _props);
                cur = 0;
            }
            RenderTexture Tex(int k) => k < 0 ? source : _scratch[k];
            int Free(int a, int b = -2) { for (var k = 0; k < 3; k++) if (k != a && k != b) return k; return 0; }
            void Pass(int pass, RenderTexture src, int dst)
            {
                cmd.SetRenderTarget(_scratch[dst]);
                _props.SetTexture(SrcId, src);
                _props.SetVector("_TargetSize", new Vector4(_scratch[dst].width, _scratch[dst].height, 0, 0));
                cmd.DrawProcedural(Matrix4x4.identity, _material, pass, MeshTopology.Triangles, 6, 1, _props);
            }
            void Blur(int from, int dst, float sigma, int keep)
            {
                // Horizontal into a scratch texture none of the others use, vertical into `dst`.
                var mid = 0;
                while (mid == from || mid == dst || mid == keep) mid++;
                _props.SetVector(BlurDirId, new Vector4(1, 0, sigma, 0));
                Pass(BlurPass, Tex(from), mid);
                _props.SetVector(BlurDirId, new Vector4(0, 1, sigma, 0));
                Pass(BlurPass, _scratch[mid], dst);
            }
            for (var s = first; s < first + count; s++)
            {
                var st = Steps[s];
                _props.Clear();
                _props.SetVector(BoundsId, st.Bounds);
                switch (st.Kind)
                {
                    case 1:
                    {
                        var dst = Free(cur);
                        Blur(cur, dst, st.Sigma, cur);
                        cur = dst;
                        break;
                    }
                    case 2:
                    {
                        // Seed (the alpha moved and tinted), blurred, then the input drawn over it.
                        var seed = Free(cur);
                        _props.SetVector(OffsetId, new Vector4(st.Dx, st.Dy, 0, 0));
                        _props.SetVector(ShadowColorId, st.Color);
                        Pass(SeedPass, Tex(cur), seed);
                        if (st.Sigma > 0.01f) Blur(seed, seed, st.Sigma, cur);
                        var dst = Free(cur, seed);
                        _props.SetTexture(ShadowId, _scratch[seed]);
                        Pass(UnderPass, Tex(cur), dst);
                        cur = dst;
                        break;
                    }
                    case 3:
                    {
                        for (var r = 0; r < 4; r++) _props.SetVector(MatrixIds[r], new Vector4(st.M[r * 5], st.M[r * 5 + 1], st.M[r * 5 + 2], st.M[r * 5 + 3]));
                        _props.SetVector(MOffId, new Vector4(st.M[4], st.M[9], st.M[14], st.M[19]));
                        var dst = Free(cur);
                        Pass(MatrixPass, Tex(cur), dst);
                        cur = dst;
                        break;
                    }
                }
            }
            return Tex(cur);
        }

        public void Dispose()
        {
            foreach (var t in _scratch) if (t != null) t.Release();
            if (_material != null) UnityEngine.Object.DestroyImmediate(_material);
        }
    }
}
