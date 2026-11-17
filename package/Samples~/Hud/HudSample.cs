using System;
using UnityEngine;
using Vitrail;

namespace Vitrail.Samples
{
    /// <summary>The HUD sample: a top bar bound to a C# model (data-bind-* in hud.html, no page script). Put it on a
    /// GameObject with a VitrailScreen; copy the sample's StreamingAssets folder into Assets/StreamingAssets.</summary>
    [RequireComponent(typeof(VitrailScreen))]
    public sealed class HudSample : MonoBehaviour
    {
        [Serializable] public sealed class Resource { public string name; public string color; public int amount, delta, cap; }
        [Serializable] public sealed class Model { public int turn = 1; public Resource[] resources; }

        public Model hud = new()
        {
            resources = new[]
            {
                new Resource { name = "Food", color = "#d9a441", amount = 42, delta = 6, cap = 120 },
                new Resource { name = "Wood", color = "#8b6b43", amount = 18, delta = -2, cap = 80 },
                new Resource { name = "Stone", color = "#9aa3a6", amount = 7, delta = 1, cap = 60 },
            },
        };

        private VitrailScreen _screen;

        private void Awake()
        {
            _screen = GetComponent<VitrailScreen>();
            _screen.page = "ui/hud/hud.html";
            _screen.placement = VitrailScreen.Placement.TopBand;
            _screen.bandHeight = 90;
            _screen.onClick.AddListener((action, _) => { if (action == "end-turn") EndTurn(); });
            if (_screen.Panel != null) _screen.Reload();
        }

        private void Start() => Push();

        private void EndTurn()
        {
            hud.turn++;
            foreach (var r in hud.resources) r.amount = Mathf.Clamp(r.amount + r.delta, 0, r.cap);
            Push();
        }

        /// <summary>Sends the whole model; the page writes only the values that changed.</summary>
        private void Push() => _screen.Panel?.SetModel("hud", JsonUtility.ToJson(hud));
    }
}
