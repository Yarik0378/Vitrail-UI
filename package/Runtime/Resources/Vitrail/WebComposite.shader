// A page (WebPanel) over the camera picture, mixed like a browser (in sRGB). The page covers the rectangle
// _VitrailBand of the screen (x, y from the top-left, width, height, as shares); all the work is done on it only: the
// scene under it is copied out (by the C#), pass 1 turns it into sRGB, passes 2 and 3 blur it across and down
// (σ = _VitrailSigma px) for `backdrop-filter`, and pass 0 draws through a viewport over the rectangle.
Shader "Hidden/Vitrail/WebComposite"
{
    // Each panel sets its own page on its own material: a texture the shader doesn't declare here isn't kept.
    Properties
    {
        _VitrailTex ("Page", 2D) = "black" {}
        _VitrailMask ("Backdrop mask", 2D) = "black" {}
        _VitrailFlip ("Flip", Float) = 0
        _VitrailBlur ("Blur", Vector) = (0, 0, 0, 0)
    }
    HLSLINCLUDE
    #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"
    #include "Packages/com.unity.render-pipelines.core/Runtime/Utilities/Blit.hlsl"
    TEXTURE2D(_VitrailTex);
    TEXTURE2D(_VitrailMask);
    TEXTURE2D(_VitrailBlurred0);
    TEXTURE2D(_VitrailBlurred1);
    TEXTURE2D(_VitrailBlurred2);
    TEXTURE2D(_VitrailBlurred3);
    SAMPLER(sampler_point_clamp);
    float4 _VitrailBlur;              // y: any backdrop (0/1)
    float4 _VitrailPageUv;            // the part of the page that lies on the screen: scale, offset (1, 1, 0, 0 for a page wholly on it)
    float4 _VitrailBand;              // the page's rectangle: x, y (from the top-left), width, height, as shares of the screen
    float _VitrailSigma;              // σ of the blur pass being drawn, in px
    float4 _VitrailTexel;             // 1/width, 1/height of the band textures
    float _VitrailFlip;               // 1: the page is the GPU painter's render texture (its top up)
    TEXTURE2D(_VitrailScene);         // the band of the scene, copied out
    float4x4 _VitrailRegionMatrix[8]; // each backdrop element's colour matrix (rows R, G, B, A over r, g, b, a)
    float4 _VitrailRegionOffset[8];   // its fifth column
    float4 _VitrailRegionInfo[8];     // x: which blurred copy it reads (-1: none)

    float3 ToSrgb(float3 c) { c = max(c, 0); return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1 / 2.4) - 0.055; }
    float3 ToLin(float3 c) { return c <= 0.04045 ? c / 12.92 : pow((c + 0.055) / 1.055, 2.4); }

    float4 Blur(float2 uv, float2 dir)
    {
        float sigma = max(_VitrailSigma, 0.01);
        int r = min((int)ceil(sigma * 3), 48);
        float4 sum = 0; float wsum = 0;
        for (int i = -r; i <= r; i++)
        {
            float w = exp(-(i * i) / (2 * sigma * sigma));
            sum += SAMPLE_TEXTURE2D_LOD(_BlitTexture, sampler_point_clamp, uv + dir * i * _VitrailTexel.xy, 0) * w;
            wsum += w;
        }
        return sum / wsum;
    }
    ENDHLSL

    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" }
        ZTest Always ZWrite Off Cull Off
        Pass
        {
            Name "Composite"
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment Frag
            half4 Frag(Varyings i) : SV_Target
            {
                float2 uv = i.texcoord;
                float2 page = float2(uv.x - _VitrailBand.x, 1 - uv.y - _VitrailBand.y) / max(_VitrailBand.zw, 1e-4);
                clip(page);                                      // outside the page: leave the picture as it is
                clip(1 - page);
                float2 buv = float2(page.x, 1 - page.y);         // the same point in the rectangle's textures
                float2 puv = page * _VitrailPageUv.xy + _VitrailPageUv.zw;   // and in the page (it may hang off the screen)
                float4 under = SAMPLE_TEXTURE2D_LOD(_VitrailScene, sampler_point_clamp, buv, 0);
                float3 scene = ToSrgb(under.rgb);
                if (_VitrailBlur.y > 0.5)
                {
                    float4 mk = SAMPLE_TEXTURE2D_LOD(_VitrailMask, sampler_point_clamp, puv, 0);
                    if (mk.a > 0.002)
                    {
                        int id = clamp((int)round(mk.r / mk.a * 255) - 1, 0, 7);
                        int g = (int)_VitrailRegionInfo[id].x;
                        float3 b = g == 0 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred0, sampler_point_clamp, buv, 0).rgb
                                 : g == 1 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred1, sampler_point_clamp, buv, 0).rgb
                                 : g == 2 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred2, sampler_point_clamp, buv, 0).rgb
                                 : g == 3 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred3, sampler_point_clamp, buv, 0).rgb
                                 : scene;
                        float4 f = mul(_VitrailRegionMatrix[id], float4(b, 1)) + _VitrailRegionOffset[id];
                        scene = lerp(scene, saturate(f.rgb), mk.a);
                    }
                }
                float4 ui = SAMPLE_TEXTURE2D_LOD(_VitrailTex, sampler_point_clamp, _VitrailFlip > 0.5 ? float2(puv.x, 1 - puv.y) : puv, 0);
                // Alpha too ("over"): a page drawn into a transparent texture (a panel in the world) keeps its own coverage.
                return half4(ToLin(ui.rgb + scene * (1 - ui.a)), saturate(ui.a + under.a * (1 - ui.a)));
            }
            ENDHLSL
        }
        Pass
        {
            Name "ToSrgb"
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment Frag
            half4 Frag(Varyings i) : SV_Target { return half4(ToSrgb(SAMPLE_TEXTURE2D_LOD(_BlitTexture, sampler_point_clamp, i.texcoord, 0).rgb), 1); }
            ENDHLSL
        }
        Pass
        {
            Name "BlurX"
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment Frag
            half4 Frag(Varyings i) : SV_Target { return Blur(i.texcoord, float2(1, 0)); }
            ENDHLSL
        }
        Pass
        {
            Name "BlurY"
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment Frag
            half4 Frag(Varyings i) : SV_Target { return Blur(i.texcoord, float2(0, 1)); }
            ENDHLSL
        }
        Pass
        {
            // An overlay scrollbar's thumb (WebPanel): drawn through a viewport over the thumb's box, a rounded
            // rectangle with soft edges in _VitrailThumbColor (linear, premultiplied), over what is there.
            Name "Thumb"
            Blend One OneMinusSrcAlpha
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment Frag
            float4 _VitrailThumbSize;   // xy: the thumb's size in px, z: its corner radius (0: round ends)
            float4 _VitrailThumbColor;
            half4 Frag(Varyings i) : SV_Target
            {
                float2 size = _VitrailThumbSize.xy;
                float2 p = i.texcoord * size - size * 0.5;
                float r = _VitrailThumbSize.z > 0 ? min(_VitrailThumbSize.z, min(size.x, size.y) * 0.5) : min(size.x, size.y) * 0.5;
                float2 q = abs(p) - (size * 0.5 - r);
                float d = length(max(q, 0)) + min(max(q.x, q.y), 0) - r;
                return half4(_VitrailThumbColor * saturate(0.5 - d));
            }
            ENDHLSL
        }
    }
}
