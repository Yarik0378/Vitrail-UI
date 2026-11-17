using UnityEngine;

namespace VitrailLab
{
    /// <summary>
    /// Plays the game's part for the live page (tests/pages/14-live.html): changes the resources every moment, moves
    /// the advisors' progress, and answers clicks — all through the panel's API, as the game will.
    /// </summary>
    public sealed class LabDemo : MonoBehaviour
    {
        private static readonly string[] Keys = { "water", "food", "wood", "stone", "planks", "iron", "horses" };
        private static readonly int[] Caps = { 40, 46, 52, 30, 24, 8, 3 };
        private VitrailPanel _panel;
        private float _next;
        private int _turn;

        private void Start()
        {
            _panel = GetComponent<VitrailPanel>();
            _panel.Clicked += (action, id) =>
                _panel.SetText("#log", $"Клік: {action} · {id}   (перемальовка {_panel.LastPaintMs:F1} мс)");
        }

        // The frame's outline as drawn for 1422 px; its right swell follows the right edge of the screen.
        private static readonly (string Id, string D)[] FramePaths =
        {
            ("#p-body", "M0 0H1422V68H1236Q1220 68 1214 60Q1206 50 1188 50H262Q246 50 240 62Q232 76 214 76H0Z"),
            ("#p-top", "M0 1.5H1422"),
            ("#p-engr", "M0 70H212Q228 70 235 58Q241 46 258 46H1184Q1202 46 1209 56Q1215 64 1232 64H1422"),
            ("#p-light", "M0 71H212Q228 71 235 59Q241 47 258 47H1184Q1202 47 1209 57Q1215 65 1232 65H1422"),
            ("#p-accent", "M0 74H214Q232 74 238 62Q245 49 262 49H1188Q1206 49 1213 59Q1219 67 1236 67H1422"),
            ("#p-edge", "M0 76H214Q232 76 240 62Q246 50 262 50H1188Q1206 50 1214 60Q1220 68 1236 68H1422"),
        };
        private float _frameWidth;

        /// <summary>Every x of the right part (≥ 1100 in the 1422-wide drawing) moves by dx; the left part stays.</summary>
        private static string Shift(string d, float dx)
        {
            var sb = new System.Text.StringBuilder();
            var num = new System.Text.StringBuilder();
            void Flush()
            {
                if (num.Length == 0) return;
                var v = float.Parse(num.ToString(), System.Globalization.CultureInfo.InvariantCulture);
                sb.Append((v >= 1100 ? v + dx : v).ToString("0.##", System.Globalization.CultureInfo.InvariantCulture));
                num.Clear();
            }
            foreach (var c in d)
            {
                if (char.IsDigit(c) || c == '.') num.Append(c);
                else { Flush(); sb.Append(c); }
            }
            Flush();
            return sb.ToString();
        }

        private void FitFrame()
        {
            var w = Mathf.Round(_panel.CssWidth);
            if (Mathf.Approximately(w, _frameWidth)) return;
            _frameWidth = w;
            var dx = w - 1422;
            _panel.SetAttr("#frame", "viewBox", $"0 0 {w} 80");
            foreach (var (id, d) in FramePaths) _panel.SetAttr(id, "d", Shift(d, dx));
            _panel.SetStyle("#frame-blur", "clip-path", $"path('{Shift(FramePaths[0].D, dx)}')");
        }

        private void Update()
        {
            FitFrame();
            if (Time.time < _next) return;
            _next = Time.time + 0.8f;
            _turn++;
            for (var i = 0; i < Keys.Length; i++)
            {
                var v = Random.Range(0, Caps[i] + 1);
                var d = Random.Range(-20, 10);
                _panel.SetText("#" + Keys[i], $"{v}/{Caps[i]}");
                _panel.SetText("#" + Keys[i] + "-d", d > 0 ? "+" + d : d < 0 ? "−" + -d : "0");
                _panel.SetAttr("#" + Keys[i] + "-d", "class", d > 0 ? "up" : d < 0 ? "dn" : "z");
            }
            _panel.SetText("#science", (20 + _turn).ToString());
            _panel.SetStyle("#ring-science .p", "--p", $"{_turn * 7 % 100}%");
            _panel.SetStyle("#ring-food .p", "--p", $"{(_turn * 13 + 40) % 100}%");
            _panel.SetClass("#ring-units", "quiet", _turn % 2 == 0);
        }

        /// <summary>In the lab scene, the live page and this demo take over the panel when play starts.</summary>
        [RuntimeInitializeOnLoadMethod(RuntimeInitializeLoadType.AfterSceneLoad)]
        private static void Hook()
        {
            var panel = FindFirstObjectByType<VitrailPanel>();
            if (panel == null || panel.GetComponent<LabDemo>() != null) return;
            if (!string.IsNullOrEmpty(panel.HtmlPath) && panel.HtmlPath.EndsWith("13-panel.html"))
                panel.HtmlPath = panel.HtmlPath.Replace("13-panel.html", "14-live.html");
            panel.gameObject.AddComponent<LabDemo>();
        }
    }
}
