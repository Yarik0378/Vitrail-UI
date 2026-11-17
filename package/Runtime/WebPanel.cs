using System;
using System.Collections.Generic;
using System.IO;
using UnityEngine;
using UnityEngine.Experimental.Rendering;
using UnityEngine.InputSystem;
using UnityEngine.Rendering;
using UnityEngine.Rendering.RenderGraphModule;
using UnityEngine.Rendering.Universal;

namespace Vitrail
{
    /// <summary>
    /// An HTML page drawn over a camera's picture: the page (a file under StreamingAssets) is laid out and painted by the
    /// native library into a rectangle of the screen (`Area`; by default a band across the top `cssHeight` CSS px tall,
    /// or the whole screen), and a URP pass after post-processing mixes it over the picture the way a browser does, in
    /// sRGB, with `backdrop-filter` blurring the scene under it. With `TargetCamera` set to a camera rendering into a
    /// RenderTexture, the page goes into that texture (a panel in the world: see VitrailWorld).
    /// The game fills the page through CSS selectors (SetText, SetAttr, SetClass, SetStyle) or scripts (Call, Triggered);
    /// clicks on elements with a `data-action` come back as Clicked. It repaints only what changed, off the main thread,
    /// and in the editor it reloads when a file next to the page is saved. Call Update(scale) once a frame.
    /// </summary>
    public sealed class WebPanel : IDisposable
    {
        private static readonly int TexId = Shader.PropertyToID("_VitrailTex");
        private static readonly int MaskId = Shader.PropertyToID("_VitrailMask");
        private static readonly int BlurId = Shader.PropertyToID("_VitrailBlur");
        private static readonly int BandId = Shader.PropertyToID("_VitrailBand");
        private static readonly int PageUvId = Shader.PropertyToID("_VitrailPageUv");
        private static readonly int MatrixId = Shader.PropertyToID("_VitrailRegionMatrix");
        private static readonly int OffsetId = Shader.PropertyToID("_VitrailRegionOffset");
        private static readonly int InfoId = Shader.PropertyToID("_VitrailRegionInfo");
        private static readonly int SigmaId = Shader.PropertyToID("_VitrailSigma");
        private static readonly int TexelId = Shader.PropertyToID("_VitrailTexel");
        private static readonly int SceneId = Shader.PropertyToID("_VitrailScene");
        private static readonly int ThumbSizeId = Shader.PropertyToID("_VitrailThumbSize");
        private static readonly int ThumbColorId = Shader.PropertyToID("_VitrailThumbColor");
        private static readonly int[] BlurredIds = { Shader.PropertyToID("_VitrailBlurred0"), Shader.PropertyToID("_VitrailBlurred1"), Shader.PropertyToID("_VitrailBlurred2"), Shader.PropertyToID("_VitrailBlurred3") };
        private const int MaxRegions = 8, MaxBlurs = 4;

        private readonly string _path;
        private readonly float _cssHeight;
        private IntPtr _ui;
        private Texture2D _tex, _mask;
        private byte[] _pixels, _maskPixels;
        private int _w, _h;
        /// <summary>Where the page's top-left corner is on the screen, in pixels from the screen's top-left.</summary>
        private int _x, _y;
        private float _scale;
        private bool _hasBackdrop;
        private readonly float[] _backdrop = new float[21 * MaxRegions];
        private readonly Matrix4x4[] _matrices = new Matrix4x4[MaxRegions];
        private readonly Vector4[] _offsets = new Vector4[MaxRegions];
        private readonly Vector4[] _infos = new Vector4[MaxRegions];
        private readonly List<float> _sigmas = new();
        private readonly Dictionary<string, Action> _state = new();
        private readonly byte[] _action = new byte[256], _id = new byte[256];
        private readonly float[] _rect = new float[4];
        private Vector2 _lastPointer = new(-1, -1);
        /// <summary>The left button went down on this page and isn't released yet.</summary>
        private bool _pressed;
        private string _hoverAction = "", _hoverId = "";
        private CompositePass _pass;
        private FileSystemWatcher _watcher;
        private volatile bool _fileChanged;
        private int _generation;
        private WebGpuPainter _gpu;
        private bool _gpuOn;
        private static readonly int FlipId = Shader.PropertyToID("_VitrailFlip");
        /// <summary>Paint with the GPU painter (triangles drawn by Unity) rather than CPU pixels; the CPU path stays as
        /// the fallback when the GPU painter's shader isn't there.</summary>
        public bool UseGpu = true;

        /// <summary>The page's script called `engine.trigger(name, ...args)`: (name, the arguments as a JSON array).</summary>
        public event Action<string, string> Triggered;

        /// <summary>Calls the page's `engine.on(name, ...)` handlers with `json` (a JSON array of arguments, or one value).</summary>
        public void Call(string name, string json = "[]") { if (_ui != IntPtr.Zero) WebNative.vitrail_js_call(_ui, name, json); }

        /// <summary>Runs JavaScript in the page (its console and errors go to the Unity console).</summary>
        public void Eval(string source) { if (_ui != IntPtr.Zero) WebNative.vitrail_js_eval(_ui, source); }

        /// <summary>Hands the page a data model for its data-bind-* attributes (as Coherent's engine.createModel and
        /// synchronizeModels): `json` is the whole model (an object), `name` what the page's expressions call it. Only
        /// values that changed are written into the page.</summary>
        public void SetModel(string name, string json) => Call("__model", "[\"" + name.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"," + json + "]");

        private byte[] _jsName = new byte[256], _jsText = new byte[4096];

        /// <summary>The page's console into the Unity console, and its `engine.trigger`s to `Triggered`.</summary>
        private void DrainScript()
        {
            for (var guard = 0; guard < 1000; guard++)
            {
                var r = WebNative.vitrail_js_log(_ui, _jsText, (UIntPtr)_jsText.Length, out var level);
                if (r < 0) { _jsText = new byte[-r * 2]; continue; }
                if (r == 0) break;
                var msg = $"[{Path.GetFileName(_path)}] {WebNative.Str(_jsText)}";
                if (level >= 2) Debug.LogError(msg); else if (level == 1) Debug.LogWarning(msg); else Debug.Log(msg);
            }
            for (var guard = 0; guard < 1000; guard++)
            {
                var r = WebNative.vitrail_js_poll(_ui, _jsName, (UIntPtr)_jsName.Length, _jsText, (UIntPtr)_jsText.Length);
                if (r < 0) { var n = -r * 2; if (n > _jsName.Length) _jsName = new byte[n]; if (n > _jsText.Length) _jsText = new byte[n]; continue; }
                if (r == 0) break;
                Triggered?.Invoke(WebNative.Str(_jsName), WebNative.Str(_jsText));
            }
        }

        /// <summary>A text field of the page has the focus: the game should leave typing keys alone.</summary>
        public bool EditingText { get; private set; }

        private readonly List<char> _typed = new();
        private Keyboard _hooked;

        /// <summary>The keyboard to the page: keys without a character (Enter, arrows, Backspace, F-keys) as they go down
        /// and up, characters as the system types them (any layout), Ctrl/Cmd shortcuts by letter. Escape leaves a text
        /// field (the game's convention).</summary>
        private void Keys()
        {
            var kb = Keyboard.current;
            if (kb == null) return;
            if (_hooked != kb)
            {
                if (_hooked != null) _hooked.onTextInput -= OnText;
                kb.onTextInput += OnText;
                _hooked = kb;
            }
            uint mods = (kb.shiftKey.isPressed ? 1u : 0) | (kb.ctrlKey.isPressed ? 2u : 0) | (kb.altKey.isPressed ? 4u : 0) | ((kb.leftCommandKey.isPressed || kb.rightCommandKey.isPressed) ? 8u : 0);
            if (EditingText && kb.escapeKey.wasPressedThisFrame) { WebNative.vitrail_blur(_ui); _typed.Clear(); }
            else
            {
                foreach (var k in kb.allKeys)
                {
                    if (k == null) continue; // slots for keys this keyboard doesn't have
                    var down = k.wasPressedThisFrame;
                    if (!down && !k.wasReleasedThisFrame) continue;
                    var (name, code) = DomKey(k.keyCode);
                    // Shortcuts: a letter with Ctrl or Cmd (the system types no character for them).
                    if (name == null && (mods & 10) != 0 && k.keyCode >= Key.A && k.keyCode <= Key.Z) { name = ((char)('a' + (k.keyCode - Key.A))).ToString(); code = "Key" + (char)('A' + (k.keyCode - Key.A)); }
                    if (name != null) WebNative.vitrail_key(_ui, down ? 1 : 0, name, code, mods, 0);
                }
                foreach (var c in _typed)
                {
                    if (char.IsControl(c)) continue;
                    var s = c.ToString();
                    WebNative.vitrail_key(_ui, 1, s, "", mods & ~1u, 0);
                    WebNative.vitrail_key(_ui, 0, s, "", mods & ~1u, 0);
                }
            }
            _typed.Clear();
            EditingText = WebNative.vitrail_editing(_ui) != 0;
            Clipboard();
            Ime(kb);
        }

        private void OnText(char c) { if (Visible) _typed.Add(c); }

        private byte[] _clip = new byte[1024];

        /// <summary>The system clipboard: what the page copied goes onto it; a paste the page asks for comes from it.</summary>
        private void Clipboard()
        {
            var r = WebNative.vitrail_clipboard_take(_ui, _clip, (UIntPtr)_clip.Length);
            if (r < 0) { _clip = new byte[-r * 2]; r = WebNative.vitrail_clipboard_take(_ui, _clip, (UIntPtr)_clip.Length); }
            if (r == 1) GUIUtility.systemCopyBuffer = WebNative.Str(_clip);
            if (WebNative.vitrail_paste_wanted(_ui) != 0) WebNative.vitrail_paste(_ui, GUIUtility.systemCopyBuffer ?? "");
        }

        private readonly float[] _imeArea = new float[4];
        private bool _imeOn;
        private string _composition;

        /// <summary>The system IME (Chinese, Japanese, Korean input) while a text field of the page has the focus: on,
        /// placed at the field's caret, its composition shown in the field; the committed text comes as typed characters.</summary>
        private void Ime(Keyboard kb)
        {
            var on = WebNative.vitrail_ime_state(_ui, _imeArea) != 0;
            if (on != _imeOn)
            {
                _imeOn = on;
                kb.SetIMEEnabled(on);
                if (on) kb.onIMECompositionChange += OnComposition; else kb.onIMECompositionChange -= OnComposition;
            }
            if (on) kb.SetIMECursorPosition(new Vector2(_x + _imeArea[0], Screen.height - (_y + _imeArea[1] + _imeArea[3])));
            if (_composition != null)
            {
                WebNative.vitrail_ime(_ui, _composition.Length == 0 ? 2 : 0, _composition, -1);
                _composition = null;
            }
        }

        private void OnComposition(UnityEngine.InputSystem.LowLevel.IMECompositionString s) => _composition = s.ToString();

        /// <summary>The DOM's key and code for a key that types no character; nulls for the rest.</summary>
        private static (string, string) DomKey(Key k) => k switch
        {
            Key.Enter => ("Enter", "Enter"), Key.NumpadEnter => ("Enter", "NumpadEnter"), Key.Escape => ("Escape", "Escape"),
            Key.Backspace => ("Backspace", "Backspace"), Key.Tab => ("Tab", "Tab"), Key.Delete => ("Delete", "Delete"), Key.Insert => ("Insert", "Insert"),
            Key.LeftArrow => ("ArrowLeft", "ArrowLeft"), Key.RightArrow => ("ArrowRight", "ArrowRight"), Key.UpArrow => ("ArrowUp", "ArrowUp"), Key.DownArrow => ("ArrowDown", "ArrowDown"),
            Key.Home => ("Home", "Home"), Key.End => ("End", "End"), Key.PageUp => ("PageUp", "PageUp"), Key.PageDown => ("PageDown", "PageDown"),
            >= Key.F1 and <= Key.F12 => ("F" + (k - Key.F1 + 1), "F" + (k - Key.F1 + 1)),
            _ => (null, null),
        };

        /// <summary>An element with `data-action` was clicked: (action, id).</summary>
        public event Action<string, string> Clicked;
        /// <summary>The element under the pointer changed: (action, id), empty when none.</summary>
        public event Action<string, string> HoverChanged;
        /// <summary>The page was (re)loaded: the place to set what depends on its size.</summary>
        public event Action Loaded;
        /// <summary>Whether UI Toolkit has something at a screen point (Unity's, y up): then the page doesn't get the pointer.</summary>
        public Func<Vector2, bool> Blocked;
        /// <summary>How long the last repaint took (style, layout, paint, upload), in ms.</summary>
        public float LastPaintMs { get; private set; }
        /// <summary>How long the main thread spent starting the last paint (styles, layout, the list of what to draw), ms.</summary>
        public float LastStartMs { get; private set; }
        /// <summary>The last change uploaded, in device px.</summary>
        public Vector2Int LastChange { get; private set; }
        /// <summary>A line for the HUD's F8 readout.</summary>
        public string Stats => $"web {Path.GetFileName(_path)} ({(_gpuOn ? "GPU" : "CPU")}): main {LastStartMs:F2} ms · worker {LastPaintMs:F1} ms · last change {LastChange.x}×{LastChange.y} of {_w}×{_h}{(_gpuOn && _gpu != null && _gpu.Assembled ? $" · every frame: {_gpu.LiveOps} steps" : "")}{(Animating ? " · animating" : "")}{(_gpuOn && _gpu != null ? $" · upload {_gpu.LastUpload / 1024} KB" : "")}";
        /// <summary>Whether a CSS animation or transition is playing (the page then paints every frame).</summary>
        public bool Animating { get; private set; }
        private double _loadedAt;
        /// <summary>Device pixels per CSS pixel.</summary>
        public float Scale => _scale;
        /// <summary>The page's width in CSS pixels.</summary>
        public float CssWidth => _w / Mathf.Max(0.01f, _scale);
        public bool Visible = true;
        /// <summary>Updated and painted but not drawn, and not taking the pointer: a page just shown is held until the paint
        /// with its new content is in, so the screen never shows it empty or with what it had before.</summary>
        public bool Hold;
        /// <summary>Still drawn but out of play (a page fading out after it was closed): it covers nothing and takes no
        /// pointer or keys, so what lies under it can be clicked at once.</summary>
        public bool Ghost;
        /// <summary>Repaints started and finished (taken in) since the panel was made: a paint started after a change has
        /// landed once <see cref="PaintsLanded"/> passes the <see cref="PaintsStarted"/> read just after the change.</summary>
        public int PaintsStarted { get; private set; }
        public int PaintsLanded { get; private set; }
        /// <summary>The camera whose picture the page is drawn over; null: Camera.main.</summary>
        public Camera TargetCamera;
        /// <summary>Maps the mouse's screen point (Unity's, y up) to a point on the page in its pixels from the top-left,
        /// or null when the pointer isn't over it. Null (the default): the page's screen rectangle. A panel in the world
        /// sets it to a raycast into its surface.</summary>
        public Func<Vector2, Vector2?> PointerToPage;
        /// <summary>The cursor shown over elements with `cursor: pointer` (null: the system cursor), and its hotspot.</summary>
        public static Texture2D HandCursor;
        public static Vector2 HandHotspot = new(4, 0);
        private Camera Cam => TargetCamera != null ? TargetCamera : Camera.main;
        private int ScreenW => TargetCamera != null ? TargetCamera.pixelWidth : Screen.width;
        private int ScreenH => TargetCamera != null ? TargetCamera.pixelHeight : Screen.height;
        /// <summary>Where the page sits, given the HUD's scale: a rectangle in screen pixels from the top-left. Null: a band
        /// across the top of the screen, `cssHeight` CSS px tall (or the whole screen).</summary>
        public Func<float, RectInt> Area;

        /// <param name="file">A page under StreamingAssets, e.g. "ui/topbar.html".</param>
        /// <param name="cssHeight">How much of the screen's top the page covers, in CSS px; 0 or less: the whole screen
        /// (a window).</param>
        /// <summary>Every live panel: the game asks them all whether the pointer is over one (`AnyCovers`), so the camera
        /// doesn't zoom or click through any of them.</summary>
        private static readonly List<WebPanel> Live = new();

        /// <summary>Whether a screen point (Unity's, y up) is over any web panel's content or scrollbar.</summary>
        public static bool AnyCovers(Vector2 screen) { foreach (var p in Live) if (p.Covers(screen)) return true; return false; }

        /// <summary>Whether a page drawn over this one covers a screen point. Pages are drawn in the order they were made,
        /// so a later page is above: a click on it must not reach the pages under it too.</summary>
        private bool CoveredAbove(Vector2 screen)
        {
            for (var i = Live.IndexOf(this) + 1; i > 0 && i < Live.Count; i++)
                if (Live[i].Covers(screen)) return true;
            return false;
        }

        public WebPanel(string file, float cssHeight)
        {
            Live.Add(this);
            _path = Path.Combine(Application.streamingAssetsPath, file);
            _cssHeight = cssHeight;
            _pass = new CompositePass(this);
            RenderPipelineManager.beginCameraRendering += Enqueue;
        }

        private readonly Dictionary<string, Texture> _textures = new();
        private bool _texturesChanged;

        /// <summary>Shows a game texture (a render texture of a camera, a portrait) where the page has
        /// &lt;img src="texture:name?w=…&amp;h=…"&gt; (w and h: its size for layout when CSS doesn't set one). Null takes it
        /// away. Only the GPU painter draws game textures; it redraws the page every frame while one is shown.</summary>
        public void SetTexture(string name, Texture texture)
        {
            if (_textures.TryGetValue(name, out var old) && old == texture) return;
            if (texture == null) _textures.Remove(name); else _textures[name] = texture;
            _gpu?.SetTexture(name, texture);
            _texturesChanged = true;
        }

        public void SetText(string selector, string value) => Apply("t|" + selector, value, () => WebNative.vitrail_set_text(_ui, selector, value));
        public void SetAttr(string selector, string name, string value) => Apply("a|" + selector + "|" + name, value, () => WebNative.vitrail_set_attr(_ui, selector, name, value));
        public void SetClass(string selector, string cls, bool on) => Apply("c|" + selector + "|" + cls, on ? "1" : "0", () => WebNative.vitrail_set_class(_ui, selector, cls, on ? 1 : 0));
        public void SetStyle(string selector, string property, string value) => Apply("s|" + selector + "|" + property, value, () => WebNative.vitrail_set_style(_ui, selector, property, value));

        /// <summary>What was last set under each key: the HUD refreshes often with the same values, and an unchanged
        /// value must not touch the page (a touched page repaints).</summary>
        private readonly Dictionary<string, string> _values = new();

        private void Apply(string key, string value, Action op)
        {
            if (_values.TryGetValue(key, out var was) && was == value) return;
            _values[key] = value;
            _state[key] = op;
            if (_ui != IntPtr.Zero) op();
        }

        /// <summary>A screen point (Unity's, y up) on the page, in its pixels from the top-left.</summary>
        private Vector2? ToPage(Vector2 screen) => PointerToPage != null ? PointerToPage(screen) : new Vector2(screen.x - _x, Screen.height - screen.y - _y);

        /// <summary>Whether a screen point (Unity's, y up) is over a part of the page that takes the pointer.</summary>
        public bool Covers(Vector2 screen)
        {
            if (_ui == IntPtr.Zero || !Visible || Hold || Ghost) return false;
            var at = ToPage(screen);
            if (at == null) return _drag != null;
            var (x, y) = (at.Value.x, at.Value.y);
            if (ThumbAt(new Vector2(x, y)) >= 0 || _drag != null) return true;
            return x >= 0 && x < _w && y >= 0 && y < _h && WebNative.vitrail_hit(_ui, x, y) != 0;
        }

        /// <summary>The first element matching a selector, in screen pixels from the top-left; null when not found.</summary>
        public Rect? RectOf(string selector)
        {
            if (_ui == IntPtr.Zero || WebNative.vitrail_rect(_ui, selector, _rect) == 0) return null;
            return new Rect(_rect[0] + _x, _rect[1] + _y, _rect[2], _rect[3]);
        }

        /// <summary>Called every frame: keeps the page the screen's width and the HUD's scale, feeds it the mouse, repaints.</summary>
        public void Update(float scale)
        {
            if (!File.Exists(_path)) return;
            var area = Area?.Invoke(scale) ?? new RectInt(0, 0, ScreenW, _cssHeight > 0 ? Mathf.Min(ScreenH, Mathf.CeilToInt(_cssHeight * scale)) : ScreenH);
            var w = Mathf.Max(1, area.width);
            var h = Mathf.Max(1, area.height);
            _x = area.x;
            _y = area.y;
            // A rebuilt native library (editor): the old document lives in the old code, so the page starts afresh.
            if (WebNative.Refresh() || _generation != WebNative.Generation)
            {
                _gpu?.Forget();
                _ui = IntPtr.Zero;
                _generation = WebNative.Generation;
            }
            if (_ui == IntPtr.Zero || _fileChanged || w != _w || h != _h || !Mathf.Approximately(scale, _scale))
            {
                _fileChanged = false;
                Load(w, h, scale);
            }
            if (Visible && !Hold && !Ghost) { Pointer(); Keys(); Pad(); }
            // Painting runs on a worker thread; the screen keeps the last page until the new one is in.
            Take();
            Scrollbars();
            if (_gpuOn && _gpu != null)
            {
                // A texture set or replaced: the last frame is read again with it. Live textures: drawn every frame.
                if (_texturesChanged) { _texturesChanged = false; _gpu.Paint(_ui, _w, _h); }
                else if (_gpu.HasGameTextures) _gpu.Redraw();
                // Composited layers (no glass): laid together again where the library now has them.
                if (_gpu.HasLayers && !_gpu.HasBackdrop) _gpu.Assemble();
            }
            // CSS animations and transitions run on unscaled time since the page loaded (its clock starts at 0 there),
            // so a paused game still animates its interface.
            Animating = WebNative.vitrail_tick(_ui, Time.unscaledTimeAsDouble - _loadedAt) != 0;
            DrainScript();
            var t0 = Time.realtimeSinceStartup;
            if (WebNative.vitrail_needs_paint(_ui) != 0 && WebNative.vitrail_paint_start(_ui) != 0)
            {
                PaintsStarted++;
                _paintStarted = Time.realtimeSinceStartup;
                LastStartMs = (_paintStarted - t0) * 1000;
            }
            WebNative.ReportErrors();
        }

        private void Load(int w, int h, float scale)
        {
            _gpu?.Forget();
            if (_ui != IntPtr.Zero) WebNative.vitrail_destroy(_ui);
            _w = w; _h = h; _scale = scale;
            var html = File.ReadAllText(_path);
            _loadedAt = Time.unscaledTimeAsDouble;
            _ui = WebNative.vitrail_create(html, new Uri(Path.GetFullPath(_path)).AbsoluteUri, (uint)w, (uint)h, scale);
            if (UseGpu && _gpu == null)
            {
                _gpu = new WebGpuPainter();
                foreach (var t in _textures) _gpu.SetTexture(t.Key, t.Value);
                if (!_gpu.Ready) Debug.LogWarning($"Vitrail: the GPU painter is off ({_gpu.Problem}); {Path.GetFileName(_path)} is painted on the CPU instead.");
            }
            _gpuOn = UseGpu && _gpu != null && _gpu.Ready;
            WebNative.vitrail_set_gpu(_ui, _gpuOn ? 2 : 0);
            // The CPU painter's page and mask with their byte copies. The GPU painter draws into its own target and makes no
            // mask, so a GPU page keeps them at one pixel: at full size they were four page-sized buffers never written
            // (two textures, two managed arrays: ~59 MB for a 2560×1440 page, ~130 MB at 4K).
            int tw = _gpuOn ? 1 : w, th = _gpuOn ? 1 : h;
            if (_tex == null || _tex.width != tw || _tex.height != th)
            {
                if (_tex != null) UnityEngine.Object.Destroy(_tex);
                if (_mask != null) UnityEngine.Object.Destroy(_mask);
                _tex = new Texture2D(tw, th, TextureFormat.RGBA32, false, true) { name = "Vitrail page", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                _mask = new Texture2D(tw, th, TextureFormat.RGBA32, false, true) { name = "Vitrail mask", filterMode = FilterMode.Point, wrapMode = TextureWrapMode.Clamp };
                _pixels = new byte[tw * th * 4];
                _maskPixels = new byte[tw * th * 4];
            }
            _lastPointer = new Vector2(-1, -1);
            foreach (var op in _state.Values) op();
            Loaded?.Invoke();
            if (Application.isEditor) Watch();
        }

        private void Pointer()
        {
            var mouse = Mouse.current;
            if (mouse == null) return;
            var m = mouse.position.ReadValue();
            var p = ToPage(m) ?? new Vector2(-1000, -1000);
            // The overlay scrollbars' thumbs come before the page: hovered, pressed and dragged here.
            var under = Blocked?.Invoke(m) == true || CoveredAbove(m);
            _pointer = under ? new Vector2(-1, -1) : p;
            if (DevPick(mouse, p)) return;
            if (Thumbs(mouse, p)) return;
            var blocked = under || p.x < 0 || p.x >= _w || p.y < 0 || p.y >= _h;
            if (blocked && mouse.leftButton.wasReleasedThisFrame) _pressed = false;
            var at = blocked ? new Vector2(-1000, -1000) : p;
            if (at != _lastPointer)
            {
                WebNative.vitrail_pointer_move(_ui, at.x, at.y);
                _lastPointer = at;
                var hovered = WebNative.vitrail_hovered(_ui, _action, _id, (UIntPtr)_action.Length) != 0;
                var action = hovered ? WebNative.Str(_action) : "";
                var id = hovered ? WebNative.Str(_id) : "";
                if (action != _hoverAction || id != _hoverId)
                {
                    _hoverAction = action;
                    _hoverId = id;
                    HoverChanged?.Invoke(action, id);
                }
            }
            Cursor(blocked ? 0 : WebNative.vitrail_cursor(_ui));
            SmoothWheel();
            if (blocked) return;
            // The wheel scrolls what is under the pointer (a game can ask whether the page has it, so its camera doesn't zoom too),
            // glided over a few frames as browsers do, not jumped.
            var wheel = mouse.scroll.ReadValue();
            if (!OwnWheel && wheel != Vector2.zero && WebNative.vitrail_hit(_ui, p.x, p.y) != 0)
            {
                if (_wheelLeft == Vector2.zero || (_wheelAt - p).sqrMagnitude > 400f) _wheelAt = p;
                _wheelLeft += new Vector2(-wheel.x, -wheel.y) * WheelStep * _scale;
            }
            if (mouse.leftButton.wasPressedThisFrame) { WebNative.vitrail_pointer_down(_ui, p.x, p.y); _pressed = true; }
            // A release counts only for a press that began on this page: a page above that closed on the press or the
            // release (a window's ✕) must not pass the click to what lay under it.
            if (mouse.leftButton.wasReleasedThisFrame && _pressed)
            {
                _pressed = false;
                if (WebNative.vitrail_pointer_up(_ui, p.x, p.y, _action, _id, (UIntPtr)_action.Length) != 0)
                    Clicked?.Invoke(WebNative.Str(_action), WebNative.Str(_id));
            }
        }

        // ---------- Overlay scrollbars, drawn by the GPU over the page (no repaint while they show, move or fade) ----------

        private const int MaxThumbs = 8;
        private readonly Vector4[] _thumbRect = new Vector4[MaxThumbs];
        private readonly Vector4[] _thumbColor = new Vector4[MaxThumbs];
        private int _thumbCount;
        private readonly float[] _thumbRadius = new float[MaxThumbs];
        private float[] _extents = new float[6 * 16];
        /// <summary>Each container's last offset and when its thumbs last showed (scrolled, hovered, dragged).</summary>
        private readonly Dictionary<ulong, (Vector2 Offset, double Shown)> _bars = new();
        /// <summary>This frame's thumbs for hit testing: container index, axis, track and thumb boxes, scroll range.</summary>
        private readonly List<(int Index, ulong Node, bool Vertical, Vector4 Track, Vector4 Thumb, float Range, float Offset)> _bar = new();
        private (ulong Node, bool Vertical, float Grab)? _drag;
        private int _hoverThumb = -1;
        /// <summary>The pointer on the page (device px) as of this frame; the thumbs of the container under it show.</summary>
        private Vector2 _pointer = new(-1, -1);

        /// <summary>The thumbs of the GPU-scrolled containers for this frame: shown when a container scrolls or its thumb
        /// is hovered or dragged, fading out half a second later (as Blitz's and macOS's overlay scrollbars do).</summary>
        private void Scrollbars()
        {
            _thumbCount = 0;
            _bar.Clear();
            DevOverlay();
            if (!_gpuOn || _gpu == null || _gpu.ScrollCount == 0 || !Visible || HideScrollbars) return;
            var n = _gpu.ScrollCount;
            if (_extents.Length < n * 6) _extents = new float[n * 12];
            n = WebNative.vitrail_scroll_extents(_ui, _extents, (UIntPtr)n);
            float s = _scale, w = 6 * s, m = 2 * s, min = 24 * s;
            var now = Time.unscaledTimeAsDouble;
            for (var i = 0; i < n && _thumbCount < MaxThumbs; i++)
            {
                var box = _gpu.ScrollBox(i);
                var node = _gpu.ScrollNode(i);
                var e = i * 6;
                var off = new Vector2(_extents[e], _extents[e + 1]);
                float cw = _extents[e + 2], ch = _extents[e + 3], sw = _extents[e + 4], sh = _extents[e + 5];
                if (!_bars.TryGetValue(node, out var st)) st = (off, double.MinValue);
                else if (st.Offset != off) st = (off, now);
                // The pointer over the container shows its thumbs (and keeps them shown), so they can be found and caught.
                if (_pointer.x >= box.x && _pointer.x <= box.z && _pointer.y >= box.y && _pointer.y <= box.w) st = (off, now);
                var both = sh > ch + 0.5f && sw > cw + 0.5f;
                for (var a = 0; a < 2; a++)
                {
                    var vertical = a == 0;
                    float content = vertical ? sh : sw, view = vertical ? ch : cw, at = vertical ? off.y : off.x;
                    if (content <= view + 0.5f) continue;
                    var track = vertical
                        ? new Vector4(box.z - m - w, box.y + m, box.z - m, box.w - m - (both ? w + m : 0))
                        : new Vector4(box.x + m, box.w - m - w, box.z - m - (both ? w + m : 0), box.w - m);
                    var len = vertical ? track.w - track.y : track.z - track.x;
                    var thumbLen = Mathf.Max(min, len * view / content);
                    var pos = (len - thumbLen) * Mathf.Clamp01(at / (content - view));
                    var thumb = vertical ? new Vector4(track.x, track.y + pos, track.z, track.y + pos + thumbLen) : new Vector4(track.x + pos, track.y, track.x + pos + thumbLen, track.w);
                    var k = _bar.Count;
                    var dragged = _drag is { } d && d.Node == node && d.Vertical == vertical;
                    var hovered = k == _hoverThumb;
                    if (dragged || hovered) st = (off, now);
                    var t = now - st.Shown;
                    var fade = t < 0.5 ? 1f : t < 0.7 ? 1f - (float)((t - 0.5) / 0.2) : 0f;
                    if (fade <= 0 && !dragged) continue;
                    _bar.Add((i, node, vertical, track, thumb, content - view, at));
                    // Wider and brighter under the pointer, as in Chrome and macOS.
                    if (hovered || dragged) thumb = vertical ? new Vector4(thumb.x - 2 * s, thumb.y, thumb.z, thumb.w) : new Vector4(thumb.x, thumb.y - 2 * s, thumb.z, thumb.w);
                    var alpha = (dragged ? 0.85f : hovered ? 0.7f : 0.5f) * fade;
                    var c = new Color(0.94f, 0.92f, 0.86f).linear;
                    _thumbRect[_thumbCount] = thumb;
                    _thumbRadius[_thumbCount] = 0;
                    _thumbColor[_thumbCount++] = new Vector4(c.r * alpha, c.g * alpha, c.b * alpha, alpha);
                }
                _bars[node] = st;
            }
        }

        // ---------- Developer tools (VitrailInspector): an element highlighted over the page, picking one with the mouse ----------

        /// <summary>The element shown highlighted over the page (a node id from vitrail_dev_tree; 0: none).</summary>
        internal ulong DevHighlight;
        /// <summary>While set, the mouse picks elements instead of using the page: hovering highlights, a click picks.</summary>
        internal bool DevPicking;
        internal event Action<ulong> DevPicked;
        internal IntPtr Native => _ui;
        /// <summary>Every live panel (for the inspector).</summary>
        internal static IReadOnlyList<WebPanel> All => Live;
        /// <summary>The page's file name.</summary>
        public string Page => Path.GetFileName(_path);
        private readonly float[] _devRect = new float[4];

        private void DevOverlay()
        {
            if (DevHighlight == 0 || _ui == IntPtr.Zero || WebNative.vitrail_dev_rect(_ui, DevHighlight, _devRect) == 0) return;
            var c = new Color(0.26f, 0.52f, 0.96f).linear;
            const float a = 0.35f;
            _thumbRect[_thumbCount] = new Vector4(_devRect[0], _devRect[1], _devRect[0] + Mathf.Max(1, _devRect[2]), _devRect[1] + Mathf.Max(1, _devRect[3]));
            _thumbRadius[_thumbCount] = 0.001f;
            _thumbColor[_thumbCount++] = new Vector4(c.r * a, c.g * a, c.b * a, a);
        }

        /// <summary>Picking: the element under the pointer is highlighted, a click picks it. True while it takes the mouse.</summary>
        private bool DevPick(Mouse mouse, Vector2 p)
        {
            if (!DevPicking) return false;
            var inside = p.x >= 0 && p.x < _w && p.y >= 0 && p.y < _h;
            DevHighlight = inside ? WebNative.vitrail_dev_pick(_ui, p.x, p.y) : 0;
            if (inside && mouse.leftButton.wasPressedThisFrame) { DevPicking = false; DevPicked?.Invoke(DevHighlight); }
            return true;
        }

        /// <summary>The thumb (index into this frame's list) under a page point, or -1.</summary>
        private int ThumbAt(Vector2 p)
        {
            // A generous strip across the thumb (it is thin), the whole track's length: a press on the track brings the
            // thumb under the pointer.
            var g = 6 * _scale;
            for (var k = 0; k < _bar.Count; k++)
            {
                var (t, tr) = (_bar[k].Thumb, _bar[k].Track);
                var hit = _bar[k].Vertical
                    ? p.x >= tr.x - g && p.x <= tr.z + g && p.y >= tr.y && p.y <= tr.w
                    : p.y >= tr.y - g && p.y <= tr.w + g && p.x >= tr.x && p.x <= tr.z;
                if (hit) return k;
            }
            return -1;
        }

        /// <summary>Hover, press and drag of the thumbs; true when the pointer is theirs this frame (the page doesn't
        /// see it). Dragging scrolls the container (on the GPU while within the held content).</summary>
        private bool Thumbs(Mouse mouse, Vector2 p)
        {
            _hoverThumb = _drag == null ? ThumbAt(p) : _hoverThumb;
            if (_drag is { } d)
            {
                if (!mouse.leftButton.isPressed) { _drag = null; return true; }
                var k = _bar.FindIndex(b => b.Node == d.Node && b.Vertical == d.Vertical);
                if (k < 0) { _drag = null; return true; }
                var b = _bar[k];
                float len = d.Vertical ? b.Track.w - b.Track.y : b.Track.z - b.Track.x, thumbLen = d.Vertical ? b.Thumb.w - b.Thumb.y : b.Thumb.z - b.Thumb.x;
                var start = (d.Vertical ? p.y : p.x) - d.Grab - (d.Vertical ? b.Track.y : b.Track.x);
                var want = Mathf.Clamp01(start / Mathf.Max(1, len - thumbLen)) * b.Range;
                var delta = want - b.Offset;
                if (Mathf.Abs(delta) > 0.25f)
                {
                    var box = _gpu.ScrollBox(b.Index);
                    var cx = (box.x + box.z) / 2; var cy = (box.y + box.w) / 2;
                    if (WebNative.vitrail_scroll(_ui, cx, cy, d.Vertical ? 0 : delta, d.Vertical ? delta : 0) == 2 && _gpuOn) _gpu?.Rescroll(_ui);
                }
                return true;
            }
            if (_hoverThumb < 0) return false;
            Cursor(1);
            if (mouse.leftButton.wasPressedThisFrame)
            {
                var b = _bar[_hoverThumb];
                float at = b.Vertical ? p.y : p.x, from = b.Vertical ? b.Thumb.y : b.Thumb.x, to = b.Vertical ? b.Thumb.w : b.Thumb.z;
                // On the thumb: held where pressed. On the track: the thumb's middle jumps to the pointer.
                _drag = (b.Node, b.Vertical, at >= from && at <= to ? at - from : (to - from) / 2);
            }
            return true;
        }

        /// <summary>The game reads the wheel itself (and scrolls with <see cref="ScrollBy"/>): the page doesn't scroll by it.</summary>
        public bool OwnWheel;
        /// <summary>How many ScrollBy calls ended at the end (0), repainted (1) or moved on the GPU (2): a probe's count.</summary>
        public readonly int[] ScrollResults = new int[3];
        /// <summary>No overlay scrollbars over the page's scroll containers (the page draws its own).</summary>
        public bool HideScrollbars;

        /// <summary>
        /// Scrolls the scroll container under a point (CSS px from the page's top-left) by dx, dy CSS px, on the GPU while
        /// the content it holds covers the move (only the kept textures shift; past it the page repaints at the new offset).
        /// False at the end of its range or with nothing to scroll there.
        /// </summary>
        public bool ScrollBy(Vector2 at, float dx, float dy)
        {
            if (_ui == IntPtr.Zero) return false;
            var r = WebNative.vitrail_scroll(_ui, at.x * _scale, at.y * _scale, dx * _scale, dy * _scale);
            if (r == 2 && _gpuOn) _gpu?.Rescroll(_ui);
            ScrollResults[Mathf.Clamp(r, 0, 2)]++;
            return r != 0;
        }

        private Vector2 _wheelLeft, _wheelAt;

        /// <summary>Scrolls a share of what the wheel still owes each frame (about 90% in 0.1 s), at the point it turned.</summary>
        private void SmoothWheel()
        {
            if (_wheelLeft == Vector2.zero) return;
            var step = _wheelLeft * (1f - Mathf.Exp(-Time.unscaledDeltaTime * 22f));
            if (_wheelLeft.sqrMagnitude < 1f || step.sqrMagnitude < 0.25f) step = _wheelLeft;
            _wheelLeft -= step;
            // 2: the library scrolled on the GPU (the content it holds covers it): only the kept textures move.
            var r = WebNative.vitrail_scroll(_ui, _wheelAt.x, _wheelAt.y, step.x, step.y);
            if (r == 0) _wheelLeft = Vector2.zero; // at the end: the rest is dropped
            else if (r == 2 && _gpuOn) _gpu?.Rescroll(_ui);
        }

        // ---------- Gamepad: spatial navigation between the page's focusable elements ----------

        /// <summary>The panel the gamepad drives (one at a time; null: none). Its d-pad or left stick moves the focus,
        /// A presses the focused element (as a click: <see cref="Clicked"/>), B backs out (closes a list, drops the focus).</summary>
        public static WebPanel GamepadFocus;
        private int _padDir = -1;
        private float _padNext;

        private void Pad()
        {
            var pad = UnityEngine.InputSystem.Gamepad.current;
            if (pad == null || GamepadFocus != this) return;
            var v = pad.dpad.ReadValue() + pad.leftStick.ReadValue();
            var dir = v.magnitude < 0.5f ? -1 : Mathf.Abs(v.x) > Mathf.Abs(v.y) ? (v.x > 0 ? 1 : 3) : (v.y > 0 ? 0 : 2);
            var now = Time.unscaledTime;
            // A held direction repeats after 0.35 s, then every 0.12 s.
            if (dir != _padDir) { _padDir = dir; if (dir >= 0) { WebNative.vitrail_navigate(_ui, dir); _padNext = now + 0.35f; } }
            else if (dir >= 0 && now >= _padNext) { WebNative.vitrail_navigate(_ui, dir); _padNext = now + 0.12f; }
            if (pad.buttonSouth.wasPressedThisFrame && WebNative.vitrail_activate(_ui, _action, _id, (UIntPtr)_action.Length) == 1)
                Clicked?.Invoke(WebNative.Str(_action), WebNative.Str(_id));
            if (pad.buttonEast.wasPressedThisFrame) WebNative.vitrail_cancel(_ui);
        }

        private readonly byte[] _value = new byte[1024];

        /// <summary>The value of the first element matching the selector: a select's chosen option, a text field's
        /// text, "on" or "" for a checkbox; null when nothing matches.</summary>
        public string Value(string selector)
        {
            if (_ui == IntPtr.Zero) return null;
            var r = WebNative.vitrail_value(_ui, selector, _value, (UIntPtr)_value.Length);
            if (r < 0) { var big = new byte[-r]; r = WebNative.vitrail_value(_ui, selector, big, (UIntPtr)big.Length); return r == 1 ? WebNative.Str(big) : null; }
            return r == 1 ? WebNative.Str(_value) : null;
        }

        /// <summary>Chooses the option with this value in the select matching the selector (no change event).</summary>
        public bool SelectValue(string selector, string value) => _ui != IntPtr.Zero && WebNative.vitrail_select_value(_ui, selector, value) == 1;

        /// <summary>CSS px a wheel's unit scrolls: the project's Input System gives one unit a notch on every platform
        /// (ScrollDeltaBehavior.UniformAcrossAllPlatforms) and fractions on a trackpad; a browser scrolls ≈100 px a notch.</summary>
        private const float WheelStep = 100f;
        private int _cursor;

        /// <summary>The page's cursor: the game's own hand over things that take a click (CSS `cursor: pointer`).</summary>
        private void Cursor(int kind)
        {
            if (kind == _cursor) return;
            _cursor = kind;
            UnityEngine.Cursor.SetCursor(kind == 1 ? HandCursor : null, kind == 1 ? HandHotspot : Vector2.zero, CursorMode.Auto);
        }

        private float _paintStarted;
        private readonly uint[] _dirty = new uint[4];
        private readonly Dictionary<(int, int), (Texture2D Tex, byte[] Data)> _staging = new();

        /// <summary>Takes a paint the worker finished: uploads only the rectangle that changed (through a small staging
        /// texture and a GPU copy), the mask when its shapes changed, and the backdrop elements' parameters.</summary>
        private void Take()
        {
            var flags = WebNative.vitrail_paint_poll(_ui, _pixels, (UIntPtr)_pixels.Length, _dirty, _maskPixels, (UIntPtr)_maskPixels.Length, _backdrop, MaxRegions, out var regions);
            if (flags == 0) return;
            PaintsLanded++;
            if ((flags & 8) != 0)
            {
                _gpu.Paint(_ui, _w, _h);
                LastChange = new Vector2Int(_w, _h);
            }
            if ((flags & 2) != 0) Upload((int)_dirty[0], (int)_dirty[1], (int)_dirty[2], (int)_dirty[3]);
            if ((flags & 4) != 0)
            {
                _mask.SetPixelData(_maskPixels, 0);
                _mask.Apply(false, false);
            }
            _hasBackdrop = regions > 0;
            Regions((int)regions);
            LastPaintMs = (Time.realtimeSinceStartup - _paintStarted) * 1000;
        }

        /// <summary>The changed rectangle (its rows packed in _pixels) into the page texture.</summary>
        private void Upload(int x, int y, int w, int h)
        {
            if (w <= 0 || h <= 0) return;
            LastChange = new Vector2Int(w, h);
            if (w == _w && h == _h)
            {
                _tex.SetPixelData(_pixels, 0, 0);
                _tex.Apply(false, false);
                return;
            }
            // A staging texture a little bigger than the rectangle (sizes in steps of 64, so a few get reused).
            var key = ((w + 63) / 64 * 64, (h + 63) / 64 * 64);
            if (!_staging.TryGetValue(key, out var st))
            {
                if (_staging.Count > 8) { foreach (var s in _staging.Values) UnityEngine.Object.Destroy(s.Tex); _staging.Clear(); }
                st = (new Texture2D(key.Item1, key.Item2, TextureFormat.RGBA32, false, true) { name = "Vitrail staging" }, new byte[key.Item1 * key.Item2 * 4]);
                _staging[key] = st;
            }
            var stride = key.Item1 * 4;
            for (var row = 0; row < h; row++) Buffer.BlockCopy(_pixels, row * w * 4, st.Data, row * stride, w * 4);
            st.Tex.SetPixelData(st.Data, 0, 0);
            st.Tex.Apply(false, false);
            Graphics.CopyTexture(st.Tex, 0, 0, 0, 0, w, h, _tex, 0, 0, x, y);
        }

        private void Regions(int regions)
        {
            // Each backdrop element's colour matrix, and which blurred copy of the scene it reads (one per distinct blur).
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
        }

        /// <summary>The composite's inputs, set on this panel's own material: several panels draw in one frame, each with
        /// its own page.</summary>
        private void SetInputs(Material m)
        {
            // The GPU painter's page is a render texture with the page's top up; the CPU page is a texture whose first row
            // is the page's top.
            var gpuPage = _gpuOn && _gpu?.Target != null;
            m.SetTexture(TexId, gpuPage ? _gpu.Target : _tex);
            m.SetFloat(FlipId, gpuPage ? 1 : 0);
            m.SetTexture(MaskId, _mask);
            m.SetVector(BlurId, new Vector4(0, _hasBackdrop ? 1 : 0, 0, 0));
            m.SetMatrixArray(MatrixId, _matrices);
            m.SetVectorArray(OffsetId, _offsets);
            m.SetVectorArray(InfoId, _infos);
        }

        private void Watch()
        {
            var dir = Path.GetDirectoryName(_path);
            if (_watcher != null || dir == null) return;
            _watcher = new FileSystemWatcher(dir) { IncludeSubdirectories = true, NotifyFilter = NotifyFilters.LastWrite | NotifyFilters.FileName | NotifyFilters.Size };
            FileSystemEventHandler on = (_, e) =>
            {
                var ext = Path.GetExtension(e.FullPath).ToLowerInvariant();
                if (ext is ".html" or ".css" or ".svg" or ".png") _fileChanged = true;
            };
            _watcher.Changed += on;
            _watcher.Created += on;
            _watcher.Renamed += (s, e) => on(s, e);
            _watcher.EnableRaisingEvents = true;
        }

        private void Enqueue(ScriptableRenderContext context, Camera camera)
        {
            if (_tex == null || !Visible || Hold || _pass.Material == null || camera != Cam || (TargetCamera == null && camera.cameraType != CameraType.Game)) return;
            if (camera.GetUniversalAdditionalCameraData() is not { } data || data.scriptableRenderer is not { } renderer) return;
            // The page's rectangle as shares of the camera's picture (from its top-left).
            SetInputs(_pass.Material);
            float sw = Mathf.Max(1, camera.pixelWidth), sh = Mathf.Max(1, camera.pixelHeight);
            _pass.Rect = new Vector4(_x / sw, _y / sh, _w / sw, _h / sh);
            _pass.ThumbCount = _thumbCount;
            Array.Copy(_thumbRect, _pass.Thumbs, _thumbCount);
            Array.Copy(_thumbColor, _pass.ThumbColors, _thumbCount);
            Array.Copy(_thumbRadius, _pass.ThumbRadii, _thumbCount);
            renderer.EnqueuePass(_pass);
        }

        public void Dispose()
        {
            Live.Remove(this);
            RenderPipelineManager.beginCameraRendering -= Enqueue;
            if (_hooked != null) _hooked.onTextInput -= OnText;
            _watcher?.Dispose();
            _gpu?.Forget();
            if (_ui != IntPtr.Zero) WebNative.vitrail_destroy(_ui);
            _ui = IntPtr.Zero;
            if (_tex != null) UnityEngine.Object.Destroy(_tex);
            if (_mask != null) UnityEngine.Object.Destroy(_mask);
            if (_pass.Material != null) UnityEngine.Object.Destroy(_pass.Material);
            foreach (var s in _staging.Values) UnityEngine.Object.Destroy(s.Tex);
            _staging.Clear();
            _gpu?.Dispose();
        }

        /// <summary>Mixes the page over the finished camera picture (after post-processing), blurring the scene under
        /// `backdrop-filter` elements first.</summary>
        private sealed class CompositePass : ScriptableRenderPass
        {
            public readonly Material Material;
            private readonly WebPanel _panel;

            public CompositePass(WebPanel panel)
            {
                _panel = panel;
                var shader = UnityEngine.Resources.Load<Shader>("Vitrail/WebComposite");
                if (shader != null) Material = new Material(shader) { hideFlags = HideFlags.HideAndDontSave };
                renderPassEvent = RenderPassEvent.AfterRenderingPostProcessing;
                requiresIntermediateTexture = true;
                profilingSampler = new ProfilingSampler("Web HUD");
            }

            /// <summary>The page's rectangle as shares of the picture, from its top-left (x, y, width, height): all the work
            /// is limited to it.</summary>
            public Vector4 Rect = new(0, 0, 1, 1);
            /// <summary>This frame's scrollbar thumbs (page px) and colours (linear, premultiplied).</summary>
            public readonly Vector4[] Thumbs = new Vector4[MaxThumbs], ThumbColors = new Vector4[MaxThumbs];
            /// <summary>Each thumb's corner radius in px; 0: round ends (a pill).</summary>
            public readonly float[] ThumbRadii = new float[MaxThumbs];
            public int ThumbCount;

            private sealed class PaintData { public TextureHandle Scene; public WebGpuPainter Painter; public Matrix4x4 View, Proj; }
            private sealed class Data { public TextureHandle Source; public TextureHandle Scene; public TextureHandle[] Blurred; public Material Material; public int Pass; public float Sigma; public Vector4 ScaleBias; public Vector4 Texel; public Rect Viewport, Full; public Vector4 Area, PageUv; public Vector4[] Thumbs, ThumbColors; public float[] ThumbRadii; public int ThumbCount; }

            public override void RecordRenderGraph(RenderGraph graph, ContextContainer frame)
            {
                var resources = frame.Get<UniversalResourceData>();
                if (resources.isActiveTargetBackBuffer) return;
                var src = resources.activeColorTexture;
                // The scene drawn at another size than the screen (URP's render scale: supersampled or upscaled): it is
                // brought to the screen's size first and stays the camera's picture from here on, so the page lands
                // pixel for pixel. Drawn into the scaled picture, a page was resampled with it and came out blurred.
                var view = frame.Get<UniversalCameraData>();
                var scaled = graph.GetTextureDesc(src);
                int sw = view.camera.pixelWidth, sh = view.camera.pixelHeight;
                if (sw > 0 && sh > 0 && (scaled.width != sw || scaled.height != sh))
                {
                    scaled.width = sw;
                    scaled.height = sh;
                    scaled.msaaSamples = MSAASamples.None;
                    scaled.name = "Web HUD screen-size scene";
                    scaled.clearBuffer = false;
                    var screen = graph.CreateTexture(scaled);
                    Blit(graph, "Web HUD scene to screen size", src, screen, -2, 0, new Vector4(1, 1, 0, 0), Vector4.zero);
                    resources.cameraColor = screen;
                    src = screen;
                }
                var desc = graph.GetTextureDesc(src);
                // Only the page's rectangle: the copy of the scene, its sRGB version and the blurs are its size.
                var full = graph.GetTextureDesc(src);
                var px = Mathf.Clamp(Mathf.RoundToInt(Rect.x * full.width), 0, full.width - 1);
                var py = Mathf.Clamp(Mathf.RoundToInt(Rect.y * full.height), 0, full.height - 1);
                var pw = Mathf.Clamp(Mathf.RoundToInt(Rect.z * full.width), 1, full.width - px);
                var ph = Mathf.Clamp(Mathf.RoundToInt(Rect.w * full.height), 1, full.height - py);
                // In texture terms (rows bottom up): the rectangle's share and where it starts.
                var scaleBias = new Vector4(pw / (float)full.width, ph / (float)full.height, px / (float)full.width, (full.height - py - ph) / (float)full.height);
                var texel = new Vector4(1f / pw, 1f / ph, 0, 0);
                desc.width = pw;
                desc.height = ph;
                desc.name = "Web HUD scene band";
                desc.clearBuffer = false;
                var copy = graph.CreateTexture(desc);
                Blit(graph, "Web HUD copy band", src, copy, -1, 0, scaleBias, texel);
                // A GPU page with backdrop-filter is drawn here every frame: its glass blurs this frame's scene band.
                var gpu = _panel._gpuOn ? _panel._gpu : null;
                if (gpu != null && gpu.HasBackdrop)
                {
                    var cam = frame.Get<UniversalCameraData>();
                    using var pb = graph.AddUnsafePass<PaintData>("Web HUD page (GPU)", out var pd);
                    pd.Scene = copy;
                    pd.Painter = gpu;
                    pd.View = cam.GetViewMatrix();
                    pd.Proj = cam.GetProjectionMatrix();
                    pb.UseTexture(copy, AccessFlags.Read);
                    pb.AllowPassCulling(false);
                    pb.AllowGlobalStateModification(true);
                    pb.SetRenderFunc(static (PaintData d, UnsafeGraphContext ctx) =>
                    {
                        ctx.cmd.SetGlobalTexture(SceneId, d.Scene);
                        var cmd = CommandBufferHelpers.GetNativeCommandBuffer(ctx.cmd);
                        d.Painter.Record(cmd);
                        cmd.SetViewProjectionMatrices(d.View, d.Proj);
                    });
                }
                var blurred = new TextureHandle[4];
                var sigmas = _panel._sigmas;
                if (_panel._hasBackdrop && sigmas.Count > 0)
                {
                    // The band in sRGB (CSS filters work on sRGB values), then one blurred copy per distinct blur.
                    var bdesc = desc;
                    bdesc.colorFormat = GraphicsFormat.R8G8B8A8_UNorm;
                    bdesc.name = "Web HUD backdrop";
                    var srgb = graph.CreateTexture(bdesc);
                    var tmp = graph.CreateTexture(bdesc);
                    var whole = new Vector4(1, 1, 0, 0);
                    Blit(graph, "Web HUD to sRGB", copy, srgb, 1, 0, whole, texel);
                    for (var g = 0; g < sigmas.Count && g < 4; g++)
                    {
                        blurred[g] = graph.CreateTexture(bdesc);
                        Blit(graph, "Web HUD blur x", srgb, tmp, 2, sigmas[g], whole, texel);
                        Blit(graph, "Web HUD blur y", tmp, blurred[g], 3, sigmas[g], whole, texel);
                    }
                }
                using var b = graph.AddRasterRenderPass<Data>("Web HUD composite", out var d);
                d.Scene = copy;
                d.Blurred = blurred;
                d.Material = Material;
                // Only the page's rectangle is drawn (a viewport over it), not the whole frame with the rest clipped away.
                d.Viewport = new Rect(px, full.height - py - ph, pw, ph);
                d.Full = new Rect(0, 0, full.width, full.height);
                d.ScaleBias = scaleBias;
                d.Area = new Vector4(scaleBias.z, py / (float)full.height, scaleBias.x, scaleBias.y);
                // A page larger than the screen or hanging off it (a wide page slid sideways instead of scrolled inside): the
                // part of it that the clamped rectangle shows.
                float ux = Rect.x * full.width, uy = Rect.y * full.height, uw = Mathf.Max(1, Rect.z * full.width), uh = Mathf.Max(1, Rect.w * full.height);
                d.PageUv = new Vector4(pw / uw, ph / uh, (px - ux) / uw, (py - uy) / uh);
                d.ThumbCount = ThumbCount;
                d.Thumbs = Thumbs;
                d.ThumbColors = ThumbColors;
                d.ThumbRadii = ThumbRadii;
                b.UseTexture(copy, AccessFlags.Read);
                foreach (var h in blurred) if (h.IsValid()) b.UseTexture(h, AccessFlags.Read);
                // Read-write: only the band is drawn, the rest of the picture must be kept.
                b.SetRenderAttachment(src, 0, AccessFlags.ReadWrite);
                b.AllowGlobalStateModification(true);
                b.SetRenderFunc(static (Data d, RasterGraphContext ctx) =>
                {
                    ctx.cmd.SetGlobalVector(BandId, d.Area);
                    ctx.cmd.SetGlobalVector(PageUvId, d.PageUv);
                    ctx.cmd.SetGlobalTexture(SceneId, d.Scene);
                    for (var g = 0; g < 4; g++) if (d.Blurred[g].IsValid()) ctx.cmd.SetGlobalTexture(BlurredIds[g], d.Blurred[g]);
                    ctx.cmd.SetViewport(d.Viewport);
                    Blitter.BlitTexture(ctx.cmd, d.Scene, d.ScaleBias, d.Material, 0);
                    // The scrollbars' thumbs over the page, each through a viewport over its own box.
                    for (var t = 0; t < d.ThumbCount; t++)
                    {
                        var r = d.Thumbs[t];
                        var vp = new Rect(d.Viewport.x + r.x, d.Viewport.y + d.Viewport.height - r.w, r.z - r.x, r.w - r.y);
                        ctx.cmd.SetViewport(vp);
                        ctx.cmd.SetGlobalVector(ThumbSizeId, new Vector4(vp.width, vp.height, d.ThumbRadii[t], 0));
                        ctx.cmd.SetGlobalVector(ThumbColorId, d.ThumbColors[t]);
                        Blitter.BlitTexture(ctx.cmd, d.Scene, new Vector4(1, 1, 0, 0), d.Material, 4);
                    }
                    // Back to the whole picture: later passes (the final blit) would otherwise draw into the band only.
                    ctx.cmd.SetViewport(d.Full);
                });
            }

            private void Blit(RenderGraph graph, string name, TextureHandle from, TextureHandle to, int pass, float sigma, Vector4 scaleBias, Vector4 texel)
            {
                using var b = graph.AddRasterRenderPass<Data>(name, out var d);
                d.Source = from;
                d.Material = Material;
                d.Pass = pass;
                d.Sigma = sigma;
                d.ScaleBias = scaleBias;
                d.Texel = texel;
                b.UseTexture(from, AccessFlags.Read);
                b.SetRenderAttachment(to, 0, AccessFlags.Write);
                b.AllowGlobalStateModification(true);
                b.SetRenderFunc(static (Data d, RasterGraphContext ctx) =>
                {
                    ctx.cmd.SetGlobalFloat(SigmaId, d.Sigma);
                    ctx.cmd.SetGlobalVector(TexelId, d.Texel);
                    // −1: a copy texel for texel; −2: a resize, smoothed.
                    if (d.Pass < 0) Blitter.BlitTexture(ctx.cmd, d.Source, d.ScaleBias, 0, d.Pass == -2);
                    else Blitter.BlitTexture(ctx.cmd, d.Source, d.ScaleBias, d.Material, d.Pass);
                });
            }
        }
    }
}
