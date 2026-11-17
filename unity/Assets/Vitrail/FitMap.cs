using UnityEngine;

namespace VitrailLab
{
    /// <summary>Keeps the map picture covering the camera's view, like CSS "background-size: cover".</summary>
    [ExecuteAlways]
    public sealed class FitMap : MonoBehaviour
    {
        private void Update()
        {
            var cam = Camera.main != null ? Camera.main : FindFirstObjectByType<Camera>();
            var tex = GetComponent<MeshRenderer>().sharedMaterial.mainTexture;
            if (cam == null || tex == null) return;
            float viewH = cam.orthographicSize * 2, viewW = viewH * cam.aspect, ta = tex.width / (float)tex.height;
            transform.localScale = viewW / viewH > ta ? new Vector3(viewW, viewW / ta, 1) : new Vector3(viewH * ta, viewH, 1);
        }
    }
}
