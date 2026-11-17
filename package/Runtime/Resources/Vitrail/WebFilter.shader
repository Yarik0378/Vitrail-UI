// CSS `filter` on a group's texture (values are premultiplied sRGB, as everywhere in the GPU painter). Each pass draws
// a quad over the group's bounds (_Bounds, device px, through the painter's own view-projection) and reads its input at
// the same pixel; reads are clamped to the bounds, so what lies outside them is never seen.
// Passes: 0 colour matrix (feColorMatrix on unpremultiplied colour), 1 one direction of a Gaussian blur, 2 a shadow's
// seed (the input's alpha moved by _Offset, in _ShadowColor), 3 the input over the shadow, 4 the input over the scene
// (backdrop-filter's source), 5 the input drawn over the target (a still layer of the page), 6 a composited layer moved and faded, 7 clears the bounds.
Shader "Hidden/Vitrail/WebFilter"
{
    HLSLINCLUDE
    #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"
    #include "Packages/com.unity.render-pipelines.core/ShaderLibrary/Color.hlsl"
    TEXTURE2D(_VitrailScene);   // the game's picture under the page (linear; its rows bottom up), set by the panel
    TEXTURE2D(_Src); TEXTURE2D(_Shadow); SAMPLER(sampler_linear_clamp);
    float4 _TargetSize, _Bounds, _PageSize;   // _PageSize: the whole page in device px (the scene band's size)
    float4 _M0, _M1, _M2, _M3, _MOff;
    float4 _BlurDir;      // xy: one pixel along the blur's axis; z: σ
    float4 _Offset;       // xy: the shadow's offset in px
    float4 _ShadowColor;
    float4 _SrcRect;      // a piece's texture holds this page rectangle (x, y, w, h): Over and Layer read it there
    // A page pixel of a piece texture (rows bottom up, as Unity samples render textures).
    half4 ReadPiece(float2 px)
    {
        float2 uv = (px - _SrcRect.xy) / _SrcRect.zw;
        return SAMPLE_TEXTURE2D_LOD(_Src, sampler_linear_clamp, float2(uv.x, 1 - uv.y), 0);
    }
    // pos: where the pixel lands in the texture; page: the same pixel in page px (y down). Rendering into a texture
    // may flip y, so reads go from page px back to the texture through the per-pixel direction `dir`.
    struct V { float4 pos : SV_POSITION; float2 page : TEXCOORD0; };
    V Vert(uint id : SV_VertexID)
    {
        // Two triangles over the bounds.
        uint k = id % 6;
        float2 c = float2(k == 1 || k == 2 || k == 4 ? 1 : 0, k == 2 || k == 4 || k == 5 ? 1 : 0);
        V v;
        v.page = lerp(_Bounds.xy, _Bounds.zw, c);
        v.pos = mul(UNITY_MATRIX_VP, float4(v.page, 0, 1));
        return v;
    }
    struct Here { float2 pos, page, dir; };
    // dir: texture pixels per page pixel along each axis, with its sign (a texture may be smaller than the page: glass
    // blurs at half size, and it may run upside down).
    Here At(V v) { Here h; h.pos = v.pos.xy; h.page = v.page; h.dir = float2(1 / ddx(v.page.x), 1 / ddy(v.page.y)); return h; }
    // The input at page px `px` (clamped to the bounds).
    half4 Read(TEXTURE2D_PARAM(t, s), Here h, float2 px)
    {
        px = clamp(px, _Bounds.xy + 0.5, _Bounds.zw - 0.5);
        return SAMPLE_TEXTURE2D_LOD(t, s, (h.pos + (px - h.page) * h.dir) / _TargetSize.xy, 0);
    }
    ENDHLSL

    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" }
        ZTest Always ZWrite Off Cull Off Blend Off
        Pass { Name "Matrix" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                Here h = At(v);
                float4 c = Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, h.page);
                float4 u = float4(c.a > 0 ? c.rgb / c.a : 0, c.a);
                float4 o = saturate(float4(dot(_M0, u), dot(_M1, u), dot(_M2, u), dot(_M3, u)) + _MOff);
                return float4(o.rgb * o.a, o.a);
            }
            ENDHLSL }
        Pass { Name "Blur" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                Here h = At(v);
                // Taps one texture pixel apart (a half-size texture blurs with half as many), σ in page px.
                float s = abs(h.dir.x);
                float sigma = _BlurDir.z * s;
                int r = (int)min(ceil(sigma * 3), 96);
                float k = -0.5 / (sigma * sigma);
                float4 sum = Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, h.page);
                float total = 1;
                // Pairs of taps read between two pixels with their combined weight (bilinear does the mixing).
                for (int i = 1; i <= r; i += 2)
                {
                    float w0 = exp(k * i * i), w1 = i + 1 <= r ? exp(k * (i + 1) * (i + 1)) : 0;
                    float w = w0 + w1;
                    float d = (i + w1 / w) / s;
                    sum += w * (Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, h.page + _BlurDir.xy * d)
                              + Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, h.page - _BlurDir.xy * d));
                    total += 2 * w;
                }
                return sum / total;
            }
            ENDHLSL }
        Pass { Name "ShadowSeed" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                Here h = At(v);
                float2 px = h.page - _Offset.xy;
                float a = (any(px < _Bounds.xy) || any(px > _Bounds.zw)) ? 0 : Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, px).a;
                return _ShadowColor * a;
            }
            ENDHLSL }
        Pass { Name "Under" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                Here h = At(v);
                half4 s = Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, h.page);
                half4 sh = Read(TEXTURE2D_ARGS(_Shadow, sampler_linear_clamp), h, h.page);
                return s + sh * (1 - s.a);
            }
            ENDHLSL }
            Pass { Name "Backdrop" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            // The page so far over the scene at one page point.
            half4 Under(Here h, float2 page)
            {
                half4 s = Read(TEXTURE2D_ARGS(_Src, sampler_linear_clamp), h, page);
                float2 px = clamp(page, 0.5, _PageSize.xy - 0.5);
                float4 scene = SAMPLE_TEXTURE2D_LOD(_VitrailScene, sampler_linear_clamp, float2(px.x / _PageSize.x, 1 - px.y / _PageSize.y), 0);
                scene.rgb = LinearToSRGB(saturate(scene.rgb));
                return s + scene * (1 - s.a);
            }
            half4 F(V v) : SV_Target
            {
                Here h = At(v);
                // Into a smaller texture (a wide blur runs at a half or a quarter of the size), each texel averages the
                // page pixels it covers: four bilinear reads a quarter of a texel off the centre cover k×k pixels.
                float k = 1 / abs(h.dir.x);
                if (k < 1.5) return Under(h, h.page);
                float d = k * 0.25;
                return (Under(h, h.page + float2(-d, -d)) + Under(h, h.page + float2(d, -d)) + Under(h, h.page + float2(-d, d)) + Under(h, h.page + float2(d, d))) * 0.25;
            }
            ENDHLSL }
            Pass { Name "Over" Blend One OneMinusSrcAlpha
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return ReadPiece(v.page); }
            ENDHLSL }
            Pass { Name "Layer" Blend One OneMinusSrcAlpha
            HLSLPROGRAM
            #pragma vertex VertLayer
            #pragma fragment F
            // A composited layer laid on the page: its bounds (_Bounds, as painted) moved by the matrix
            // (_LayerM0 = a b c d, _LayerM1.xy = e f) and faded by _LayerM1.z; each pixel reads the layer where it was.
            float4 _LayerM0, _LayerM1;
            struct VL { float4 pos : SV_POSITION; float2 page : TEXCOORD0; float2 src : TEXCOORD1; };
            VL VertLayer(uint id : SV_VertexID)
            {
                uint k = id % 6;
                float2 c = float2(k == 1 || k == 2 || k == 4 ? 1 : 0, k == 2 || k == 4 || k == 5 ? 1 : 0);
                VL v;
                v.src = lerp(_Bounds.xy, _Bounds.zw, c);
                v.page = float2(_LayerM0.x * v.src.x + _LayerM0.z * v.src.y + _LayerM1.x, _LayerM0.y * v.src.x + _LayerM0.w * v.src.y + _LayerM1.y);
                v.pos = mul(UNITY_MATRIX_VP, float4(v.page, 0, 1));
                return v;
            }
            half4 F(VL i) : SV_Target
            {
                float2 px = clamp(i.src, _Bounds.xy + 0.5, _Bounds.zw - 0.5);
                return ReadPiece(px) * _LayerM1.z;
            }
            ENDHLSL }
            Pass { Name "Clear" Blend One Zero
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            // Clears the bounds (a changed rectangle of a kept texture, before it is drawn again).
            half4 F(V v) : SV_Target { return 0; }
            ENDHLSL }
    }
}
