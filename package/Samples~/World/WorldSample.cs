using UnityEngine;
using Vitrail;

namespace Vitrail.Samples
{
    /// <summary>A page on a quad in the scene: GameObject > UI > Vitrail UI World Panel makes one; this sets its page and
    /// listens to it. The mouse reaches it through a raycast from the main camera.</summary>
    [RequireComponent(typeof(VitrailWorld))]
    public sealed class WorldSample : MonoBehaviour
    {
        private void Awake()
        {
            var world = GetComponent<VitrailWorld>();
            world.page = "ui/world/terminal.html";
            world.size = new Vector2Int(640, 480);
            world.onClick.AddListener((action, _) => Debug.Log("Terminal: " + action));
        }
    }
}
