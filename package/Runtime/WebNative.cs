using System;
using System.IO;
using System.Runtime.InteropServices;
using UnityEngine;

namespace Vitrail
{
    /// <summary>
    /// The native page renderer (vitrail: Blitz = HTML and CSS without a browser, built from ~/projects/vitrail-ui/native):
    /// a document from HTML text, laid out and painted on the CPU into premultiplied sRGB bytes. Plugins/Vitrail holds the
    /// macOS and Windows builds.
    /// In a player the functions are plain P/Invoke. In the editor the library is loaded by hand from a fresh copy, so a
    /// rebuilt library is picked up while the editor runs (Unity keeps a P/Invoked library until it restarts): when the
    /// file changes, <see cref="Generation"/> goes up and every panel loads its page again on the new code. Old copies
    /// are never unloaded (their worker threads may still be parked in them); it is only the editor.
    /// </summary>
    internal static class WebNative
    {
        public delegate IntPtr CreateFn([MarshalAs(UnmanagedType.LPUTF8Str)] string html, [MarshalAs(UnmanagedType.LPUTF8Str)] string baseUrl, uint width, uint height, float scale);
        public delegate void ResizeFn(IntPtr ui, uint width, uint height, float scale);
        public delegate int RenderFn(IntPtr ui, byte[] output, UIntPtr len);
        public delegate uint BackdropFn(IntPtr ui, byte[] mask, UIntPtr len, float[] parameters, uint maxRegions);
        public delegate void DestroyFn(IntPtr ui);
        public delegate int JsPollFn(IntPtr ui, byte[] name, UIntPtr nameCap, byte[] json, UIntPtr jsonCap);
        public delegate int JsLogFn(IntPtr ui, byte[] buf, UIntPtr cap, out int level);
        public delegate int JsCallFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string json);
        public delegate int KeyFn(IntPtr ui, int down, [MarshalAs(UnmanagedType.LPUTF8Str)] string key, [MarshalAs(UnmanagedType.LPUTF8Str)] string code, uint mods, int repeat);
        public delegate int EditingFn(IntPtr ui);
        public delegate int ClipboardTakeFn(IntPtr ui, byte[] buf, UIntPtr cap);
        public delegate void PasteFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string text);
        public delegate int ImeStateFn(IntPtr ui, float[] area);
        public delegate void ImeFn(IntPtr ui, int kind, [MarshalAs(UnmanagedType.LPUTF8Str)] string text, int caret);
        public delegate void BlurFn(IntPtr ui);
        public delegate int DevTreeFn(IntPtr ui, byte[] buf, UIntPtr cap);
        public delegate int DevNodeFn(IntPtr ui, ulong node, byte[] buf, UIntPtr cap);
        public delegate ulong DevPickFn(IntPtr ui, float x, float y);
        public delegate int DevRectFn(IntPtr ui, ulong node, float[] rect);
        public delegate int DevSetStyleFn(IntPtr ui, ulong node, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        public delegate int NavigateFn(IntPtr ui, int dir);
        public delegate int ActivateFn(IntPtr ui, byte[] action, byte[] id, UIntPtr cap);
        public delegate int CancelFn(IntPtr ui);
        public delegate int ValueFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, byte[] buf, UIntPtr cap);
        public delegate int SelectValueFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        public delegate int ScrollExtentsFn(IntPtr ui, float[] output, UIntPtr max);
        public delegate int JsEvalFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string source);
        public delegate int NeedsPaintFn(IntPtr ui);
        public delegate int SetTextFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        public delegate int SetAttrFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        public delegate int SetClassFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string cls, int on);
        public delegate int SetStyleFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string property, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        public delegate int PointFn(IntPtr ui, float x, float y);
        public delegate void PointerDownFn(IntPtr ui, float x, float y);
        public delegate int PointerUpFn(IntPtr ui, float x, float y, byte[] action, byte[] id, UIntPtr cap);
        public delegate int HoveredFn(IntPtr ui, byte[] action, byte[] id, UIntPtr cap);
        public delegate int RectFn(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, float[] xywh);
        public delegate int PaintStartFn(IntPtr ui);
        public delegate int TickFn(IntPtr ui, double seconds);
        public delegate int ChangedFn(IntPtr ui, uint[] output);
        public delegate int SpansFn(IntPtr ui, uint[] output, UIntPtr max);
        public delegate int DirtyFn(IntPtr ui, float[] output, UIntPtr max);
        public delegate int VertexRangeFn(IntPtr ui, IntPtr output, UIntPtr len, uint start, uint end);
        public delegate int LayerStatesFn(IntPtr ui, float[] output, UIntPtr len);
        public delegate int ScrollFn(IntPtr ui, float x, float y, float dx, float dy);
        public delegate int CursorFn(IntPtr ui);
        public delegate UIntPtr LastErrorFn(byte[] buf, UIntPtr cap);
        public delegate void SetGpuFn(IntPtr ui, int on);
        public delegate uint TextureIdFn([MarshalAs(UnmanagedType.LPUTF8Str)] string name);
        public delegate uint ImageTextureFn(IntPtr ui, uint index);
        public delegate int GpuCountsFn(IntPtr ui, uint[] counts);
        public delegate int GpuCopyFn(IntPtr ui, IntPtr vertices, UIntPtr verticesLen, IntPtr indices, UIntPtr indicesLen, IntPtr batches, UIntPtr batchesLen);
        public delegate int GpuBufferFn(IntPtr ui, IntPtr output, UIntPtr len);
        public delegate int GpuImageFn(IntPtr ui, uint index, uint[] info, IntPtr output, UIntPtr len);
        public delegate int PaintPollFn(IntPtr ui, byte[] output, UIntPtr outLen, uint[] rect, byte[] mask, UIntPtr maskLen, float[] parameters, uint maxRegions, out uint regions);

        public static CreateFn vitrail_create;
        public static ResizeFn vitrail_resize;
        public static RenderFn vitrail_render;
        public static BackdropFn vitrail_backdrop;
        public static DestroyFn vitrail_destroy;
        public static NeedsPaintFn vitrail_needs_paint;
        public static SetTextFn vitrail_set_text;
        public static SetAttrFn vitrail_set_attr;
        public static SetClassFn vitrail_set_class;
        public static SetStyleFn vitrail_set_style;
        public static PointFn vitrail_pointer_move;
        public static PointFn vitrail_hit;
        public static PointerDownFn vitrail_pointer_down;
        public static PointerUpFn vitrail_pointer_up;
        public static HoveredFn vitrail_hovered;
        public static RectFn vitrail_rect;
        public static PaintStartFn vitrail_paint_start;
        public static TickFn vitrail_tick;
        public static ChangedFn vitrail_gpu_changed;
        public static SpansFn vitrail_gpu_vertex_spans;
        public static DirtyFn vitrail_gpu_dirty;
        public static VertexRangeFn vitrail_gpu_vertex_range;
        public static PaintPollFn vitrail_paint_poll;
        public static ScrollFn vitrail_scroll;
        public static CursorFn vitrail_cursor;
        public static LastErrorFn vitrail_last_error;
        public static SetGpuFn vitrail_set_gpu;
        public static GpuCountsFn vitrail_gpu_counts;
        public static GpuCopyFn vitrail_gpu_copy;
        public static GpuBufferFn vitrail_gpu_ops, vitrail_gpu_ramps, vitrail_gpu_atlas, vitrail_gpu_filters, vitrail_gpu_layers, vitrail_gpu_image_atlas, vitrail_gpu_scrolls;
        public static LayerStatesFn vitrail_layer_states, vitrail_gpu_clips, vitrail_scroll_states;
        public static GpuImageFn vitrail_gpu_image;
        public static TextureIdFn vitrail_texture_id;
        public static ImageTextureFn vitrail_gpu_image_texture;
        public static JsPollFn vitrail_js_poll;
        public static JsLogFn vitrail_js_log;
        public static JsCallFn vitrail_js_call;
        public static JsEvalFn vitrail_js_eval;
        public static ScrollExtentsFn vitrail_scroll_extents;
        public static KeyFn vitrail_key;
        public static EditingFn vitrail_editing;
        public static ClipboardTakeFn vitrail_clipboard_take;
        public static EditingFn vitrail_paste_wanted;
        public static PasteFn vitrail_paste;
        public static ImeStateFn vitrail_ime_state;
        public static ImeFn vitrail_ime;
        public static BlurFn vitrail_blur;
        public static DevTreeFn vitrail_dev_tree;
        public static DevNodeFn vitrail_dev_node;
        public static DevPickFn vitrail_dev_pick;
        public static DevRectFn vitrail_dev_rect;
        public static DevSetStyleFn vitrail_dev_set_style;
        public static NavigateFn vitrail_navigate;
        public static ActivateFn vitrail_activate;
        public static CancelFn vitrail_cancel;
        public static ValueFn vitrail_value;
        public static SelectValueFn vitrail_select_value;

        /// <summary>Goes up each time a new build of the library is loaded (editor only); panels reload their page then.</summary>
        public static int Generation { get; private set; }

        static WebNative() => Load();

        private static readonly byte[] ErrorBuffer = new byte[2048];

        /// <summary>Puts a panic from inside the library (caught at its edge) in the console.</summary>
        public static void ReportErrors()
        {
            if (vitrail_last_error == null) return;
            if ((ulong)vitrail_last_error(ErrorBuffer, (UIntPtr)ErrorBuffer.Length) > 0) Debug.LogError("Vitrail: " + Str(ErrorBuffer));
        }

        /// <summary>A NUL-terminated UTF-8 buffer as a string.</summary>
        public static string Str(byte[] b)
        {
            var n = Array.IndexOf(b, (byte)0);
            return System.Text.Encoding.UTF8.GetString(b, 0, n < 0 ? b.Length : n);
        }

#if UNITY_EDITOR
        private static string _source;
        private static DateTime _loadedStamp;
        private static float _nextCheck;

        private static string SourcePath => Application.platform == RuntimePlatform.WindowsEditor
            ? Path.GetFullPath("Packages/com.vitrail.ui/Plugins/Windows/x86_64/vitrail.dll")
            : Application.platform == RuntimePlatform.LinuxEditor
                ? Path.GetFullPath("Packages/com.vitrail.ui/Plugins/Linux/x86_64/libvitrail.so")
                : Path.GetFullPath("Packages/com.vitrail.ui/Plugins/macOS/libvitrail.dylib");

        /// <summary>Loads a newer build of the library if the file changed (checked twice a second). True when it did.</summary>
        public static bool Refresh()
        {
            if (Time.realtimeSinceStartup < _nextCheck) return false;
            _nextCheck = Time.realtimeSinceStartup + 0.5f;
            if (!File.Exists(SourcePath) || File.GetLastWriteTimeUtc(SourcePath) == _loadedStamp) return false;
            Load();
            return true;
        }

        private static void Load()
        {
            _source = SourcePath;
            if (!File.Exists(_source)) { Debug.LogError("Vitrail: native library not found at " + _source); return; }
            _loadedStamp = File.GetLastWriteTimeUtc(_source);
            // A copy with its own name: the system loader hands back the old image for a path it has loaded before.
            var dir = Path.GetFullPath("Library/VitrailNative");
            Directory.CreateDirectory(dir);
            var copy = Path.Combine(dir, $"vitrail-{_loadedStamp.Ticks}{Path.GetExtension(_source)}");
            if (!File.Exists(copy)) File.Copy(_source, copy);
            var lib = Open(copy);
            if (lib == IntPtr.Zero) { Debug.LogError("Vitrail: could not load " + copy); return; }
            T F<T>(string name) where T : Delegate
            {
                var p = Symbol(lib, name);
                if (p == IntPtr.Zero) throw new MissingMethodException("vitrail", name);
                return Marshal.GetDelegateForFunctionPointer<T>(p);
            }
            vitrail_create = F<CreateFn>("vitrail_create");
            vitrail_resize = F<ResizeFn>("vitrail_resize");
            vitrail_render = F<RenderFn>("vitrail_render");
            vitrail_backdrop = F<BackdropFn>("vitrail_backdrop");
            vitrail_destroy = F<DestroyFn>("vitrail_destroy");
            vitrail_needs_paint = F<NeedsPaintFn>("vitrail_needs_paint");
            vitrail_set_text = F<SetTextFn>("vitrail_set_text");
            vitrail_set_attr = F<SetAttrFn>("vitrail_set_attr");
            vitrail_set_class = F<SetClassFn>("vitrail_set_class");
            vitrail_set_style = F<SetStyleFn>("vitrail_set_style");
            vitrail_pointer_move = F<PointFn>("vitrail_pointer_move");
            vitrail_hit = F<PointFn>("vitrail_hit");
            vitrail_pointer_down = F<PointerDownFn>("vitrail_pointer_down");
            vitrail_pointer_up = F<PointerUpFn>("vitrail_pointer_up");
            vitrail_hovered = F<HoveredFn>("vitrail_hovered");
            vitrail_rect = F<RectFn>("vitrail_rect");
            vitrail_paint_start = F<PaintStartFn>("vitrail_paint_start");
            vitrail_tick = F<TickFn>("vitrail_tick");
            vitrail_gpu_changed = F<ChangedFn>("vitrail_gpu_changed");
            vitrail_gpu_vertex_spans = F<SpansFn>("vitrail_gpu_vertex_spans");
            vitrail_gpu_dirty = F<DirtyFn>("vitrail_gpu_dirty");
            vitrail_gpu_vertex_range = F<VertexRangeFn>("vitrail_gpu_vertex_range");
            vitrail_paint_poll = F<PaintPollFn>("vitrail_paint_poll");
            vitrail_scroll = F<ScrollFn>("vitrail_scroll");
            vitrail_cursor = F<CursorFn>("vitrail_cursor");
            vitrail_last_error = F<LastErrorFn>("vitrail_last_error");
            vitrail_set_gpu = F<SetGpuFn>("vitrail_set_gpu");
            vitrail_gpu_counts = F<GpuCountsFn>("vitrail_gpu_counts");
            vitrail_gpu_copy = F<GpuCopyFn>("vitrail_gpu_copy");
            vitrail_js_poll = F<JsPollFn>("vitrail_js_poll");
            vitrail_js_log = F<JsLogFn>("vitrail_js_log");
            vitrail_js_call = F<JsCallFn>("vitrail_js_call");
            vitrail_js_eval = F<JsEvalFn>("vitrail_js_eval");
            vitrail_scroll_extents = F<ScrollExtentsFn>("vitrail_scroll_extents");
            vitrail_key = F<KeyFn>("vitrail_key");
            vitrail_editing = F<EditingFn>("vitrail_editing");
            vitrail_clipboard_take = F<ClipboardTakeFn>("vitrail_clipboard_take");
            vitrail_paste_wanted = F<EditingFn>("vitrail_paste_wanted");
            vitrail_paste = F<PasteFn>("vitrail_paste");
            vitrail_ime_state = F<ImeStateFn>("vitrail_ime_state");
            vitrail_ime = F<ImeFn>("vitrail_ime");
            vitrail_blur = F<BlurFn>("vitrail_blur");
            vitrail_dev_tree = F<DevTreeFn>("vitrail_dev_tree");
            vitrail_dev_node = F<DevNodeFn>("vitrail_dev_node");
            vitrail_dev_pick = F<DevPickFn>("vitrail_dev_pick");
            vitrail_dev_rect = F<DevRectFn>("vitrail_dev_rect");
            vitrail_dev_set_style = F<DevSetStyleFn>("vitrail_dev_set_style");
            vitrail_navigate = F<NavigateFn>("vitrail_navigate");
            vitrail_activate = F<ActivateFn>("vitrail_activate");
            vitrail_cancel = F<CancelFn>("vitrail_cancel");
            vitrail_value = F<ValueFn>("vitrail_value");
            vitrail_select_value = F<SelectValueFn>("vitrail_select_value");
            vitrail_gpu_ops = F<GpuBufferFn>("vitrail_gpu_ops");
            vitrail_gpu_ramps = F<GpuBufferFn>("vitrail_gpu_ramps");
            vitrail_gpu_atlas = F<GpuBufferFn>("vitrail_gpu_atlas");
            vitrail_gpu_filters = F<GpuBufferFn>("vitrail_gpu_filters");
            vitrail_gpu_layers = F<GpuBufferFn>("vitrail_gpu_layers");
            vitrail_gpu_image_atlas = F<GpuBufferFn>("vitrail_gpu_image_atlas");
            vitrail_layer_states = F<LayerStatesFn>("vitrail_layer_states");
            vitrail_gpu_clips = F<LayerStatesFn>("vitrail_gpu_clips");
            vitrail_scroll_states = F<LayerStatesFn>("vitrail_scroll_states");
            vitrail_gpu_scrolls = F<GpuBufferFn>("vitrail_gpu_scrolls");
            vitrail_gpu_image = F<GpuImageFn>("vitrail_gpu_image");
            vitrail_texture_id = F<TextureIdFn>("vitrail_texture_id");
            vitrail_gpu_image_texture = F<ImageTextureFn>("vitrail_gpu_image_texture");
            Generation++;
            if (Generation > 1) Debug.Log("Vitrail: loaded the new native library " + Path.GetFileName(copy));
        }

        private static IntPtr Open(string path) => Application.platform switch
        {
            RuntimePlatform.WindowsEditor => LoadLibraryW(path),
            RuntimePlatform.LinuxEditor => dlopenLinux(path, 2 /* RTLD_NOW */),
            _ => dlopen(path, 2 /* RTLD_NOW */),
        };
        private static IntPtr Symbol(IntPtr lib, string name) => Application.platform switch
        {
            RuntimePlatform.WindowsEditor => GetProcAddress(lib, name),
            RuntimePlatform.LinuxEditor => dlsymLinux(lib, name),
            _ => dlsym(lib, name),
        };

        [DllImport("libdl.so.2", EntryPoint = "dlopen")] private static extern IntPtr dlopenLinux(string path, int mode);
        [DllImport("libdl.so.2", EntryPoint = "dlsym")] private static extern IntPtr dlsymLinux(IntPtr handle, string name);

        [DllImport("/usr/lib/libSystem.dylib")] private static extern IntPtr dlopen(string path, int mode);
        [DllImport("/usr/lib/libSystem.dylib")] private static extern IntPtr dlsym(IntPtr handle, string name);
        [DllImport("kernel32", CharSet = CharSet.Unicode)] private static extern IntPtr LoadLibraryW(string path);
        [DllImport("kernel32", CharSet = CharSet.Ansi)] private static extern IntPtr GetProcAddress(IntPtr lib, string name);
#else
        public static bool Refresh() => false;

        private static void Load()
        {
            vitrail_create = P.vitrail_create;
            vitrail_resize = P.vitrail_resize;
            vitrail_render = P.vitrail_render;
            vitrail_backdrop = P.vitrail_backdrop;
            vitrail_destroy = P.vitrail_destroy;
            vitrail_needs_paint = P.vitrail_needs_paint;
            vitrail_set_text = P.vitrail_set_text;
            vitrail_set_attr = P.vitrail_set_attr;
            vitrail_set_class = P.vitrail_set_class;
            vitrail_set_style = P.vitrail_set_style;
            vitrail_pointer_move = P.vitrail_pointer_move;
            vitrail_hit = P.vitrail_hit;
            vitrail_pointer_down = P.vitrail_pointer_down;
            vitrail_pointer_up = P.vitrail_pointer_up;
            vitrail_hovered = P.vitrail_hovered;
            vitrail_rect = P.vitrail_rect;
            vitrail_paint_start = P.vitrail_paint_start;
            vitrail_tick = P.vitrail_tick;
            vitrail_gpu_changed = P.vitrail_gpu_changed;
            vitrail_gpu_vertex_spans = P.vitrail_gpu_vertex_spans;
            vitrail_gpu_dirty = P.vitrail_gpu_dirty;
            vitrail_gpu_vertex_range = P.vitrail_gpu_vertex_range;
            vitrail_paint_poll = P.vitrail_paint_poll;
            vitrail_scroll = P.vitrail_scroll;
            vitrail_cursor = P.vitrail_cursor;
            vitrail_last_error = P.vitrail_last_error;
            vitrail_set_gpu = P.vitrail_set_gpu;
            vitrail_gpu_counts = P.vitrail_gpu_counts;
            vitrail_gpu_copy = P.vitrail_gpu_copy;
            vitrail_js_poll = P.vitrail_js_poll;
            vitrail_js_log = P.vitrail_js_log;
            vitrail_js_call = P.vitrail_js_call;
            vitrail_js_eval = P.vitrail_js_eval;
            vitrail_scroll_extents = P.vitrail_scroll_extents;
            vitrail_key = P.vitrail_key;
            vitrail_editing = P.vitrail_editing;
            vitrail_clipboard_take = P.vitrail_clipboard_take;
            vitrail_paste_wanted = P.vitrail_paste_wanted;
            vitrail_paste = P.vitrail_paste;
            vitrail_ime_state = P.vitrail_ime_state;
            vitrail_ime = P.vitrail_ime;
            vitrail_blur = P.vitrail_blur;
            vitrail_dev_tree = P.vitrail_dev_tree;
            vitrail_dev_node = P.vitrail_dev_node;
            vitrail_dev_pick = P.vitrail_dev_pick;
            vitrail_dev_rect = P.vitrail_dev_rect;
            vitrail_dev_set_style = P.vitrail_dev_set_style;
            vitrail_navigate = P.vitrail_navigate;
            vitrail_activate = P.vitrail_activate;
            vitrail_cancel = P.vitrail_cancel;
            vitrail_value = P.vitrail_value;
            vitrail_select_value = P.vitrail_select_value;
            vitrail_gpu_ops = P.vitrail_gpu_ops;
            vitrail_gpu_ramps = P.vitrail_gpu_ramps;
            vitrail_gpu_atlas = P.vitrail_gpu_atlas;
            vitrail_gpu_filters = P.vitrail_gpu_filters;
            vitrail_gpu_layers = P.vitrail_gpu_layers;
            vitrail_gpu_image_atlas = P.vitrail_gpu_image_atlas;
            vitrail_layer_states = P.vitrail_layer_states;
            vitrail_gpu_clips = P.vitrail_gpu_clips;
            vitrail_scroll_states = P.vitrail_scroll_states;
            vitrail_gpu_scrolls = P.vitrail_gpu_scrolls;
            vitrail_gpu_image = P.vitrail_gpu_image;
            vitrail_texture_id = P.vitrail_texture_id;
            vitrail_gpu_image_texture = P.vitrail_gpu_image_texture;
            Generation = 1;
        }

        /// <summary>The library as Unity ships it with a player.</summary>
        private static class P
        {
            private const string Lib = "vitrail";
            [DllImport(Lib)] public static extern IntPtr vitrail_create([MarshalAs(UnmanagedType.LPUTF8Str)] string html, [MarshalAs(UnmanagedType.LPUTF8Str)] string baseUrl, uint width, uint height, float scale);
            [DllImport(Lib)] public static extern void vitrail_resize(IntPtr ui, uint width, uint height, float scale);
            [DllImport(Lib)] public static extern int vitrail_render(IntPtr ui, byte[] output, UIntPtr len);
            [DllImport(Lib)] public static extern uint vitrail_backdrop(IntPtr ui, byte[] mask, UIntPtr len, float[] parameters, uint maxRegions);
            [DllImport(Lib)] public static extern void vitrail_destroy(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_needs_paint(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_set_text(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
            [DllImport(Lib)] public static extern int vitrail_set_attr(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
            [DllImport(Lib)] public static extern int vitrail_set_class(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string cls, int on);
            [DllImport(Lib)] public static extern int vitrail_set_style(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string property, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
            [DllImport(Lib)] public static extern int vitrail_pointer_move(IntPtr ui, float x, float y);
            [DllImport(Lib)] public static extern int vitrail_hit(IntPtr ui, float x, float y);
            [DllImport(Lib)] public static extern void vitrail_pointer_down(IntPtr ui, float x, float y);
            [DllImport(Lib)] public static extern int vitrail_pointer_up(IntPtr ui, float x, float y, byte[] action, byte[] id, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_hovered(IntPtr ui, byte[] action, byte[] id, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_rect(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, float[] xywh);
            [DllImport(Lib)] public static extern int vitrail_paint_start(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_tick(IntPtr ui, double seconds);
            [DllImport(Lib)] public static extern int vitrail_gpu_changed(IntPtr ui, uint[] output);
            [DllImport(Lib)] public static extern int vitrail_gpu_vertex_spans(IntPtr ui, uint[] output, UIntPtr max);
            [DllImport(Lib)] public static extern int vitrail_gpu_dirty(IntPtr ui, float[] output, UIntPtr max);
            [DllImport(Lib)] public static extern int vitrail_gpu_vertex_range(IntPtr ui, IntPtr output, UIntPtr len, uint start, uint end);
            [DllImport(Lib)] public static extern int vitrail_scroll(IntPtr ui, float x, float y, float dx, float dy);
            [DllImport(Lib)] public static extern int vitrail_cursor(IntPtr ui);
            [DllImport(Lib)] public static extern UIntPtr vitrail_last_error(byte[] buf, UIntPtr cap);
            [DllImport(Lib)] public static extern void vitrail_set_gpu(IntPtr ui, int on);
            [DllImport(Lib)] public static extern int vitrail_gpu_counts(IntPtr ui, uint[] counts);
            [DllImport(Lib)] public static extern int vitrail_js_poll(IntPtr ui, byte[] name, UIntPtr nameCap, byte[] json, UIntPtr jsonCap);
            [DllImport(Lib)] public static extern int vitrail_js_log(IntPtr ui, byte[] buf, UIntPtr cap, out int level);
            [DllImport(Lib)] public static extern int vitrail_js_call(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string json);
            [DllImport(Lib)] public static extern int vitrail_key(IntPtr ui, int down, [MarshalAs(UnmanagedType.LPUTF8Str)] string key, [MarshalAs(UnmanagedType.LPUTF8Str)] string code, uint mods, int repeat);
            [DllImport(Lib)] public static extern int vitrail_editing(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_clipboard_take(IntPtr ui, byte[] buf, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_paste_wanted(IntPtr ui);
            [DllImport(Lib)] public static extern void vitrail_paste(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string text);
            [DllImport(Lib)] public static extern int vitrail_ime_state(IntPtr ui, float[] area);
            [DllImport(Lib)] public static extern void vitrail_ime(IntPtr ui, int kind, [MarshalAs(UnmanagedType.LPUTF8Str)] string text, int caret);
            [DllImport(Lib)] public static extern void vitrail_blur(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_dev_tree(IntPtr ui, byte[] buf, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_dev_node(IntPtr ui, ulong node, byte[] buf, UIntPtr cap);
            [DllImport(Lib)] public static extern ulong vitrail_dev_pick(IntPtr ui, float x, float y);
            [DllImport(Lib)] public static extern int vitrail_dev_rect(IntPtr ui, ulong node, float[] rect);
            [DllImport(Lib)] public static extern int vitrail_dev_set_style(IntPtr ui, ulong node, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
            [DllImport(Lib)] public static extern int vitrail_navigate(IntPtr ui, int dir);
            [DllImport(Lib)] public static extern int vitrail_activate(IntPtr ui, byte[] action, byte[] id, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_cancel(IntPtr ui);
            [DllImport(Lib)] public static extern int vitrail_value(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, byte[] buf, UIntPtr cap);
            [DllImport(Lib)] public static extern int vitrail_select_value(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
            [DllImport(Lib)] public static extern int vitrail_scroll_extents(IntPtr ui, float[] output, UIntPtr max);
            [DllImport(Lib)] public static extern int vitrail_js_eval(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string source);
            [DllImport(Lib)] public static extern int vitrail_gpu_copy(IntPtr ui, IntPtr vertices, UIntPtr verticesLen, IntPtr indices, UIntPtr indicesLen, IntPtr batches, UIntPtr batchesLen);
            [DllImport(Lib)] public static extern int vitrail_gpu_ops(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_ramps(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_atlas(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_filters(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_layers(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_image_atlas(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_layer_states(IntPtr ui, float[] output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_clips(IntPtr ui, float[] output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_scroll_states(IntPtr ui, float[] output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_gpu_scrolls(IntPtr ui, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern uint vitrail_texture_id([MarshalAs(UnmanagedType.LPUTF8Str)] string name);
            [DllImport(Lib)] public static extern uint vitrail_gpu_image_texture(IntPtr ui, uint index);
            [DllImport(Lib)] public static extern int vitrail_gpu_image(IntPtr ui, uint index, uint[] info, IntPtr output, UIntPtr len);
            [DllImport(Lib)] public static extern int vitrail_paint_poll(IntPtr ui, byte[] output, UIntPtr outLen, uint[] rect, byte[] mask, UIntPtr maskLen, float[] parameters, uint maxRegions, out uint regions);
        }
#endif
    }
}
