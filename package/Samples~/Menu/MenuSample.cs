using UnityEngine;
using Vitrail;

namespace Vitrail.Samples
{
    /// <summary>The main menu sample: put it on a GameObject with a VitrailScreen showing ui/menu/menu.html (copy the
    /// sample's StreamingAssets folder into Assets/StreamingAssets). Clicks and the page's events arrive here.</summary>
    [RequireComponent(typeof(VitrailScreen))]
    public sealed class MenuSample : MonoBehaviour
    {
        private void Awake()
        {
            var screen = GetComponent<VitrailScreen>();
            screen.page = "ui/menu/menu.html";
            screen.placement = VitrailScreen.Placement.FullScreen;
            screen.gamepad = true;
            screen.onClick.AddListener((action, id) =>
            {
                Debug.Log($"Menu: {action}");
                if (action == "quit") Application.Quit();
            });
            screen.onTrigger.AddListener((name, json) => Debug.Log($"Menu event {name}: {json}"));
            if (screen.Panel != null) screen.Reload();
        }

        private void Start()
        {
            // The game reads a form's value at any time.
            Debug.Log("Resolution in the menu: " + GetComponent<VitrailScreen>().Panel?.Value("#resolution"));
        }
    }
}
