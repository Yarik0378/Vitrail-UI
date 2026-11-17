// The page over the camera picture, mixed like a browser (in sRGB). Pass 0 mixes; pass 1 copies the scene into sRGB;
// passes 2 and 3 blur that copy across and down (a Gaussian, σ = _VitrailBlur.x px) for `backdrop-filter`.
Shader "Hidden/Vitrail/Composite"
{
    HLSLINCLUDE
    #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"
    #include "Packages/com.unity.render-pipelines.core/Runtime/Utilities/Blit.hlsl"
    TEXTURE2D(_VitrailTex);
    TEXTURE2D(_VitrailMask);
    TEXTURE2D(_VitrailBlurred0);
    TEXTURE2D(_VitrailBlurred1);
    TEXTURE2D(_VitrailBlurred2);
    TEXTURE2D(_VitrailBlurred3);
    TEXTURE2D(_VitrailSceneSrgb);
    SAMPLER(sampler_point_clamp);
    SAMPLER(sampler_linear_clamp);
    float4 _VitrailBlur;              // y: any backdrop (0/1)
    float _VitrailSigma;              // σ of the blur pass being drawn, in px
    float4x4 _VitrailRegionMatrix[8]; // each backdrop element's colour matrix (rows R, G, B, A over r, g, b, a)
    float4 _VitrailRegionOffset[8];   // its fifth column
    float4 _VitrailRegionInfo[8];     // x: which blurred copy it reads (-1: none, the scene as it is)
    float4 _VitrailTexel;          // 1/width, 1/height of the screen

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
                float2 page = float2(uv.x, 1 - uv.y);
                float3 scene = ToSrgb(SAMPLE_TEXTURE2D_LOD(_BlitTexture, sampler_point_clamp, uv, 0).rgb);
                if (_VitrailBlur.y > 0.5)
                {
                    float4 mk = SAMPLE_TEXTURE2D_LOD(_VitrailMask, sampler_point_clamp, page, 0);
                    if (mk.a > 0.002)
                    {
                        int id = clamp((int)round(mk.r / mk.a * 255) - 1, 0, 7);
                        int g = (int)_VitrailRegionInfo[id].x;
                        float3 b = g == 0 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred0, sampler_point_clamp, uv, 0).rgb
                                 : g == 1 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred1, sampler_point_clamp, uv, 0).rgb
                                 : g == 2 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred2, sampler_point_clamp, uv, 0).rgb
                                 : g == 3 ? SAMPLE_TEXTURE2D_LOD(_VitrailBlurred3, sampler_point_clamp, uv, 0).rgb
                                 : scene;
                        float4 f = mul(_VitrailRegionMatrix[id], float4(b, 1)) + _VitrailRegionOffset[id];
                        scene = lerp(scene, saturate(f.rgb), mk.a);
                    }
                }
                float4 ui = SAMPLE_TEXTURE2D_LOD(_VitrailTex, sampler_point_clamp, page, 0);
                return half4(ToLin(ui.rgb + scene * (1 - ui.a)), 1);
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
    }
}
