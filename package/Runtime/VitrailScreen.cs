using System;
using UnityEngine;
using UnityEngine.Events;

namespace Vitrail
{
    /// <summary>
    /// An HTML page over the screen (a HUD, a menu, a window): add it to any GameObject, pick a page under
    /// StreamingAssets and where it sits. It owns a <see cref="WebPanel"/> (<see cref="Panel"/>) for everything else:
    /// setting text and classes from C#, calling the page's scripts, game textures in &lt;img&gt;.
    /// </summary>
    [AddComponentMenu("UI/Vitrail UI Screen")]
    [DisallowMultipleComponent]
    public sealed class VitrailScreen : MonoBehaviour
    {
        public enum Placement
        {
            /// <summary>The whole screen (a menu, a window); only painted parts take the pointer.</summary>
            FullScreen,
            /// <summary>A band across the top, <see cref="bandHeight"/> CSS px tall (a top bar).</summary>
            TopBand,
            /// <summary>A rectangle given as shares of the screen from its top-left (<see cref="rect"/>).</summary>
            Rectangle,
        }

        [Tooltip("The page, a path under StreamingAssets (e.g. ui/menu.html). Pages next to it (CSS, scripts, images) load relative to it.")]
        public string page = "ui/index.html";
        public Placement placement = Placement.FullScreen;
        [Tooltip("Top band only: its height in CSS px.")]
        public float bandHeight = 80;
        [Tooltip("Rectangle only: x, y, width, height as shares of the screen, from its top-left.")]
        public Rect rect = new(0, 0, 1, 1);
        [Tooltip("CSS px per screen pixel at the reference height; 1 = one CSS px per pixel.")]
        public float scale = 1;
        [Tooltip("Scales the page with the screen: at this screen height the scale is as set; 0 keeps it constant.")]
        public float referenceHeight = 1080;
        [Tooltip("The gamepad drives this page (d-pad or stick moves the focus, A presses, B backs out).")]
        public bool gamepad;
        [Tooltip("The camera whose picture the page is drawn over; empty: the main camera.")]
        public Camera targetCamera;

        [Serializable] public sealed class StringPairEvent : UnityEvent<string, string> { }
        [Tooltip("A click on an element with a data-action (or an id): the action and the id.")]
        public StringPairEvent onClick = new();
        [Tooltip("engine.trigger(name, ...args) from the page's scripts: the name and the arguments as a JSON array.")]
        public StringPairEvent onTrigger = new();

        /// <summary>The page; null while the component is disabled.</summary>
        public WebPanel Panel { get; private set; }

        /// <summary>The scale the page is drawn at now (screen pixels per CSS px).</summary>
        public float Scale => scale * (referenceHeight > 0 ? Screen.height / referenceHeight : 1f);

        private void OnEnable() => Open();
        private void OnDisable() => Close();

        /// <summary>Loads the page again (after changing <see cref="page"/> from code).</summary>
        public void Reload() { Close(); Open(); }

        private void Open()
        {
            if (string.IsNullOrEmpty(page)) return;
            Panel = new WebPanel(page, placement == Placement.TopBand ? bandHeight : 0) { TargetCamera = targetCamera };
            if (placement == Placement.Rectangle)
                Panel.Area = _ => new RectInt(Mathf.RoundToInt(rect.x * Screen.width), Mathf.RoundToInt(rect.y * Screen.height),
                    Mathf.Max(1, Mathf.RoundToInt(rect.width * Screen.width)), Mathf.Max(1, Mathf.RoundToInt(rect.height * Screen.height)));
            Panel.Clicked += (a, i) => onClick.Invoke(a, i);
            Panel.Triggered += (n, j) => onTrigger.Invoke(n, j);
        }

        private void Close()
        {
            if (Panel == null) return;
            if (WebPanel.GamepadFocus == Panel) WebPanel.GamepadFocus = null;
            Panel.Dispose();
            Panel = null;
        }

        private void Update()
        {
            if (Panel == null) return;
            if (gamepad) WebPanel.GamepadFocus = Panel;
            else if (WebPanel.GamepadFocus == Panel) WebPanel.GamepadFocus = null;
            Panel.Update(Scale);
        }
    }
}
