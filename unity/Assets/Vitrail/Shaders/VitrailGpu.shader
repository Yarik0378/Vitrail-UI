// The GPU painter's materials. Values are sRGB, premultiplied (the target is UNORM, nothing converts), so blending is
// done on sRGB values as a browser does. Passes: 0 solid colour, 1 linear gradient, 2 radial, 3 sweep, 4 image;
// 5/6 push/pop a clip in the stencil; 7 composite a group's texture through its clip shape (blend from the material);
// 8 a glyph from the atlas (its coverage times the text colour); 9 a blurred rounded rectangle (box-shadow), the same
// approximation as the CPU painter (vello: Raph Levien's erf fit); 10 mix-blend-mode; 11 an inset box-shadow; 12 every
// material in one pass (the vertex picks it: what the painter uses for draws). Every pass weighs by the vertex's shader
// clips (ClipsCoverage, from the _ClipTable texture).
// Every draw is tested against the stencil = the current clip depth (_StencilRef).
Shader "Hidden/Vitrail/Gpu"
{
    Properties
    {
        _StencilRef ("Clip depth", Int) = 0
        _SrcBlend ("Src blend", Int) = 1
        _DstBlend ("Dst blend", Int) = 10
    }
    HLSLINCLUDE
    #include "Packages/com.unity.render-pipelines.universal/ShaderLibrary/Core.hlsl"
    #include "Packages/com.unity.render-pipelines.core/ShaderLibrary/Color.hlsl"
    TEXTURE2D(_Ramps); SAMPLER(sampler_Ramps);
    TEXTURE2D(_Image); SAMPLER(sampler_linear_clamp);
    float _RampRows;
    TEXTURE2D(_Group); SAMPLER(sampler_point_clamp);
    TEXTURE2D(_Atlas);
    float4 _TargetSize;
    float _GroupAlpha;
    float _ImageGame, _ImageSrgb;
    struct A { float2 pos : POSITION; float4 color : COLOR; float2 uv : TEXCOORD0; float4 p : TEXCOORD1; uint2 meta : TEXCOORD2; float4 q : TEXCOORD3; };
    struct V { float4 pos : SV_POSITION; float4 color : COLOR; float2 uv : TEXCOORD0; nointerpolation float4 p : TEXCOORD1; float2 page : TEXCOORD2;
               nointerpolation uint2 meta : TEXCOORD3; nointerpolation float4 q : TEXCOORD4; };
    // Scroll containers whose content the frame holds unscrolled: _Scroll[i].xy is container i's scroll offset now
    // (device px, its enclosing containers' included); a vertex names its container in meta.x's top 8 bits.
    float4 _Scroll[256];
    float2 ScrollOf(uint id) { return id > 0 ? _Scroll[id].xy : float2(0, 0); }
    V Vert(A a)
    {
        V v;
        float2 p = a.pos - ScrollOf(a.meta.x >> 24);
        v.pos = mul(UNITY_MATRIX_VP, float4(p, 0, 1)); v.color = a.color; v.uv = a.uv; v.p = a.p; v.page = p; v.meta = a.meta; v.q = a.q;
        return v;
    }

    // peniko Extend: 0 pad, 1 repeat, 2 reflect.
    float Extend(float t, float mode)
    {
        if (mode > 1.5) return 1 - abs(frac(t * 0.5) * 2 - 1);
        if (mode > 0.5) return frac(t);
        return saturate(t);
    }
    half4 Ramp(float t, float4 p)
    {
        t = Extend(t, p.y);
        return SAMPLE_TEXTURE2D_LOD(_Ramps, sampler_Ramps, float2((t * 255 + 0.5) / 256, (p.x + 0.5) / _RampRows), 0);
    }

    // Shader clips (Frame::clips) from a table texture, four texels a clip: x0 y0 x1 y1 (x0 > x1: everything but it),
    // the radii rx ry top-left, top-right, then bottom-right, bottom-left, then the scroll container it moves with. A
    // vertex names up to three (Vertex::meta).
    TEXTURE2D_FLOAT(_ClipTable);
    float ClipShape(float2 p, float4 r, float4 r0, float4 r1)
    {
        bool inv = r.x > r.z;
        if (inv) r.xz = r.zx;
        float2 tl = r0.xy, tr = r0.zw, br = r1.xy, bl = r1.zw, c = 0, rad = 0;
        if (p.x < r.x + tl.x && p.y < r.y + tl.y) { c = r.xy + tl; rad = tl; }
        else if (p.x > r.z - tr.x && p.y < r.y + tr.y) { c = float2(r.z - tr.x, r.y + tr.y); rad = tr; }
        else if (p.x > r.z - br.x && p.y > r.w - br.y) { c = r.zw - br; rad = br; }
        else if (p.x < r.x + bl.x && p.y > r.w - bl.y) { c = float2(r.x + bl.x, r.w - bl.y); rad = bl; }
        float d;
        if (rad.x > 1e-3 && rad.y > 1e-3)
        {
            // Distance to an ellipse, first order (exact for circles).
            float2 q = (p - c) / rad;
            float k0 = length(q), k1 = length(q / rad);
            d = k0 * (k0 - 1) / max(k1, 1e-6);
        }
        else d = max(max(r.x - p.x, p.x - r.z), max(r.y - p.y, p.y - r.w));
        float cov = saturate(0.5 - d);
        return inv ? 1 - cov : cov;
    }
    float OneClip(float2 p, uint index)
    {
        if (index == 0) return 1;
        int row = (int)index - 1;
        // A clip inside scrolled content moves with it: it is tested where the pixel is in that content.
        p += ScrollOf((uint)LOAD_TEXTURE2D(_ClipTable, int2(3, row)).x);
        return ClipShape(p, LOAD_TEXTURE2D(_ClipTable, int2(0, row)), LOAD_TEXTURE2D(_ClipTable, int2(1, row)), LOAD_TEXTURE2D(_ClipTable, int2(2, row)));
    }
    float ClipsCoverage(float2 p, uint2 meta)
    {
        return OneClip(p, (meta.x >> 8) & 0xFFFF) * OneClip(p, meta.y & 0xFFFF) * OneClip(p, meta.y >> 16);
    }

    // A blurred rounded rectangle's coverage (box-shadow), the CPU painter's approximation (vello: Raph Levien's erf
    // fit): uv is the pixel relative to the rectangle, p its width, height, radius and σ.
    float Erf7(float x)
    {
        x = clamp(x, -10, 10) * 1.1283791671;   // 2/sqrt(pi)
        float xx = x * x;
        float p = (0.0104 * xx + 0.03395) * xx + 0.24295;
        x = p * (x * xx) + x;
        return x / sqrt(x * x + 1);
    }
    float ShadowCoverage(float2 uv, float4 p)
    {
        float width = p.x, height = p.y, radius = min(p.z, 0.5 * min(p.x, p.y)), sd = max(p.w, 1e-6);
        float minEdge = min(width, height);
        float rmax = 0.5 * minEdge;
        float r0 = min(length(float2(radius, sd * 1.15)), rmax);
        float r1 = min(length(float2(radius, sd * 2.0)), rmax);
        float ex = 2 * r1 / r0;
        float inv = 1 / sd;
        float delta = 1.25 * sd * (exp(-pow(0.5 * inv * width, 2)) - exp(-pow(0.5 * inv * height, 2)));
        float w = width + min(delta, 0), h = height - max(delta, 0);
        float scale = 0.5 * Erf7(inv * 0.5 * (max(w, h) - 0.5 * radius));
        float y = uv.y - 0.5 * height;
        float y0 = r1 - (h * 0.5 - abs(y));
        float y1 = max(y0, 0);
        float x = uv.x - 0.5 * width;
        float x0 = r1 - (w * 0.5 - abs(x));
        float x1 = max(x0, 0);
        float dPos = pow(pow(x1, ex) + pow(y1, ex), 1 / ex);
        float dNeg = min(max(x0, y0), 0);
        float d = dPos + dNeg - r1;
        float z = scale * (Erf7(inv * (minEdge + d)) - Erf7(inv * d));
        return saturate(z);
    }
    ENDHLSL

    SubShader
    {
        Tags { "RenderPipeline" = "UniversalPipeline" }
        ZTest Always ZWrite Off Cull Off
        Blend One OneMinusSrcAlpha
        Stencil { Ref [_StencilRef] Comp Equal Pass Keep }
        Pass { Name "Solid" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return v.color * ClipsCoverage(v.page, v.meta); }
            ENDHLSL }
        Pass { Name "Linear" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return Ramp(v.uv.x, v.p) * (v.color.a * ClipsCoverage(v.page, v.meta)); }
            ENDHLSL }
        Pass { Name "Radial" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { float r0 = v.p.z, r1 = v.p.w; return Ramp((length(v.uv) - r0) / max(r1 - r0, 1e-6), v.p) * (v.color.a * ClipsCoverage(v.page, v.meta)); }
            ENDHLSL }
        Pass { Name "Sweep" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                float a = atan2(v.uv.y, v.uv.x);
                if (a < 0) a += 6.2831853;
                float a0 = v.p.z, a1 = v.p.w;
                return Ramp((a - a0) / max(a1 - a0, 1e-6), v.p) * (v.color.a * ClipsCoverage(v.page, v.meta));
            }
            ENDHLSL }
        Pass { Name "Image" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                float2 uv = float2(Extend(v.uv.x, v.p.z), Extend(v.uv.y, v.p.w));
                // A game texture (texture: URL): Unity's rows run bottom up, and an sRGB texture is read as linear.
                if (_ImageGame > 0.5) uv.y = 1 - uv.y;
                half4 c = SAMPLE_TEXTURE2D_LOD(_Image, sampler_linear_clamp, uv, 0);
                if (_ImageSrgb > 0.5) c.rgb = LinearToSRGB(c.rgb);
                if (v.p.y < 0.5) c.rgb *= c.a;          // straight alpha → premultiplied
                return c * (v.p.x * v.color.a * ClipsCoverage(v.page, v.meta));
            }
            ENDHLSL }
            Pass { Name "PushClip" ColorMask 0
            Stencil { Ref [_StencilRef] Comp Equal Pass IncrSat }
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return 0; }
            ENDHLSL }
        Pass { Name "PopClip" ColorMask 0
            Stencil { Ref [_StencilRef] Comp Equal Pass DecrSat }
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return 0; }
            ENDHLSL }
        Pass { Name "Composite" Blend [_SrcBlend] [_DstBlend]
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target
            {
                float2 uv = v.pos.xy / _TargetSize.xy;
                #if UNITY_UV_STARTS_AT_TOP
                #else
                uv.y = 1 - uv.y;
                #endif
                return SAMPLE_TEXTURE2D_LOD(_Group, sampler_linear_clamp, uv, 0) * (_GroupAlpha * v.color.a * ClipsCoverage(v.page, v.meta));
            }
            ENDHLSL }
        Pass { Name "Glyph" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return v.color * (SAMPLE_TEXTURE2D_LOD(_Atlas, sampler_point_clamp, v.uv, 0).r * ClipsCoverage(v.page, v.meta)); }
            ENDHLSL }
        Pass { Name "BoxShadow" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            half4 F(V v) : SV_Target { return v.color * (ShadowCoverage(v.uv, v.p) * ClipsCoverage(v.page, v.meta)); }
            ENDHLSL }
            Pass { Name "Blend" Blend One Zero
            HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            // mix-blend-mode (W3C Compositing Level 1): the group (source) over the copied backdrop with B(Cb, Cs) in
            // place of the source colour where both are present; the group's alpha and the edge coverage weigh it.
            TEXTURE2D(_Backdrop);
            int _Mix;
            float Lum(float3 c) { return dot(c, float3(0.3, 0.59, 0.11)); }
            float3 ClipColor(float3 c)
            {
                float l = Lum(c), n = min(c.r, min(c.g, c.b)), x = max(c.r, max(c.g, c.b));
                if (n < 0) c = l + (c - l) * l / max(l - n, 1e-6);
                if (x > 1) c = l + (c - l) * (1 - l) / max(x - l, 1e-6);
                return c;
            }
            float3 SetLum(float3 c, float l) { return ClipColor(c + (l - Lum(c))); }
            float Sat(float3 c) { return max(c.r, max(c.g, c.b)) - min(c.r, min(c.g, c.b)); }
            float3 SetSat(float3 c, float s)
            {
                float n = min(c.r, min(c.g, c.b)), x = max(c.r, max(c.g, c.b));
                return x > n ? (c - n) * s / (x - n) : 0;
            }
            float SoftLight(float b, float s)
            {
                float d = b <= 0.25 ? ((16 * b - 12) * b + 4) * b : sqrt(b);
                return s <= 0.5 ? b - (1 - 2 * s) * b * (1 - b) : b + (2 * s - 1) * (d - b);
            }
            float3 Mix(float3 b, float3 s)
            {
                switch (_Mix)
                {
                    case 1: return b * s;
                    case 2: return b + s - b * s;
                    case 3: return b <= 0.5 ? 2 * s * b : 1 - 2 * (1 - s) * (1 - b);            // overlay = hard-light(s, b)
                    case 4: return min(b, s);
                    case 5: return max(b, s);
                    case 6: return b <= 0 ? 0 : (s >= 1 ? 1 : min(1, b / (1 - s)));
                    case 7: return b >= 1 ? 1 : (s <= 0 ? 0 : 1 - min(1, (1 - b) / s));
                    case 8: return s <= 0.5 ? 2 * s * b : 1 - 2 * (1 - s) * (1 - b);
                    case 9: return float3(SoftLight(b.r, s.r), SoftLight(b.g, s.g), SoftLight(b.b, s.b));
                    case 10: return abs(b - s);
                    case 11: return b + s - 2 * b * s;
                    case 12: return SetLum(SetSat(s, Sat(b)), Lum(b));
                    case 13: return SetLum(SetSat(b, Sat(s)), Lum(b));
                    case 14: return SetLum(s, Lum(b));
                    case 15: return SetLum(b, Lum(s));
                    default: return s;
                }
            }
            half4 F(V v) : SV_Target
            {
                float2 uv = v.pos.xy / _TargetSize.xy;
                #if !UNITY_UV_STARTS_AT_TOP
                uv.y = 1 - uv.y;
                #endif
                float4 src = SAMPLE_TEXTURE2D_LOD(_Group, sampler_point_clamp, uv, 0) * (_GroupAlpha * v.color.a * ClipsCoverage(v.page, v.meta));
                float4 dst = SAMPLE_TEXTURE2D_LOD(_Backdrop, sampler_point_clamp, uv, 0);
                float3 cs = src.a > 0 ? src.rgb / src.a : 0, cb = dst.a > 0 ? dst.rgb / dst.a : 0;
                float3 rgb = src.rgb * (1 - dst.a) + dst.rgb * (1 - src.a) + src.a * dst.a * saturate(Mix(cb, cs));
                return float4(rgb, src.a + dst.a - src.a * dst.a);
            }
            ENDHLSL }
            Pass { Name "InsetShadow" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            // An inset box-shadow in one draw: the colour where the blurred hole doesn't reach, in the padding box.
            half4 F(V v) : SV_Target { return v.color * ((1 - ShadowCoverage(v.uv, v.p)) * ClipsCoverage(v.page, v.meta)); }
            ENDHLSL }
            Pass { Name "Uber" HLSLPROGRAM
            #pragma vertex Vert
            #pragma fragment F
            // Every material in one pass: the vertex says which (Vertex::meta, low 8 bits), so a run of fills,
            // gradients, text, shadows and atlas images is one draw. Small images come from the image atlas (q).
            TEXTURE2D(_ImageAtlas);
            half4 F(V v) : SV_Target
            {
                uint kind = v.meta.x & 255;
                half4 c;
                if (kind == 0) c = v.color;
                else if (kind == 1) c = Ramp(v.uv.x, v.p) * v.color.a;
                else if (kind == 2) { float r0 = v.p.z, r1 = v.p.w; c = Ramp((length(v.uv) - r0) / max(r1 - r0, 1e-6), v.p) * v.color.a; }
                else if (kind == 3)
                {
                    float a = atan2(v.uv.y, v.uv.x);
                    if (a < 0) a += 6.2831853;
                    c = Ramp((a - v.p.z) / max(v.p.w - v.p.z, 1e-6), v.p) * v.color.a;
                }
                else if (kind == 4)
                {
                    if (v.q.z > v.q.x)
                    {
                        // The image's rectangle; reads stay half a texel inside it (the atlas is 2048 wide).
                        float2 a = lerp(v.q.xy, v.q.zw, saturate(v.uv));
                        a = clamp(a, v.q.xy + 0.5 / 2048, v.q.zw - 0.5 / 2048);
                        c = SAMPLE_TEXTURE2D_LOD(_ImageAtlas, sampler_linear_clamp, a, 0);
                    }
                    else
                    {
                        float2 uv = float2(Extend(v.uv.x, v.p.z), Extend(v.uv.y, v.p.w));
                        if (_ImageGame > 0.5) uv.y = 1 - uv.y;
                        c = SAMPLE_TEXTURE2D_LOD(_Image, sampler_linear_clamp, uv, 0);
                        if (_ImageSrgb > 0.5) c.rgb = LinearToSRGB(c.rgb);
                    }
                    if (v.p.y < 0.5) c.rgb *= c.a;
                    c *= v.p.x * v.color.a;
                }
                else if (kind == 8) c = v.color * SAMPLE_TEXTURE2D_LOD(_Atlas, sampler_point_clamp, v.uv, 0).r;
                else if (kind == 9) c = v.color * ShadowCoverage(v.uv, v.p);
                else if (kind == 11) c = v.color * (1 - ShadowCoverage(v.uv, v.p));
                else c = 0;
                return c * ClipsCoverage(v.page, v.meta);
            }
            ENDHLSL }
    }
}
