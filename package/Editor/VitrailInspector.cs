using System;
using System.Collections.Generic;
using System.Linq;
using UnityEditor;
using UnityEngine;

namespace Vitrail.Editor
{
    /// <summary>
    /// The developer tools (Window > Vitrail UI > Inspector): a live page's element tree, the chosen element's box,
    /// attributes and computed styles (editable), the element highlighted over the game, and picking an element with
    /// the mouse in the Game view.
    /// </summary>
    public sealed class VitrailInspector : EditorWindow
    {
        private WebPanel _panel;
        private readonly List<(ulong Id, int Depth, bool Element, string Label)> _tree = new();
        private readonly HashSet<ulong> _closed = new();
        private ulong _selected;
        private readonly List<(string Kind, string Name, string Value)> _info = new();
        private string _filter = "";
        private Vector2 _treeScroll, _infoScroll;
        /// <summary>The tree's share of the window's width; the bar between the two panes drags it.</summary>
        [SerializeField] private float _split = 0.4f;
        private bool _dragging;
        private readonly Dictionary<string, string> _edits = new();
        private byte[] _buf = new byte[1 << 16];

        [MenuItem("Window/Vitrail UI/Inspector")]
        public static void ShowWindow() => Get();

        public static void Open(WebPanel panel) => Get().Select(panel);

        /// <summary>The window, opened as a tab beside Unity's own Inspector when there is one.</summary>
        private static VitrailInspector Get()
        {
            var inspector = Type.GetType("UnityEditor.InspectorWindow,UnityEditor");
            return inspector != null
                ? GetWindow<VitrailInspector>("Vitrail UI Inspector", true, inspector)
                : GetWindow<VitrailInspector>("Vitrail UI Inspector");
        }

        private void Select(WebPanel panel)
        {
            if (_panel != null) { _panel.DevHighlight = 0; _panel.DevPicking = false; _panel.DevPicked -= Picked; }
            _panel = panel;
            _selected = 0;
            _info.Clear();
            if (_panel != null) _panel.DevPicked += Picked;
            ReadTree();
        }

        private void OnDisable() { if (_panel != null) { _panel.DevHighlight = 0; _panel.DevPicking = false; _panel.DevPicked -= Picked; } }

        private void Picked(ulong id)
        {
            ReadTree();
            Choose(id);
            // Open the way down to it.
            var at = _tree.FindIndex(t => t.Id == id);
            if (at >= 0) for (int i = at - 1, d = _tree[at].Depth; i >= 0 && d > 0; i--) if (_tree[i].Depth < d) { _closed.Remove(_tree[i].Id); d = _tree[i].Depth; }
            Repaint();
        }

        private string Read(Func<byte[], int> f)
        {
            var r = f(_buf);
            if (r < 0) { _buf = new byte[-r + 1024]; r = f(_buf); }
            return r == 1 ? WebNative.Str(_buf) : "";
        }

        private void ReadTree()
        {
            _tree.Clear();
            if (_panel == null || _panel.Native == IntPtr.Zero) return;
            foreach (var line in Read(b => WebNative.vitrail_dev_tree(_panel.Native, b, (UIntPtr)b.Length)).Split('\n'))
            {
                var p = line.Split('\t');
                if (p.Length < 4) continue;
                _tree.Add((ulong.Parse(p[0]), int.Parse(p[1]), p[2] == "e", p[3]));
            }
        }

        private void Choose(ulong id)
        {
            _selected = id;
            if (_panel != null) _panel.DevHighlight = id;
            ReadInfo();
        }

        private void ReadInfo()
        {
            _info.Clear();
            _edits.Clear();
            if (_panel == null || _selected == 0) return;
            foreach (var line in Read(b => WebNative.vitrail_dev_node(_panel.Native, _selected, b, (UIntPtr)b.Length)).Split('\n'))
            {
                var p = line.Split('\t');
                if (p.Length == 2) _info.Add((p[0], p[0], p[1]));
                else if (p.Length >= 3) _info.Add((p[0], p[1], p[2]));
            }
        }

        private void OnInspectorUpdate() { if (Application.isPlaying) Repaint(); }

        private void OnGUI()
        {
            if (!Application.isPlaying) { EditorGUILayout.HelpBox("Enter play mode: the inspector shows live pages.", MessageType.Info); return; }
            var panels = WebPanel.All.ToArray();
            if (_panel != null && !panels.Contains(_panel)) Select(null);
            if (_panel == null && panels.Length > 0) Select(panels[0]);
            using (new EditorGUILayout.HorizontalScope(EditorStyles.toolbar))
            {
                var names = panels.Select(p => p.Page).ToArray();
                var i = EditorGUILayout.Popup(Array.IndexOf(panels, _panel), names, EditorStyles.toolbarPopup, GUILayout.Width(180));
                if (i >= 0 && panels[i] != _panel) Select(panels[i]);
                if (_panel != null)
                {
                    var picking = GUILayout.Toggle(_panel.DevPicking, "Pick", EditorStyles.toolbarButton, GUILayout.Width(50));
                    if (picking != _panel.DevPicking) _panel.DevPicking = picking;
                    if (GUILayout.Button("Refresh", EditorStyles.toolbarButton, GUILayout.Width(60))) { ReadTree(); ReadInfo(); }
                }
                GUILayout.FlexibleSpace();
            }
            if (_panel == null) { EditorGUILayout.HelpBox("No web page is open.", MessageType.Info); return; }
            EditorGUILayout.LabelField(_panel.Stats, EditorStyles.miniLabel);
            using (new EditorGUILayout.HorizontalScope())
            {
                var treeWidth = Mathf.Round(position.width * _split);
                using (var s = new EditorGUILayout.ScrollViewScope(_treeScroll, GUILayout.Width(treeWidth)))
                {
                    _treeScroll = s.scrollPosition;
                    DrawTree();
                }
                Splitter();
                using (var s = new EditorGUILayout.ScrollViewScope(_infoScroll))
                {
                    _infoScroll = s.scrollPosition;
                    // Names take a little under half of the pane, values the rest.
                    EditorGUIUtility.labelWidth = Mathf.Clamp((position.width - treeWidth) * 0.45f, 90, 320);
                    DrawInfo();
                    EditorGUIUtility.labelWidth = 0;
                }
            }
        }

        /// <summary>The bar between the tree and the element's details: drag it to share the width.</summary>
        private void Splitter()
        {
            var r = GUILayoutUtility.GetRect(5, 5, GUILayout.ExpandHeight(true));
            EditorGUI.DrawRect(new Rect(r.x + 2, r.y, 1, r.height), new Color(0, 0, 0, 0.35f));
            var grab = new Rect(r.x - 3, r.y, r.width + 6, r.height);
            EditorGUIUtility.AddCursorRect(grab, MouseCursor.ResizeHorizontal);
            var e = Event.current;
            if (e.type == EventType.MouseDown && grab.Contains(e.mousePosition)) { _dragging = true; e.Use(); }
            else if (_dragging && e.type == EventType.MouseDrag) { _split = Mathf.Clamp(e.mousePosition.x / Mathf.Max(1, position.width), 0.15f, 0.85f); e.Use(); Repaint(); }
            else if (_dragging && e.rawType == EventType.MouseUp) { _dragging = false; e.Use(); }
        }

        private void DrawTree()
        {
            var hideBelow = int.MaxValue;
            for (var i = 0; i < _tree.Count; i++)
            {
                var (id, depth, element, label) = _tree[i];
                if (depth > hideBelow) continue;
                hideBelow = int.MaxValue;
                var hasKids = i + 1 < _tree.Count && _tree[i + 1].Depth > depth;
                var closed = _closed.Contains(id);
                if (closed) hideBelow = depth;
                using (new EditorGUILayout.HorizontalScope())
                {
                    GUILayout.Space(depth * 12);
                    if (hasKids)
                    {
                        if (GUILayout.Button(closed ? "▸" : "▾", EditorStyles.label, GUILayout.Width(12))) { if (closed) _closed.Remove(id); else _closed.Add(id); }
                    }
                    else GUILayout.Space(16);
                    var style = id == _selected ? EditorStyles.whiteBoldLabel : element ? EditorStyles.label : EditorStyles.miniLabel;
                    if (GUILayout.Button(element ? label : "\"" + label + "\"", style) && element) Choose(id);
                }
            }
        }

        private void DrawInfo()
        {
            if (_selected == 0) { EditorGUILayout.LabelField("Choose an element in the tree, or Pick one in the Game view."); return; }
            foreach (var (kind, name, value) in _info.Where(x => x.Kind is "rect" or "margin" or "border" or "padding"))
                EditorGUILayout.LabelField(kind == "rect" ? "box (device px)" : kind + " (CSS px)", value);
            EditorGUILayout.Space();
            EditorGUILayout.LabelField("Attributes", EditorStyles.boldLabel);
            foreach (var (_, name, value) in _info.Where(x => x.Kind == "attr")) EditorGUILayout.LabelField(name, value);
            EditorGUILayout.Space();
            EditorGUILayout.LabelField("Computed styles (edits go into the element's style)", EditorStyles.boldLabel);
            _filter = EditorGUILayout.TextField("Filter", _filter);
            foreach (var (_, name, value) in _info.Where(x => x.Kind == "style"))
            {
                if (_filter.Length > 0 && !name.Contains(_filter)) continue;
                var shown = _edits.TryGetValue(name, out var e) ? e : value;
                EditorGUI.BeginChangeCheck();
                var now = EditorGUILayout.DelayedTextField(name, shown);
                if (EditorGUI.EndChangeCheck())
                {
                    _edits[name] = now;
                    WebNative.vitrail_dev_set_style(_panel.Native, _selected, name, now);
                }
            }
        }
    }
}
