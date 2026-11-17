using System;
using System.Runtime.InteropServices;

namespace VitrailLab
{
    /// <summary>The native Blitz bridge (native/vitrail-ffi): HTML + CSS laid out and painted without a browser.</summary>
    public static class VitrailNative
    {
        private const string Lib = "vitrail";
        [DllImport(Lib)] public static extern IntPtr vitrail_create([MarshalAs(UnmanagedType.LPUTF8Str)] string html, [MarshalAs(UnmanagedType.LPUTF8Str)] string baseUrl, uint width, uint height, float scale);
        [DllImport(Lib)] public static extern void vitrail_resize(IntPtr ui, uint width, uint height, float scale);
        [DllImport(Lib)] public static extern unsafe int vitrail_render(IntPtr ui, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe uint vitrail_backdrop(IntPtr ui, byte* mask, UIntPtr len, float* parameters, uint maxRegions);
        [DllImport(Lib)] public static extern void vitrail_destroy(IntPtr ui);
        [DllImport(Lib)] public static extern void vitrail_set_gpu(IntPtr ui, int on);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_clips(IntPtr ui, float* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_image_atlas(IntPtr ui, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_scroll_states(IntPtr ui, float* output, UIntPtr max);
        [DllImport(Lib)] public static extern int vitrail_scroll(IntPtr ui, float x, float y, float dx, float dy);
        [DllImport(Lib)] public static extern uint vitrail_texture_id(string name);
        [DllImport(Lib)] public static extern uint vitrail_gpu_image_texture(IntPtr ui, uint index);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_filters(IntPtr ui, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_build(IntPtr ui, uint* counts);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_ramps(IntPtr ui, uint* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_ops(IntPtr ui, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_atlas(IntPtr ui, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_image(IntPtr ui, uint index, uint* info, byte* output, UIntPtr len);
        [DllImport(Lib)] public static extern unsafe int vitrail_gpu_copy(IntPtr ui, byte* vertices, UIntPtr verticesLen, uint* indices, UIntPtr indicesLen, byte* batches, UIntPtr batchesLen);
        [DllImport(Lib)] public static extern int vitrail_needs_paint(IntPtr ui);
        [DllImport(Lib)] public static extern int vitrail_set_text(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        [DllImport(Lib)] public static extern int vitrail_set_attr(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string name, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        [DllImport(Lib)] public static extern int vitrail_set_class(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string cls, int on);
        [DllImport(Lib)] public static extern int vitrail_set_style(IntPtr ui, [MarshalAs(UnmanagedType.LPUTF8Str)] string selector, [MarshalAs(UnmanagedType.LPUTF8Str)] string property, [MarshalAs(UnmanagedType.LPUTF8Str)] string value);
        [DllImport(Lib)] public static extern int vitrail_pointer_move(IntPtr ui, float x, float y);
        [DllImport(Lib)] public static extern int vitrail_hit(IntPtr ui, float x, float y);
        [DllImport(Lib)] public static extern void vitrail_pointer_down(IntPtr ui, float x, float y);
        [DllImport(Lib)] public static extern unsafe int vitrail_pointer_up(IntPtr ui, float x, float y, byte* action, byte* id, UIntPtr cap);
    }
}
