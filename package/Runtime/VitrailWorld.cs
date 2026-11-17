using System;
using UnityEngine;
using UnityEngine.Rendering;
using UnityEngine.Rendering.Universal;

namespace Vitrail
{
    /// <summary>
    /// An HTML page on a surface in the scene (a terminal, a sign, a shop counter): the page is drawn into a
    /// RenderTexture by a camera of its own that renders nothing else, and the texture is shown on this object's
    /// renderer. The pointer reaches it through a raycast from the main camera into this object's collider (a
    /// MeshCollider gives texture coordinates; a quad with one is what the menu item makes).
    /// backdrop-filter has no scene behind it here: it blurs only the page itself.
    /// </summary>
    [AddComponentMenu("UI/Vitrail UI World Panel")]
    [RequireComponent(typeof(Renderer))]
    public sealed class VitrailWorld : MonoBehaviour
    {
        [Tooltip("The page, a path under StreamingAssets.")]
        public string page = "ui/index.html";
        [Tooltip("The page's size in CSS px.")]
        public Vector2Int size = new(800, 600);
        [Tooltip("Texture pixels per CSS px (2 is sharp up close).")]
        public float pixelRatio = 2;
        [Tooltip("The camera the pointer is cast from; empty: the main camera.")]
        public Camera pointerCamera;
        [Tooltip("The gamepad drives this page.")]
        public bool gamepad;
        [Tooltip("Writes depth like an opaque surface, so effects that read the depth buffer (fog, outlines, decals) see the panel. Off for see-through pages.")]
        public bool writeDepth = true;
        public VitrailScreen.StringPairEvent onClick = new();
        public VitrailScreen.StringPairEvent onTrigger = new();

        public WebPanel Panel { get; private set; }
        public RenderTexture Texture { get; private set; }
        private Camera _camera;
        private Material _material;
        private Collider _collider;

        private void OnEnable()
        {
            var w = Mathf.Max(1, Mathf.RoundToInt(size.x * pixelRatio));
            var h = Mathf.Max(1, Mathf.RoundToInt(size.y * pixelRatio));
            Texture = new RenderTexture(w, h, 0, RenderTextureFormat.ARGB32, 0) { name = "Vitrail UI " + page, useMipMap = true, autoGenerateMips = true };
            var go = new GameObject("Vitrail UI camera") { hideFlags = HideFlags.HideAndDontSave };
            go.transform.SetParent(transform, false);
            _camera = go.AddComponent<Camera>();
            _camera.cullingMask = 0;
            _camera.clearFlags = CameraClearFlags.SolidColor;
            _camera.backgroundColor = Color.clear;
            _camera.targetTexture = Texture;
            _camera.orthographic = true;
            _camera.depth = -100;
            var data = go.AddComponent<UniversalAdditionalCameraData>();
            data.renderPostProcessing = false;
            data.renderShadows = false;
            _collider = GetComponent<Collider>();
            Panel = new WebPanel(page, 0) { TargetCamera = _camera, PointerToPage = ToPage };
            Panel.Clicked += (a, i) => onClick.Invoke(a, i);
            Panel.Triggered += (n, j) => onTrigger.Invoke(n, j);
            var shader = Shader.Find("Universal Render Pipeline/Unlit");
            _material = new Material(shader) { mainTexture = Texture, hideFlags = HideFlags.HideAndDontSave };
            // Transparent, premultiplied: the page's own alpha shows the world behind it.
            _material.SetFloat("_Surface", 1);
            _material.SetFloat("_Blend", 1); // premultiplied
            _material.EnableKeyword("_ALPHAPREMULTIPLY_ON");
            _material.SetFloat("_SrcBlend", (float)BlendMode.One);
            _material.SetFloat("_DstBlend", (float)BlendMode.OneMinusSrcAlpha);
            _material.SetFloat("_ZWrite", writeDepth ? 1 : 0);
            _material.EnableKeyword("_SURFACE_TYPE_TRANSPARENT");
            _material.renderQueue = (int)RenderQueue.Transparent;
            GetComponent<Renderer>().sharedMaterial = _material;
        }

        private void OnDisable()
        {
            if (WebPanel.GamepadFocus == Panel) WebPanel.GamepadFocus = null;
            Panel?.Dispose();
            Panel = null;
            if (_camera != null) Destroy(_camera.gameObject);
            if (Texture != null) { Texture.Release(); Destroy(Texture); }
            if (_material != null) Destroy(_material);
        }

        /// <summary>The mouse's screen point on the page (texture pixels from its top-left), through a raycast.</summary>
        private Vector2? ToPage(Vector2 screen)
        {
            var cam = pointerCamera != null ? pointerCamera : Camera.main;
            // Looked up again if it went (a collider replaced after this was enabled).
            if (_collider == null) _collider = GetComponent<Collider>();
            if (cam == null || _collider == null || Texture == null) return null;
            if (!_collider.Raycast(cam.ScreenPointToRay(screen), out var hit, cam.farClipPlane)) return null;
            var uv = hit.textureCoord;
            return new Vector2(uv.x * Texture.width, (1 - uv.y) * Texture.height);
        }

        private void Update()
        {
            if (Panel == null) return;
            if (gamepad) WebPanel.GamepadFocus = Panel;
            else if (WebPanel.GamepadFocus == Panel) WebPanel.GamepadFocus = null;
            Panel.Update(pixelRatio);
        }
    }
}
