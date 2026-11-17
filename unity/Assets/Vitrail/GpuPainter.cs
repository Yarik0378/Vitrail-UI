using System;
using System.Runtime.InteropServices;
using Unity.Collections;
using UnityEngine;
using UnityEngine.Rendering;

namespace VitrailLab
{
    /// <summary>
    /// The GPU painter (step 1): the page's triangles from the native library drawn by Unity into a multisampled
    /// texture. The texture holds sRGB values (a UNORM target, no conversion), so premultiplied blending works on sRGB
    /// values as in a browser; the composite pass reads it like the CPU page.
    /// </summary>
    public sealed class GpuPainter : IDisposable
    {
        [StructLayout(LayoutKind.Sequential)]
        public struct Vertex { public float X, Y; public uint Color; public float U, V; public Vector4 P; public uint Meta, Meta2; public Vector4 Q; }
        [StructLayout(LayoutKind.Sequential)]
        public struct Batch { public uint Kind, First, Count, Texture, Stencil; }
        [StructLayout(LayoutKind.Sequential)]
        public struct Op { public uint Kind, Batch; public float Alpha; public uint Compose, Mix, Filter, FilterCount; }
        private Op[] _ops = new Op[0];
        private readonly System.Collections.Generic.Dictionary<int, Material> _depthMaterials = new();
        private readonly System.Collections.Generic.Dictionary<(int, uint), Material> _compositeMaterials = new();
        private readonly System.Collections.Generic.List<RenderTexture> _groups = new();
        private static readonly int StencilRefId = Shader.PropertyToID("_StencilRef");
        private static readonly int SrcBlendId = Shader.PropertyToID("_SrcBlend");
        private static readonly int DstBlendId = Shader.PropertyToID("_DstBlend");
        private static readonly int GroupId = Shader.PropertyToID("_Group");
        private static readonly int TargetSizeId = Shader.PropertyToID("_TargetSize");
        private const int ClipPass = 5, PushPass = 5, PopPass = 6, CompositePass = 7, BlendPass = 10;
        private static readonly int BackdropId = Shader.PropertyToID("_Backdrop");
        private static readonly int MixId = Shader.PropertyToID("_Mix");
        private RenderTexture _backdrop;

        /// <summary>The scratch copy of the target a blend mode reads.</summary>
        private RenderTexture Backdrop(int width, int height)
        {
            if (_backdrop != null && _backdrop.width == width && _backdrop.height == height) return _backdrop;
            if (_backdrop != null) _backdrop.Release();
            _backdrop = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 0) { sRGB = false }) { name = "Vitrail backdrop" };
            _backdrop.Create();
            return _backdrop;
        }

        /// <summary>The material for drawing at a clip depth (the stencil must equal it).</summary>
        private Material AtDepth(int depth)
        {
            if (_depthMaterials.TryGetValue(depth, out var m)) return m;
            m = new Material(_material) { hideFlags = HideFlags.HideAndDontSave };
            m.SetInt(StencilRefId, depth);
            return _depthMaterials[depth] = m;
        }

        /// <summary>The material compositing a group at a clip depth with a Porter-Duff operator (peniko Compose).</summary>
        private Material Composite(int depth, uint compose)
        {
            if (_compositeMaterials.TryGetValue((depth, compose), out var m)) return m;
            m = new Material(_material) { hideFlags = HideFlags.HideAndDontSave };
            m.SetInt(StencilRefId, depth);
            var (s, d) = compose switch
            {
                0 => (BlendMode.Zero, BlendMode.Zero),                         // clear
                1 => (BlendMode.One, BlendMode.Zero),                          // copy
                2 => (BlendMode.Zero, BlendMode.One),                          // dest
                4 => (BlendMode.OneMinusDstAlpha, BlendMode.One),              // dest over
                5 => (BlendMode.DstAlpha, BlendMode.Zero),                     // src in
                6 => (BlendMode.Zero, BlendMode.SrcAlpha),                     // dest in
                7 => (BlendMode.OneMinusDstAlpha, BlendMode.Zero),             // src out
                8 => (BlendMode.Zero, BlendMode.OneMinusSrcAlpha),             // dest out
                9 => (BlendMode.DstAlpha, BlendMode.OneMinusSrcAlpha),         // src atop
                10 => (BlendMode.OneMinusDstAlpha, BlendMode.SrcAlpha),        // dest atop
                11 => (BlendMode.OneMinusDstAlpha, BlendMode.OneMinusSrcAlpha),// xor
                12 or 13 => (BlendMode.One, BlendMode.One),                    // plus
                _ => (BlendMode.One, BlendMode.OneMinusSrcAlpha),              // src over
            };
            m.SetInt(SrcBlendId, (int)s);
            m.SetInt(DstBlendId, (int)d);
            return _compositeMaterials[(depth, compose)] = m;
        }

        private RenderTexture Group(int level, int width, int height, int msaa)
        {
            while (_groups.Count <= level) _groups.Add(null);
            var g = _groups[level];
            if (g == null || g.width != width || g.height != height)
            {
                if (g != null) g.Release();
                g = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 24) { sRGB = false, msaaSamples = msaa }) { name = "Vitrail GPU group" };
                g.Create();
                _groups[level] = g;
            }
            return g;
        }

        private readonly Material _material;
        private readonly Mesh _mesh = new() { name = "Vitrail GPU page" };
        private readonly CommandBuffer _cmd = new() { name = "Vitrail GPU page" };
        private Vertex[] _vertices = new Vertex[0];
        private uint[] _indices = new uint[0];
        private Batch[] _batches = new Batch[0];
        private Texture2D _ramps;
        private Texture2D _atlas;
        private byte[] _atlasData;
        private static readonly int AtlasId = Shader.PropertyToID("_Atlas");
        private const int Atlas = 2048;
        private uint[] _rampData = new uint[0];
        private readonly System.Collections.Generic.Dictionary<ulong, Texture2D> _images = new();
        private Texture[] _frameImages = new Texture[0];
        private bool[] _frameGame = new bool[0];
        private readonly System.Collections.Generic.Dictionary<uint, Texture> _gameTextures = new();
        private static readonly int ImageGameId = Shader.PropertyToID("_ImageGame"), ImageSrgbId = Shader.PropertyToID("_ImageSrgb");

        /// <summary>Puts a game texture where the page has <img src="texture:name"> (null removes it).</summary>
        public void SetTexture(string name, Texture texture)
        {
            var id = VitrailNative.vitrail_texture_id(name);
            if (texture == null) _gameTextures.Remove(id); else _gameTextures[id] = texture;
        }

        /// <summary>Whether the last frame shows game textures (it must be drawn again whenever they change).</summary>
        public bool HasGameTextures { get; private set; }
        private readonly MaterialPropertyBlock _props = new();
        private static readonly int RampsId = Shader.PropertyToID("_Ramps");
        private static readonly int RampRowsId = Shader.PropertyToID("_RampRows");
        private static readonly int ImageId = Shader.PropertyToID("_Image");
        public RenderTexture Target { get; private set; }
        private readonly GpuFilters _filters = new();
        private float[] _clips = new float[0];
        private static readonly int ClipRectId = Shader.PropertyToID("_ClipRect"), ClipR0Id = Shader.PropertyToID("_ClipR0"), ClipR1Id = Shader.PropertyToID("_ClipR1"), ClipOnId = Shader.PropertyToID("_ClipOn");

        private Texture2D _clipTable, _imageAtlas;
        private byte[] _imageAtlasData;

        public GpuPainter()
        {
            _material = new Material(Shader.Find("Hidden/Vitrail/Gpu")) { hideFlags = HideFlags.HideAndDontSave };
            _mesh.MarkDynamic();
        }

        /// <summary>Builds the page's triangles and draws them into Target (width×height device px).</summary>
        public unsafe void Paint(IntPtr ui, int width, int height, int msaa = 8)
        {
            var counts = stackalloc uint[7];
            VitrailNative.vitrail_gpu_build(ui, counts);
            int nv = (int)counts[0], ni = (int)counts[1], nb = (int)counts[2], nr = (int)counts[3], nimg = (int)counts[4], nops = (int)counts[5];
            // The glyph atlas, when it got new glyphs.
            if (counts[6] != 0)
            {
                if (_atlas == null)
                {
                    _atlas = new Texture2D(Atlas, Atlas, TextureFormat.R8, false, true) { name = "Vitrail glyphs", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                    _atlasData = new byte[Atlas * Atlas];
                }
                fixed (byte* a = _atlasData)
                    if (VitrailNative.vitrail_gpu_atlas(ui, a, (UIntPtr)_atlasData.Length) != 0) { _atlas.SetPixelData(_atlasData, 0); _atlas.Apply(false, false); }
            }
            if (_ops.Length < nops) _ops = new Op[Mathf.NextPowerOfTwo(nops)];
            fixed (Op* o = _ops) VitrailNative.vitrail_gpu_ops(ui, (byte*)o, (UIntPtr)(_ops.Length * sizeof(Op)));
            _filters.Load(ui);
            var nc = VitrailNative.vitrail_gpu_clips(ui, null, UIntPtr.Zero);
            if (nc > 0)
            {
                var rows = Mathf.NextPowerOfTwo(Mathf.Max(nc, 16));
                if (_clips.Length < rows * 16) _clips = new float[rows * 16];
                fixed (float* c = _clips) VitrailNative.vitrail_gpu_clips(ui, c, (UIntPtr)_clips.Length);
                if (_clipTable == null || _clipTable.height < rows)
                {
                    if (_clipTable != null) UnityEngine.Object.DestroyImmediate(_clipTable);
                    _clipTable = new Texture2D(4, rows, TextureFormat.RGBAFloat, false, true) { filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                }
                _clipTable.SetPixelData(_clips, 0);
                _clipTable.Apply(false, false);
            }
            if (_imageAtlasData == null) _imageAtlasData = new byte[2048 * 2048 * 4];
            int gotImages;
            fixed (byte* a = _imageAtlasData) gotImages = VitrailNative.vitrail_gpu_image_atlas(ui, a, (UIntPtr)_imageAtlasData.Length);
            if (gotImages != 0)
            {
                if (_imageAtlas == null) _imageAtlas = new Texture2D(2048, 2048, TextureFormat.RGBA32, false, true) { filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                _imageAtlas.SetPixelData(_imageAtlasData, 0);
                _imageAtlas.Apply(false, false);
            }
            // Gradient ramps: one row of 256 colours each, in one texture.
            if (nr > 0)
            {
                var rows = Mathf.NextPowerOfTwo(nr);
                if (_ramps == null || _ramps.height < rows)
                {
                    if (_ramps != null) UnityEngine.Object.DestroyImmediate(_ramps);
                    _ramps = new Texture2D(256, rows, TextureFormat.RGBA32, false, true) { name = "Vitrail ramps", filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                    _rampData = new uint[256 * rows];
                }
                fixed (uint* r = _rampData) VitrailNative.vitrail_gpu_ramps(ui, r, (UIntPtr)_rampData.Length);
                _ramps.SetPixelData(_rampData, 0);
                _ramps.Apply(false, false);
            }
            // Images, uploaded once each (the same pixels keep their id).
            if (_frameImages.Length < nimg) { _frameImages = new Texture[Mathf.NextPowerOfTwo(nimg)]; _frameGame = new bool[_frameImages.Length]; }
            HasGameTextures = false;
            var info = stackalloc uint[5];
            for (var k = 0; k < nimg; k++)
            {
                var game = VitrailNative.vitrail_gpu_image_texture(ui, (uint)k);
                _frameGame[k] = game != 0 && _gameTextures.TryGetValue(game, out var gameTex) && gameTex != null;
                if (_frameGame[k]) { _frameImages[k] = _gameTextures[game]; HasGameTextures = true; continue; }
                VitrailNative.vitrail_gpu_image(ui, (uint)k, info, null, UIntPtr.Zero);
                var id = info[0] | ((ulong)info[1] << 32);
                if (!_images.TryGetValue(id, out var tex))
                {
                    int iw = (int)info[2], ih = (int)info[3];
                    var px = new byte[iw * ih * 4];
                    fixed (byte* d = px) VitrailNative.vitrail_gpu_image(ui, (uint)k, info, d, (UIntPtr)px.Length);
                    tex = new Texture2D(iw, ih, TextureFormat.RGBA32, false, true) { name = "Vitrail image", filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                    tex.SetPixelData(px, 0);
                    tex.Apply(false, true);
                    _images[id] = tex;
                }
                _frameImages[k] = tex;
            }
            if (_vertices.Length < nv) _vertices = new Vertex[Mathf.NextPowerOfTwo(nv)];
            if (_indices.Length < ni) _indices = new uint[Mathf.NextPowerOfTwo(ni)];
            if (_batches.Length < nb) _batches = new Batch[Mathf.NextPowerOfTwo(nb)];
            fixed (Vertex* v = _vertices) fixed (uint* i = _indices) fixed (Batch* b = _batches)
                VitrailNative.vitrail_gpu_copy(ui, (byte*)v, (UIntPtr)(_vertices.Length * sizeof(Vertex)), i, (UIntPtr)_indices.Length, (byte*)b, (UIntPtr)(_batches.Length * sizeof(Batch)));

            if (Target == null || Target.width != width || Target.height != height)
            {
                if (Target != null) Target.Release();
                Target = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 24)
                    { sRGB = false, msaaSamples = msaa }) { name = "Vitrail GPU page" };
                Target.Create();
            }
            _mesh.Clear();
            _mesh.SetVertexBufferParams(nv,
                new VertexAttributeDescriptor(VertexAttribute.Position, VertexAttributeFormat.Float32, 2),
                new VertexAttributeDescriptor(VertexAttribute.Color, VertexAttributeFormat.UNorm8, 4),
                new VertexAttributeDescriptor(VertexAttribute.TexCoord0, VertexAttributeFormat.Float32, 2),
                new VertexAttributeDescriptor(VertexAttribute.TexCoord1, VertexAttributeFormat.Float32, 4),
                new VertexAttributeDescriptor(VertexAttribute.TexCoord2, VertexAttributeFormat.UInt32, 2),
                new VertexAttributeDescriptor(VertexAttribute.TexCoord3, VertexAttributeFormat.Float32, 4));
            _mesh.SetVertexBufferData(_vertices, 0, 0, nv, 0, MeshUpdateFlags.DontValidateIndices | MeshUpdateFlags.DontRecalculateBounds);
            _mesh.SetIndexBufferParams(ni, IndexFormat.UInt32);
            _mesh.SetIndexBufferData(_indices, 0, 0, ni, MeshUpdateFlags.DontValidateIndices | MeshUpdateFlags.DontRecalculateBounds);
            _mesh.subMeshCount = nb;
            for (var k = 0; k < nb; k++)
                _mesh.SetSubMesh(k, new SubMeshDescriptor((int)_batches[k].First, (int)_batches[k].Count), MeshUpdateFlags.DontValidateIndices | MeshUpdateFlags.DontRecalculateBounds);
            _mesh.bounds = new Bounds(new Vector3(width / 2f, height / 2f, 0), new Vector3(width, height, 10));

            _cmd.Clear();
            _cmd.SetRenderTarget(Target);
            _cmd.ClearRenderTarget(true, true, Color.clear);
            // Device px, y down, to clip space.
            _cmd.SetViewProjectionMatrices(Matrix4x4.identity, Matrix4x4.Ortho(0, width, height, 0, -1, 1));
            if (_ramps != null) { _cmd.SetGlobalTexture(RampsId, _ramps); _cmd.SetGlobalFloat(RampRowsId, _ramps.height); }
            _cmd.SetGlobalVector(TargetSizeId, new Vector4(width, height, 0, 0));
            _cmd.SetGlobalVector("_PageSize", new Vector4(width, height, 0, 0));
            if (_clipTable != null) _cmd.SetGlobalTexture("_ClipTable", _clipTable);
            // GPU-scrolled containers: their offsets now.
            var scroll = new Vector4[256];
            var states = new float[512];
            int ns;
            fixed (float* st = states) ns = VitrailNative.vitrail_scroll_states(ui, st, (UIntPtr)255);
            for (var k = 0; k < ns; k++) scroll[k + 1] = new Vector4(states[k * 2], states[k * 2 + 1], 0, 0);
            _cmd.SetGlobalVectorArray("_Scroll", scroll);
            if (_imageAtlas != null) _cmd.SetGlobalTexture("_ImageAtlas", _imageAtlas);
            if (_atlas != null) _cmd.SetGlobalTexture(AtlasId, _atlas);
            // The ops: draws at the current clip depth; clips push/pop the stencil; groups draw into their own texture,
            // then come back composited through their clip shape.
            var stack = new System.Collections.Generic.List<(RenderTexture Target, int Depth)>();
            var target = Target;
            var depth = 0;
            for (var k = 0; k < nops; k++)
            {
                var op = _ops[k];
                var b = (int)op.Batch;
                switch (op.Kind)
                {
                    case 0:
                        _props.Clear();
                        var tex = _batches[b].Texture;
                        if (tex > 0)
                        {
                            var image = _frameImages[tex - 1];
                            _props.SetTexture(ImageId, image);
                            _props.SetFloat(ImageGameId, _frameGame[tex - 1] ? 1 : 0);
                            _props.SetFloat(ImageSrgbId, _frameGame[tex - 1] && image.isDataSRGB ? 1 : 0);
                        }
                        _cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, (int)_batches[b].Kind, _props);
                        break;
                    case 1:
                        _cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, PushPass);
                        depth++;
                        break;
                    case 2:
                        _cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, PopPass);
                        depth--;
                        break;
                    case 3:
                        stack.Add((target, depth));
                        target = Group(stack.Count - 1, width, height, msaa);
                        depth = 0;
                        _cmd.SetRenderTarget(target);
                        _cmd.ClearRenderTarget(true, true, Color.clear);
                        break;
                    case 5:
                    {
                        // backdrop-filter: what lies under the element (the scene and the page so far), filtered,
                        // put back through the element's shape; the element then draws over it.
                        var under = _filters.Run(_cmd, target, (int)op.Filter, (int)op.FilterCount, width, height, true);
                        _cmd.SetRenderTarget(target);
                        _props.Clear();
                        _props.SetTexture(GroupId, under);
                        _props.SetFloat("_GroupAlpha", 1);
                        _cmd.DrawMesh(_mesh, Matrix4x4.identity, Composite(depth, 3), b, CompositePass, _props);
                        break;
                    }
                    case 4:
                        var group = _groups[stack.Count - 1];
                        if (op.FilterCount > 0) group = _filters.Run(_cmd, _groups[stack.Count - 1], (int)op.Filter, (int)op.FilterCount, width, height);
                        (target, depth) = stack[stack.Count - 1];
                        stack.RemoveAt(stack.Count - 1);
                        _props.Clear();
                        _props.SetTexture(GroupId, group);
                        _props.SetFloat("_GroupAlpha", op.Alpha);
                        if (op.Mix != 0 && op.Compose == 3)
                        {
                            // A blend mode reads what lies under the group: a copy of the target so far.
                            var backdrop = Backdrop(width, height);
                            _cmd.CopyTexture(target, backdrop);
                            _cmd.SetRenderTarget(target);
                            _props.SetTexture(BackdropId, backdrop);
                            _props.SetInt(MixId, (int)op.Mix);
                            _cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, BlendPass, _props);
                        }
                        else
                        {
                            _cmd.SetRenderTarget(target);
                            _cmd.DrawMesh(_mesh, Matrix4x4.identity, Composite(depth, op.Compose), b, CompositePass, _props);
                        }
                        break;
                }
            }
            Graphics.ExecuteCommandBuffer(_cmd);
        }

        public void Dispose()
        {
            if (Target != null) Target.Release();
            _filters.Dispose();
            if (_backdrop != null) _backdrop.Release();
            if (_ramps != null) UnityEngine.Object.DestroyImmediate(_ramps);
            if (_atlas != null) UnityEngine.Object.DestroyImmediate(_atlas);
            foreach (var i in _images.Values) UnityEngine.Object.DestroyImmediate(i);
            foreach (var g in _groups) if (g != null) g.Release();
            foreach (var m in _depthMaterials.Values) UnityEngine.Object.DestroyImmediate(m);
            foreach (var m in _compositeMaterials.Values) UnityEngine.Object.DestroyImmediate(m);
            UnityEngine.Object.DestroyImmediate(_mesh);
            UnityEngine.Object.DestroyImmediate(_material);
            _cmd.Release();
        }
    }
}
