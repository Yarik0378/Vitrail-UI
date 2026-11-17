using UnityEngine;
using Vitrail;

namespace Vitrail.Samples
{
    /// <summary>The inventory sample: a window of 300 cells (scrolled on the GPU), items dragged between slots by the
    /// page's script; moves arrive as engine.trigger events. Put it on a GameObject with a VitrailScreen.</summary>
    [RequireComponent(typeof(VitrailScreen))]
    public sealed class InventorySample : MonoBehaviour
    {
        private void Awake()
        {
            var screen = GetComponent<VitrailScreen>();
            screen.page = "ui/inventory/inventory.html";
            screen.placement = VitrailScreen.Placement.FullScreen;
            screen.onTrigger.AddListener((name, json) => { if (name == "moved") Debug.Log("Moved " + json); });
            if (screen.Panel != null) screen.Reload();
        }
    }
}
