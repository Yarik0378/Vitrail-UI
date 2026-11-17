using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using UnityEngine;
using UnityEngine.Rendering;

namespace Vitrail
{
    /// <summary>
    /// The GPU painter: the native library hands over the page as
    /// triangles in batches and steps (draw, push/pop a clip, push/pop a group), and this draws them with Unity into a
    /// multisampled texture of sRGB values (a UNORM target), so blending works on sRGB values as in a browser. Text comes
    /// from a glyph atlas the library rasterizes once per glyph; gradients from colour ramps; images are uploaded once
    /// each. Shaders: Resources/Vitrail/WebGpu.shader. Against the CPU painter the top bar differs by 0.5/255.
    /// </summary>
    public sealed class WebGpuPainter : IDisposable
    {
        [StructLayout(LayoutKind.Sequential)] private struct Vertex { public float X, Y; public uint Color; public float U, V; public Vector4 P; public uint Meta, Meta2; public Vector4 Q; }
        [StructLayout(LayoutKind.Sequential)] private struct Batch { public uint Kind, First, Count, Texture, Stencil; }
        [StructLayout(LayoutKind.Sequential)] private struct Op { public uint Kind, Batch; public float Alpha; public uint Compose, Mix, Filter, FilterCount; }

        private const int Atlas = 2048, PushPass = 5, PopPass = 6, CompositePass = 7, BlendPass = 10, Msaa = 1; // edges come antialiased in the triangles (vitrail_set_gpu 2)
        private static readonly int RampsId = Shader.PropertyToID("_Ramps"), RampRowsId = Shader.PropertyToID("_RampRows"), ImageId = Shader.PropertyToID("_Image");
        private static readonly int AtlasId = Shader.PropertyToID("_Atlas"), GroupId = Shader.PropertyToID("_Group"), GroupAlphaId = Shader.PropertyToID("_GroupAlpha");
        private static readonly int TargetSizeId = Shader.PropertyToID("_TargetSize"), StencilRefId = Shader.PropertyToID("_StencilRef");
        private static readonly int SrcBlendId = Shader.PropertyToID("_SrcBlend"), DstBlendId = Shader.PropertyToID("_DstBlend");
        private static readonly int BackdropId = Shader.PropertyToID("_Backdrop"), MixId = Shader.PropertyToID("_Mix");

        private readonly Material _material;
        private readonly Mesh _mesh = new() { name = "Web HUD page" };
        private readonly CommandBuffer _cmd = new() { name = "Web HUD page" };
        private readonly MaterialPropertyBlock _props = new();
        private readonly uint[] _counts = new uint[7];
        private readonly uint[] _info = new uint[5];
        private Vertex[] _vertices = new Vertex[0];
        private uint[] _indices = new uint[0];
        private Batch[] _batches = new Batch[0];
        private Op[] _ops = new Op[0];
        private uint[] _rampData = new uint[0];
        private Texture2D _ramps, _atlas;
        /// <summary>
        /// The bytes an atlas is read into on its way to its texture, shared by every painter: the library hands over the
        /// whole atlas each time it changed and the texture keeps its own copy, so nothing is kept here between paints
        /// (painting runs on the main thread only). Each painter held its own 4 MB glyph and 16 MB image buffers before,
        /// half a gigabyte of managed memory for a HUD of twenty-five pages, made again with every page (a new game).
        /// </summary>
        private static byte[] _scratch;
        private static byte[] Scratch(int bytes) => _scratch != null && _scratch.Length >= bytes ? _scratch : _scratch = new byte[bytes];
        private Texture[] _frameImages = new Texture[0];
        private bool[] _frameGame = new bool[0];
        private readonly Dictionary<uint, Texture> _gameTextures = new();
        private static readonly int ImageGameId = Shader.PropertyToID("_ImageGame"), ImageSrgbId = Shader.PropertyToID("_ImageSrgb");

        /// <summary>Puts a game texture (a render texture, a portrait) where the page has &lt;img src="texture:name"&gt;;
        /// null takes it away. The next Paint uses it.</summary>
        public void SetTexture(string name, Texture texture)
        {
            var id = WebNative.vitrail_texture_id(name);
            if (texture == null) _gameTextures.Remove(id); else _gameTextures[id] = texture;
        }

        /// <summary>Whether the last frame shows game textures: then it is drawn again every frame (Redraw), as they may
        /// change any time.</summary>
        public bool HasGameTextures { get; private set; }

        /// <summary>Draws the last frame again without reading it from the library (for live game textures).</summary>
        public void Redraw() { if (!Assembled) Graphics.ExecuteCommandBuffer(_cmd); }

        /// <summary>Whether the last frame has backdrop-filter: then the panel records it every frame inside the
        /// camera's rendering (Record), since what lies under it changes all the time.</summary>
        public bool HasBackdrop { get; private set; }
        private int _width, _height, _nops;
        private readonly Dictionary<ulong, Texture2D> _images = new();
        private readonly HashSet<ulong> _usedImages = new();
        private readonly List<ulong> _staleImages = new();
        private readonly Dictionary<int, Material> _depth = new();
        private readonly Dictionary<(int, uint), Material> _composite = new();
        private readonly List<RenderTexture> _groups = new();
        private readonly WebFilters _filters = new();
        private RenderTexture _backdrop;

        /// <summary>The page as painted (sRGB values, premultiplied; resolved when sampled).</summary>
        public RenderTexture Target { get; private set; }
        public bool Ready => _material != null;
        /// <summary>Why the GPU painter can't run here, or null.</summary>
        public string Problem { get; private set; }

        public WebGpuPainter()
        {
            var shader = UnityEngine.Resources.Load<Shader>("Vitrail/WebGpu");
            if (shader == null) Problem = "the shader Resources/Vitrail/WebGpu was not found";
            else if (!shader.isSupported) Problem = "the shader Hidden/Vitrail/WebGpu isn't supported on " + SystemInfo.graphicsDeviceType;
            else if (SystemInfo.copyTextureSupport == CopyTextureSupport.None) Problem = "the GPU can't copy textures";
            else _material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
            _mesh.MarkDynamic();
        }

        private static void Pin<T>(T[] array, Action<IntPtr> use)
        {
            var h = GCHandle.Alloc(array, GCHandleType.Pinned);
            try { use(h.AddrOfPinnedObject()); } finally { h.Free(); }
        }

        /// <summary>Reads the frame the library just built and draws it into Target (width × height device px).</summary>
        public void Paint(IntPtr ui, int width, int height)
        {
            if (_material == null) return;
            WebNative.vitrail_gpu_counts(ui, _counts);
            int nv = (int)_counts[0], ni = (int)_counts[1], nb = (int)_counts[2], nr = (int)_counts[3], nimg = (int)_counts[4], nops = (int)_counts[5];
            // The vertex mirror keeps what earlier frames wrote: only the spans the library wrote are copied into it.
            if (_vertices.Length < nv) Array.Resize(ref _vertices, Mathf.NextPowerOfTwo(nv));
            if (_indices.Length < ni) _indices = new uint[Mathf.NextPowerOfTwo(ni)];
            if (_batches.Length < nb) _batches = new Batch[Mathf.NextPowerOfTwo(nb)];
            if (_ops.Length < nops) _ops = new Op[Mathf.NextPowerOfTwo(nops)];
            var vsize = Marshal.SizeOf<Vertex>();
            Pin(_vertices, v => Pin(_indices, i => Pin(_batches, b =>
                WebNative.vitrail_gpu_copy(ui, IntPtr.Zero, UIntPtr.Zero, i, (UIntPtr)_indices.Length, b, (UIntPtr)(_batches.Length * Marshal.SizeOf<Batch>())))));
            var nspans = WebNative.vitrail_gpu_vertex_spans(ui, _spans, (UIntPtr)(_spans.Length / 2));
            Pin(_vertices, v => { for (var k = 0; k < nspans; k++) WebNative.vitrail_gpu_vertex_range(ui, v, (UIntPtr)(_vertices.Length * vsize), _spans[k * 2], _spans[k * 2 + 1]); });
            Pin(_ops, o => WebNative.vitrail_gpu_ops(ui, o, (UIntPtr)(_ops.Length * Marshal.SizeOf<Op>())));
            _filters.Load(ui);
            if (_counts[6] != 0)
            {
                if (_atlas == null) _atlas = new Texture2D(Atlas, Atlas, TextureFormat.R8, false, true) { name = "Web HUD glyphs", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                var glyphs = Scratch(Atlas * Atlas);
                var got = 0;
                Pin(glyphs, a => got = WebNative.vitrail_gpu_atlas(ui, a, (UIntPtr)(Atlas * Atlas)));
                if (got != 0) { _atlas.SetPixelData(glyphs, 0, 0); _atlas.Apply(false, false); }
            }
            // The image atlas: small images packed by the library, uploaded when it got new ones.
            var images = Scratch(ImageAtlas * ImageAtlas * 4);
            var gotImages = 0;
            Pin(images, a => gotImages = WebNative.vitrail_gpu_image_atlas(ui, a, (UIntPtr)(ImageAtlas * ImageAtlas * 4)));
            if (gotImages != 0)
            {
                if (_imageAtlas == null) _imageAtlas = new Texture2D(ImageAtlas, ImageAtlas, TextureFormat.RGBA32, false, true) { name = "Web HUD image atlas", filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                _imageAtlas.SetPixelData(images, 0, 0);
                _imageAtlas.Apply(false, false);
            }
            if (nr > 0)
            {
                var rows = Mathf.NextPowerOfTwo(nr);
                if (_ramps == null || _ramps.height < rows)
                {
                    if (_ramps != null) UnityEngine.Object.Destroy(_ramps);
                    _ramps = new Texture2D(256, rows, TextureFormat.RGBA32, false, true) { name = "Web HUD ramps", filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                    _rampData = new uint[256 * rows];
                }
                Pin(_rampData, r => WebNative.vitrail_gpu_ramps(ui, r, (UIntPtr)_rampData.Length));
                _ramps.SetPixelData(_rampData, 0);
                _ramps.Apply(false, false);
            }
            if (_frameImages.Length < nimg) { _frameImages = new Texture[Mathf.NextPowerOfTwo(nimg)]; _frameGame = new bool[_frameImages.Length]; }
            HasGameTextures = false;
            for (var k = 0; k < nimg; k++)
            {
                // A game texture stands in for its placeholder; an unknown one leaves the (transparent) placeholder.
                var game = WebNative.vitrail_gpu_image_texture(ui, (uint)k);
                _frameGame[k] = game != 0 && _gameTextures.TryGetValue(game, out var gameTex) && gameTex != null;
                if (_frameGame[k]) { _frameImages[k] = _gameTextures[game]; HasGameTextures = true; continue; }
                WebNative.vitrail_gpu_image(ui, (uint)k, _info, IntPtr.Zero, UIntPtr.Zero);
                var id = _info[0] | ((ulong)_info[1] << 32);
                if (!_images.TryGetValue(id, out var tex))
                {
                    int iw = (int)_info[2], ih = (int)_info[3];
                    var px = Scratch(iw * ih * 4);
                    var index = (uint)k;
                    Pin(px, d => WebNative.vitrail_gpu_image(ui, index, _info, d, (UIntPtr)(iw * ih * 4)));
                    tex = new Texture2D(iw, ih, TextureFormat.RGBA32, false, true) { name = "Web HUD image", filterMode = FilterMode.Bilinear, wrapMode = TextureWrapMode.Clamp };
                    tex.SetPixelData(px, 0, 0);
                    tex.Apply(false, true);
                    _images[id] = tex;
                }
                _frameImages[k] = tex;
                _usedImages.Add(id);
            }
            // Images this frame no longer draws go with their textures. Keyed by the image's data, which a reloaded page
            // (a resize, a new size of its area, the HUD's scale) or a rebuilt <img> decodes anew: kept until the page was
            // destroyed, they piled up with every reload.
            if (_images.Count > _usedImages.Count)
            {
                _staleImages.Clear();
                foreach (var kv in _images) if (!_usedImages.Contains(kv.Key)) _staleImages.Add(kv.Key);
                foreach (var stale in _staleImages)
                {
                    UnityEngine.Object.Destroy(_images[stale]);
                    _images.Remove(stale);
                }
            }
            _usedImages.Clear();

            if (Target == null || Target.width != width || Target.height != height)
            {
                if (Target != null) UnityEngine.Object.Destroy(Target);
                Target = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 24) { sRGB = false, msaaSamples = Msaa }) { name = "Web HUD page (GPU)" };
                Target.Create();
                _targetDrawn = false;
            }
            // The mesh keeps its buffers (grown in powers of two): only what the library says changed is uploaded.
            const MeshUpdateFlags quiet = MeshUpdateFlags.DontValidateIndices | MeshUpdateFlags.DontRecalculateBounds | MeshUpdateFlags.DontNotifyMeshUsers;
            WebNative.vitrail_gpu_changed(ui, _changed);
            uint i0 = _changed[2], i1 = _changed[3];
            var allVertices = false;
            if (nv > _vcap)
            {
                _vcap = Mathf.NextPowerOfTwo(Mathf.Max(nv, 1024));
                _mesh.SetVertexBufferParams(_vcap,
                    new VertexAttributeDescriptor(VertexAttribute.Position, VertexAttributeFormat.Float32, 2),
                    new VertexAttributeDescriptor(VertexAttribute.Color, VertexAttributeFormat.UNorm8, 4),
                    new VertexAttributeDescriptor(VertexAttribute.TexCoord0, VertexAttributeFormat.Float32, 2),
                    new VertexAttributeDescriptor(VertexAttribute.TexCoord1, VertexAttributeFormat.Float32, 4),
                    new VertexAttributeDescriptor(VertexAttribute.TexCoord2, VertexAttributeFormat.UInt32, 2),
                    new VertexAttributeDescriptor(VertexAttribute.TexCoord3, VertexAttributeFormat.Float32, 4));
                allVertices = true; // a new buffer: everything goes up
            }
            if (ni > _icap)
            {
                _icap = Mathf.NextPowerOfTwo(Mathf.Max(ni, 4096));
                _mesh.SetIndexBufferParams(_icap, IndexFormat.UInt32);
                (i0, i1) = (0, (uint)ni);
            }
            i1 = Math.Min(i1, (uint)ni);
            var uploaded = 0;
            if (allVertices) { _mesh.SetVertexBufferData(_vertices, 0, 0, nv, 0, quiet); uploaded = nv; }
            else
                for (var k = 0; k < nspans; k++)
                {
                    int a = (int)_spans[k * 2], e = Math.Min((int)_spans[k * 2 + 1], nv);
                    if (e > a) { _mesh.SetVertexBufferData(_vertices, a, a, e - a, 0, quiet); uploaded += e - a; }
                }
            if (i1 > i0) _mesh.SetIndexBufferData(_indices, (int)i0, (int)i0, (int)(i1 - i0), quiet);
            LastUpload = uploaded * Marshal.SizeOf<Vertex>() + (int)(i1 - i0) * 4;
            if (_subMeshes.Length < nb) _subMeshes = new SubMeshDescriptor[Mathf.NextPowerOfTwo(nb)];
            for (var k = 0; k < nb; k++) _subMeshes[k] = new SubMeshDescriptor((int)_batches[k].First, (int)_batches[k].Count);
            _mesh.SetSubMeshes(_subMeshes, 0, nb, quiet);
            _mesh.bounds = new Bounds(new Vector3(width / 2f, height / 2f, 0), new Vector3(width, height, 10));

            _width = width; _height = height; _nops = nops;
            _ui = ui;
            var nl = WebNative.vitrail_gpu_layers(ui, IntPtr.Zero, UIntPtr.Zero);
            if (nl > 0)
            {
                if (_layerInfo.Length < nl) { _layerInfo = new CompLayer[Mathf.NextPowerOfTwo(nl)]; _states = new float[_layerInfo.Length * 7]; }
                Pin(_layerInfo, l => WebNative.vitrail_gpu_layers(ui, l, (UIntPtr)(_layerInfo.Length * Marshal.SizeOf<CompLayer>())));
            }
            var nc = WebNative.vitrail_gpu_clips(ui, null, UIntPtr.Zero);
            if (nc > 0)
            {
                // The table the shaders read: four RGBA float texels a clip (shape, radii, scroll space), one clip a row.
                var rows = Mathf.NextPowerOfTwo(Mathf.Max(nc, 16));
                if (_clips.Length < rows * 16) _clips = new float[rows * 16];
                WebNative.vitrail_gpu_clips(ui, _clips, (UIntPtr)_clips.Length);
                if (_clipTable == null || _clipTable.height < rows)
                {
                    if (_clipTable != null) UnityEngine.Object.Destroy(_clipTable);
                    _clipTable = new Texture2D(4, rows, TextureFormat.RGBAFloat, false, true) { name = "Web HUD clips", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                }
                _clipTable.SetPixelData(_clips, 0);
                _clipTable.Apply(false, false);
            }
            ReadScrolls(ui);
            Split();
            _cmd.Clear();
            Begin(_cmd);
            // What changed on the page: kept textures are redrawn only inside these rectangles (when the frame's
            // pieces are the same as last time; otherwise whole).
            var nd = WebNative.vitrail_gpu_dirty(ui, _dirty, (UIntPtr)4);
            _dirtyCountNow = nd;
            var same = _pieces.Count == _lastPieces.Count && _assembledBefore == Assembled;
            for (var k = 0; same && k < _pieces.Count; k++) same = _pieces[k] == _lastPieces[k];
            if (!Assembled)
            {
                var whole = new RectInt(0, 0, width, height);
                Redraw(_cmd, Target, whole, 0, nops, same && _targetDrawn);
                _targetDrawn = true;
            }
            else
            {
                // What doesn't change from frame to frame is drawn now, each piece into its own texture: the still
                // parts of the page and the composited layers (as painted). Record lays them together every frame.
                // Each texture is only as big as what the piece draws.
                var li = 0;
                var lastRects = new List<RectInt>(_pieceRects);
                _pieceRects.Clear();
                foreach (var piece in _pieces)
                {
                    if (piece.Kind == Glass) continue;
                    var r = PieceRect(piece.From, piece.To);
                    var keep = same && li < lastRects.Count && lastRects[li] == r && li < _layers.Count && _layers[li] != null
                        && _layers[li].width == r.width && _layers[li].height == r.height;
                    _pieceRects.Add(r);
                    var layer = Layer(li++, r.width, r.height);
                    Redraw(_cmd, layer, r, piece.From, piece.To, keep);
                }
                _targetDrawn = false;
            }
            _lastPieces.Clear();
            _lastPieces.AddRange(_pieces);
            _assembledBefore = Assembled;
            Graphics.ExecuteCommandBuffer(_cmd);
            // Without glass the page is laid together here (and again each frame by the panel while layers move).
            Assemble();
        }

        /// <summary>A composited layer as the library hands it over (gpu::CompLayer, 80 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct CompLayer
        {
            public ulong Id;
            public uint Inline, Pad;
            public float W0, W1, W2, W3, W4, W5, L0, L1, L2, L3, L4, L5;
            public Vector4 Bounds;
        }

        private const int Still = 0, Glass = 1, Moving = 2;
        /// <summary>The frame cut at its top level into pieces: still ones (drawn once per change), glass (drawn every
        /// frame, as the scene under it moves) and composited layers (drawn once, moved and faded every frame).</summary>
        private readonly List<(int From, int To, int Kind, int Layer)> _pieces = new();
        private readonly List<RenderTexture> _layers = new();
        private CompLayer[] _layerInfo = new CompLayer[0];
        private float[] _states = new float[0];
        private float[] _clips = new float[0];
        private readonly uint[] _changed = new uint[4];
        private readonly uint[] _spans = new uint[64];
        private int _vcap, _icap;
        private SubMeshDescriptor[] _subMeshes = new SubMeshDescriptor[0];
        /// <summary>Bytes of vertices and indices the last paint uploaded.</summary>
        public int LastUpload { get; private set; }
        private static readonly int ClipTableId = Shader.PropertyToID("_ClipTable"), ImageAtlasId = Shader.PropertyToID("_ImageAtlas");
        private const int ImageAtlas = 2048;
        private Texture2D _clipTable, _imageAtlas;

        private IntPtr _ui;

        /// <summary>The document the last frame came from is about to be destroyed (a reload, a rebuilt library): nothing
        /// is drawn and the library isn't asked until the next Paint. Record runs inside the camera's frame and asked the
        /// destroyed document for its layer states, which crashed the editor.</summary>
        public void Forget()
        {
            _ui = IntPtr.Zero;
            _nops = 0;
        }
        private readonly CommandBuffer _frameCmd = new() { name = "Web HUD layers" };

        /// <summary>Whether the frame is laid together every frame from pieces (glass or composited layers).</summary>
        public bool Assembled => HasBackdrop || HasLayers;
        /// <summary>Whether the frame has composited layers (moved by the library's layer states every frame).</summary>
        public bool HasLayers { get; private set; }

        private void Split()
        {
            // Units: a composited layer's steps (6..7), and the outermost group around each backdrop-filter (or the
            // step itself when only stencil clips enclose it). Cuts fall where only stencil clips are open, so each
            // piece can re-push them.
            _pieces.Clear();
            HasBackdrop = HasLayers = false;
            var stack = new List<int>();
            int segStart = 0, unitStart = -1, unitDepth = -1, layerStart = -1, layerIndex = -1;
            var layerGlass = false;
            for (var k = 0; k < _nops; k++)
            {
                var kind = _ops[k].Kind;
                if (layerStart >= 0)
                {
                    // Inside a composited layer: glass in it makes the whole layer live.
                    if (kind == 5) layerGlass = true;
                    if (kind == 7)
                    {
                        _pieces.Add((layerStart, k + 1, layerGlass ? Glass : Moving, layerIndex));
                        if (layerGlass) HasBackdrop = true; else HasLayers = true;
                        segStart = k + 1;
                        layerStart = -1;
                    }
                    continue;
                }
                if (kind == 6 && unitStart < 0)
                {
                    if (k > segStart) _pieces.Add((segStart, k, Still, -1));
                    layerStart = k;
                    layerIndex = (int)_ops[k].Batch;
                    layerGlass = false;
                    continue;
                }
                if (kind == 5 && unitStart < 0)
                {
                    var outer = stack.FindIndex(i => _ops[i].Kind == 3);
                    if (outer < 0)
                    {
                        if (k > segStart) _pieces.Add((segStart, k, Still, -1));
                        _pieces.Add((k, k + 1, Glass, -1));
                        segStart = k + 1;
                        HasBackdrop = true;
                        continue;
                    }
                    unitStart = stack[outer];
                    unitDepth = outer;
                }
                if (kind == 1 || kind == 3) stack.Add(k);
                else if ((kind == 2 || kind == 4) && stack.Count > 0)
                {
                    stack.RemoveAt(stack.Count - 1);
                    if (unitStart >= 0 && stack.Count == unitDepth)
                    {
                        if (unitStart > segStart) _pieces.Add((segStart, unitStart, Still, -1));
                        _pieces.Add((unitStart, k + 1, Glass, -1));
                        segStart = k + 1;
                        unitStart = -1;
                        HasBackdrop = true;
                    }
                }
            }
            if (_nops > segStart) _pieces.Add((segStart, _nops, Still, -1));
        }

        /// <summary>Lays the frame together again with the layers where the library now has them (a page with
        /// composited layers but no glass, every frame while it animates).</summary>
        public void Assemble()
        {
            if (!Assembled || HasBackdrop) return;
            _frameCmd.Clear();
            Record(_frameCmd);
            Graphics.ExecuteCommandBuffer(_frameCmd);
        }

        /// <summary>How many steps are drawn every frame (for the stats).</summary>
        public int LiveOps { get { var n = 0; if (Assembled) foreach (var p in _pieces) if (p.Kind != Still) n += p.Kind == Glass ? p.To - p.From : 1; return n; } }

        /// <summary>A scroll container whose content the frame holds unscrolled (gpu::ScrollInfo, 40 bytes).</summary>
        [StructLayout(LayoutKind.Sequential)]
        private struct ScrollInfo { public ulong Id; public uint Parent, Pad; public float OffsetX, OffsetY, X0, Y0, X1, Y1; }

        private ScrollInfo[] _scrollInfo = new ScrollInfo[0];
        private int _nscrolls;
        private readonly Vector4[] _scroll = new Vector4[256];
        private readonly float[] _scrollStates = new float[512];
        private static readonly int ScrollId = Shader.PropertyToID("_Scroll");

        /// <summary>The frame's GPU-scrolled containers and their offsets now (what the vertex shader moves them by).</summary>
        private void ReadScrolls(IntPtr ui)
        {
            _nscrolls = WebNative.vitrail_gpu_scrolls(ui, IntPtr.Zero, UIntPtr.Zero);
            if (_nscrolls > 0)
            {
                if (_scrollInfo.Length < _nscrolls) _scrollInfo = new ScrollInfo[Mathf.NextPowerOfTwo(_nscrolls)];
                Pin(_scrollInfo, p => WebNative.vitrail_gpu_scrolls(ui, p, (UIntPtr)(_scrollInfo.Length * Marshal.SizeOf<ScrollInfo>())));
            }
            var n = WebNative.vitrail_scroll_states(ui, _scrollStates, (UIntPtr)255);
            for (var i = 0; i < n; i++) _scroll[i + 1] = new Vector4(_scrollStates[i * 2], _scrollStates[i * 2 + 1], 0, 0);
        }

        /// <summary>Whether the frame scrolls content on the GPU.</summary>
        public bool HasScrolls => _nscrolls > 0;

        /// <summary>How many GPU-scrolled containers the frame has (`ScrollBox` 0..count-1).</summary>
        public int ScrollCount => _nscrolls;

        /// <summary>Where scroll container `i` is on the page now (device px, x0 y0 x1 y1): its box as painted, moved by
        /// the containers it sits in.</summary>
        public Vector4 ScrollBox(int i)
        {
            var sc = _scrollInfo[i];
            var p = sc.Parent > 0 ? _scroll[sc.Parent] : Vector4.zero;
            return new Vector4(sc.X0 - p.x, sc.Y0 - p.y, sc.X1 - p.x, sc.Y1 - p.y);
        }

        public ulong ScrollNode(int i) => _scrollInfo[i].Id;

        /// <summary>After the library scrolled on the GPU (vitrail_scroll returned 2): the new offsets, and the kept textures
        /// redrawn inside the scrolled containers (no paint, no new vertices).</summary>
        public void Rescroll(IntPtr ui)
        {
            if (_nops == 0 || _nscrolls == 0) return;
            ReadScrolls(ui);
            _dirtyCountNow = 0;
            for (var i = 0; i < _nscrolls && _dirtyCountNow < 4; i++)
            {
                var sc = _scrollInfo[i];
                if (sc.Parent != 0) continue; // inside an outer container: its box covers it
                var o = _dirtyCountNow++ * 4;
                _dirty[o] = sc.X0; _dirty[o + 1] = sc.Y0; _dirty[o + 2] = sc.X1; _dirty[o + 3] = sc.Y1;
            }
            _cmd.Clear();
            if (!Assembled) Redraw(_cmd, Target, new RectInt(0, 0, _width, _height), 0, _nops, _targetDrawn);
            else
            {
                var li = 0;
                foreach (var piece in _pieces)
                {
                    if (piece.Kind == Glass) continue;
                    var r = _pieceRects[li];
                    Redraw(_cmd, _layers[li++], r, piece.From, piece.To, true);
                }
            }
            Graphics.ExecuteCommandBuffer(_cmd);
            Assemble();
        }

        private readonly float[] _dirty = new float[16];
        private readonly List<(int From, int To, int Kind, int Layer)> _lastPieces = new();
        private bool _assembledBefore, _targetDrawn;

        /// <summary>Draws steps from..to into `into`, which holds page rectangle `r`: whole, or, when `keep` (it holds
        /// the last frame's drawing of the same steps), only inside the changed rectangles, through a scissor.</summary>
        private void Redraw(CommandBuffer cmd, RenderTexture into, RectInt r, int from, int to, bool keep)
        {
            Begin(cmd, r.x, r.y, r.width, r.height);
            cmd.SetRenderTarget(into);
            if (!keep)
            {
                cmd.ClearRenderTarget(true, true, Color.clear);
                RecordRange(cmd, into, from, to, r.width, r.height);
                return;
            }
            for (var k = 0; k < _dirtyCountNow; k++)
            {
                int x0 = Mathf.Max(r.xMin, (int)_dirty[k * 4]), y0 = Mathf.Max(r.yMin, (int)_dirty[k * 4 + 1]);
                int x1 = Mathf.Min(r.xMax, Mathf.CeilToInt(_dirty[k * 4 + 2])), y1 = Mathf.Min(r.yMax, Mathf.CeilToInt(_dirty[k * 4 + 3]));
                if (x1 <= x0 || y1 <= y0) continue;
                // Scissor in the texture's pixels, origin bottom left.
                _scissor = new Rect(x0 - r.x, r.yMax - y1, x1 - x0, y1 - y0);
                cmd.EnableScissorRect(_scissor.Value);
                _filters.Clear(cmd, new Vector4(x0, y0, x1, y1));
                RecordRange(cmd, into, from, to, r.width, r.height);
                _scissor = null;
                cmd.DisableScissorRect();
                cmd.SetRenderTarget(into);
            }
        }
        private int _dirtyCountNow;
        /// <summary>While only the changed rectangles are drawn again: the scissor that keeps drawing inside the one being
        /// drawn. Binding another target drops it, so it is set again whenever drawing comes back to the kept texture;
        /// groups and filters draw whole (a blur must not read stale pixels round the rectangle).</summary>
        private Rect? _scissor;

        /// <summary>Draws into `target` from now on: the kept texture under the scissor, anything else whole.</summary>
        private void Bind(CommandBuffer cmd, RenderTexture target, RenderTexture into)
        {
            cmd.DisableScissorRect();
            cmd.SetRenderTarget(target);
            if (_scissor is { } s && target == into) cmd.EnableScissorRect(s);
        }

        private RenderTexture Layer(int index, int width, int height)
        {
            while (_layers.Count <= index) _layers.Add(null);
            var l = _layers[index];
            if (l != null && l.width == width && l.height == height) return l;
            if (l != null) UnityEngine.Object.Destroy(l);
            l = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 24) { sRGB = false }) { name = "Web HUD still layer" };
            l.Create();
            return _layers[index] = l;
        }

        private void Begin(CommandBuffer cmd) => Begin(cmd, 0, 0, _width, _height);

        /// <summary>Draws the page's rectangle (x, y, w, h device px) into a w×h target.</summary>
        private void Begin(CommandBuffer cmd, int x, int y, int w, int h)
        {
            cmd.SetViewProjectionMatrices(Matrix4x4.identity, Matrix4x4.Ortho(x, x + w, y + h, y, -1, 1));
            if (_ramps != null) { cmd.SetGlobalTexture(RampsId, _ramps); cmd.SetGlobalFloat(RampRowsId, _ramps.height); }
            if (_atlas != null) cmd.SetGlobalTexture(AtlasId, _atlas);
            cmd.SetGlobalVector(TargetSizeId, new Vector4(w, h, 0, 0));
            cmd.SetGlobalVector(PageSizeId, new Vector4(_width, _height, 0, 0));
            if (_clipTable != null) cmd.SetGlobalTexture(ClipTableId, _clipTable);
            cmd.SetGlobalVectorArray(ScrollId, _scroll);
            if (_imageAtlas != null) cmd.SetGlobalTexture(ImageAtlasId, _imageAtlas);
        }

        private static readonly int PageSizeId = Shader.PropertyToID("_PageSize");

        private readonly List<RectInt> _pieceRects = new();

        /// <summary>The page rectangle steps from..to draw in (from their vertices), whole pixels, within the page.</summary>
        private RectInt PieceRect(int from, int to)
        {
            float x0 = float.MaxValue, y0 = float.MaxValue, x1 = float.MinValue, y1 = float.MinValue;
            for (var k = from; k < to; k++)
            {
                var kind = _ops[k].Kind;
                if (kind == 2 || kind == 6 || kind == 7) continue;
                var bt = _batches[_ops[k].Batch];
                for (var i = bt.First; i < bt.First + bt.Count; i++)
                {
                    var v = _vertices[_indices[i]];
                    var sid = (int)(v.Meta >> 24);
                    if (sid > 0 && sid <= _nscrolls)
                    {
                        // Scrolled content shows only inside its container: the container's box stands for it.
                        var sc = _scrollInfo[sid - 1];
                        if (sc.X0 < x0) x0 = sc.X0; if (sc.X1 > x1) x1 = sc.X1;
                        if (sc.Y0 < y0) y0 = sc.Y0; if (sc.Y1 > y1) y1 = sc.Y1;
                        continue;
                    }
                    if (v.X < x0) x0 = v.X; if (v.X > x1) x1 = v.X;
                    if (v.Y < y0) y0 = v.Y; if (v.Y > y1) y1 = v.Y;
                }
            }
            if (x0 > x1) return new RectInt(0, 0, 1, 1);
            int ix0 = Mathf.Clamp(Mathf.FloorToInt(x0) - 2, 0, _width - 1), iy0 = Mathf.Clamp(Mathf.FloorToInt(y0) - 2, 0, _height - 1);
            int ix1 = Mathf.Clamp(Mathf.CeilToInt(x1) + 2, ix0 + 1, _width), iy1 = Mathf.Clamp(Mathf.CeilToInt(y1) + 2, iy0 + 1, _height);
            return new RectInt(ix0, iy0, ix1 - ix0, iy1 - iy0);
        }

        /// <summary>Records the last frame's drawing into `cmd` (into Target; the caller's view-projection is replaced).
        /// A page with backdrop-filter is recorded this way inside the camera's frame, after the scene is copied
        /// (_VitrailScene), every frame; others once per change by Paint.</summary>
        public void Record(CommandBuffer cmd)
        {
            if (Target == null || _nops == 0) return;
            Begin(cmd);
            cmd.SetRenderTarget(Target);
            cmd.ClearRenderTarget(true, true, Color.clear);
            if (HasLayers && _ui != IntPtr.Zero) WebNative.vitrail_layer_states(_ui, _states, (UIntPtr)_states.Length);
            var li = 0;
            foreach (var piece in _pieces)
            {
                if (piece.Kind == Glass) { RecordRange(cmd, Target, piece.From, piece.To, _width, _height); continue; }
                var rect = _pieceRects[li];
                var tex = _layers[li++];
                if (piece.Kind == Still) _filters.Over(cmd, tex, Target, rect);
                else
                {
                    var st = piece.Layer * 7;
                    _filters.Layer(cmd, tex, Target, rect,
                        new Vector4(_states[st], _states[st + 1], _states[st + 2], _states[st + 3]), new Vector3(_states[st + 4], _states[st + 5], _states[st + 6]));
                }
                cmd.SetRenderTarget(Target);
            }
        }

        /// <summary>Records steps from..to into `into` (already bound and cleared by the caller).</summary>
        private void RecordRange(CommandBuffer cmd, RenderTexture into, int from, int to, int width, int height)
        {
            var stack = new List<(RenderTexture Target, int Depth)>();
            var target = into;
            var depth = 0;
            // The stencil clips open where the piece starts are pushed again first (a piece only starts where nothing
            // but stencil clips is open) and whatever is still open at its end is popped.
            var clips = new List<int>();
            for (var k = 0; k < from; k++)
            {
                if (_ops[k].Kind == 1) clips.Add((int)_ops[k].Batch);
                else if (_ops[k].Kind == 2 && clips.Count > 0) clips.RemoveAt(clips.Count - 1);
            }
            foreach (var c in clips) { cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), c, PushPass); depth++; }
            var open = new List<int>(clips);
            for (var k = from; k < to; k++)
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
                        cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, (int)_batches[b].Kind, _props);
                        break;
                    case 1:
                        cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, PushPass);
                        depth++;
                        if (stack.Count == 0) open.Add(b);
                        break;
                    case 2:
                        cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, PopPass);
                        depth--;
                        if (stack.Count == 0 && open.Count > 0) open.RemoveAt(open.Count - 1);
                        break;
                    case 3:
                        stack.Add((target, depth));
                        target = Group(stack.Count - 1, width, height);
                        depth = 0;
                        Bind(cmd, target, into);
                        cmd.ClearRenderTarget(true, true, Color.clear);
                        break;
                    case 5:
                    {
                        // backdrop-filter: what lies under the element (the scene and the page so far), filtered and
                        // put back through the element's shape; the element then draws over it.
                        cmd.DisableScissorRect();
                        var under = _filters.Run(cmd, target, (int)op.Filter, (int)op.FilterCount, width, height, true);
                        Bind(cmd, target, into);
                        _props.Clear();
                        _props.SetTexture(GroupId, under);
                        _props.SetFloat(GroupAlphaId, 1);
                        cmd.DrawMesh(_mesh, Matrix4x4.identity, Composite(depth, 3), b, CompositePass, _props);
                        break;
                    }
                    case 4:
                        var group = _groups[stack.Count - 1];
                        if (op.FilterCount > 0) { cmd.DisableScissorRect(); group = _filters.Run(cmd, group, (int)op.Filter, (int)op.FilterCount, width, height); }
                        (target, depth) = stack[stack.Count - 1];
                        stack.RemoveAt(stack.Count - 1);
                        _props.Clear();
                        _props.SetTexture(GroupId, group);
                        _props.SetFloat(GroupAlphaId, op.Alpha);
                        if (op.Mix != 0 && op.Compose == 3)
                        {
                            // mix-blend-mode reads what lies under the group: a copy of the target so far.
                            var backdrop = Backdrop(width, height);
                            cmd.CopyTexture(target, backdrop);
                            Bind(cmd, target, into);
                            _props.SetTexture(BackdropId, backdrop);
                            _props.SetInt(MixId, (int)op.Mix);
                            cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), b, BlendPass, _props);
                        }
                        else
                        {
                            Bind(cmd, target, into);
                            cmd.DrawMesh(_mesh, Matrix4x4.identity, Composite(depth, op.Compose), b, CompositePass, _props);
                        }
                        break;
                }
            }
            for (var c = open.Count - 1; c >= 0; c--) { cmd.DrawMesh(_mesh, Matrix4x4.identity, AtDepth(depth), open[c], PopPass); depth--; }
        }

        private Material AtDepth(int depth)
        {
            if (_depth.TryGetValue(depth, out var m)) return m;
            m = new Material(_material) { hideFlags = HideFlags.HideAndDontSave };
            m.SetInt(StencilRefId, depth);
            return _depth[depth] = m;
        }

        private Material Composite(int depth, uint compose)
        {
            if (_composite.TryGetValue((depth, compose), out var m)) return m;
            m = new Material(_material) { hideFlags = HideFlags.HideAndDontSave };
            m.SetInt(StencilRefId, depth);
            var (s, d) = compose switch
            {
                0 => (BlendMode.Zero, BlendMode.Zero),
                1 => (BlendMode.One, BlendMode.Zero),
                2 => (BlendMode.Zero, BlendMode.One),
                4 => (BlendMode.OneMinusDstAlpha, BlendMode.One),
                5 => (BlendMode.DstAlpha, BlendMode.Zero),
                6 => (BlendMode.Zero, BlendMode.SrcAlpha),
                7 => (BlendMode.OneMinusDstAlpha, BlendMode.Zero),
                8 => (BlendMode.Zero, BlendMode.OneMinusSrcAlpha),
                9 => (BlendMode.DstAlpha, BlendMode.OneMinusSrcAlpha),
                10 => (BlendMode.OneMinusDstAlpha, BlendMode.SrcAlpha),
                11 => (BlendMode.OneMinusDstAlpha, BlendMode.OneMinusSrcAlpha),
                12 or 13 => (BlendMode.One, BlendMode.One),
                _ => (BlendMode.One, BlendMode.OneMinusSrcAlpha),
            };
            m.SetInt(SrcBlendId, (int)s);
            m.SetInt(DstBlendId, (int)d);
            return _composite[(depth, compose)] = m;
        }

        /// <summary>The scratch copy of the target that a blend mode reads.</summary>
        private RenderTexture Backdrop(int width, int height)
        {
            if (_backdrop != null && _backdrop.width == width && _backdrop.height == height) return _backdrop;
            if (_backdrop != null) UnityEngine.Object.Destroy(_backdrop);
            _backdrop = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 0) { sRGB = false }) { name = "Web HUD backdrop" };
            _backdrop.Create();
            return _backdrop;
        }

        private RenderTexture Group(int level, int width, int height)
        {
            while (_groups.Count <= level) _groups.Add(null);
            var g = _groups[level];
            if (g == null || g.width != width || g.height != height)
            {
                if (g != null) UnityEngine.Object.Destroy(g);
                g = new RenderTexture(new RenderTextureDescriptor(width, height, RenderTextureFormat.ARGB32, 24) { sRGB = false, msaaSamples = Msaa }) { name = "Web HUD group" };
                g.Create();
                _groups[level] = g;
            }
            return g;
        }

        public void Dispose()
        {
            if (Target != null) UnityEngine.Object.Destroy(Target);
            _filters.Dispose();
            if (_backdrop != null) UnityEngine.Object.Destroy(_backdrop);
            foreach (var g in _groups) if (g != null) UnityEngine.Object.Destroy(g);
            foreach (var l in _layers) if (l != null) UnityEngine.Object.Destroy(l);
            _frameCmd.Release();
            foreach (var m in _depth.Values) UnityEngine.Object.Destroy(m);
            foreach (var m in _composite.Values) UnityEngine.Object.Destroy(m);
            foreach (var i in _images.Values) UnityEngine.Object.Destroy(i);
            if (_ramps != null) UnityEngine.Object.Destroy(_ramps);
            if (_atlas != null) UnityEngine.Object.Destroy(_atlas);
            if (_clipTable != null) UnityEngine.Object.Destroy(_clipTable);
            if (_imageAtlas != null) UnityEngine.Object.Destroy(_imageAtlas);
            if (_material != null) UnityEngine.Object.Destroy(_material);
            UnityEngine.Object.Destroy(_mesh);
            _cmd.Release();
        }
    }
}
