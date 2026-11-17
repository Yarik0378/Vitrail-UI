//! A game texture in <img src="texture:name">: the frame says which image stands for it, at any drawn size.
use std::ffi::CString;
use vitrail::*;

#[test]
fn texture_images_are_recognised() {
    let html = r#"<html><body style="margin:0">
        <img src="texture:minimap?w=128&h=64" style="width:256px;height:128px">
        <img src="texture:portrait?w=200&h=200" style="width:40px;height:40px;border-radius:50%">
        </body></html>"#;
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new("file:///tmp/t.html").unwrap().as_ptr(), 400, 200, 1.0);
        vitrail_set_gpu(ui, 2);
        let mut c = [0u32; 7];
        vitrail_gpu_build(ui, c.as_mut_ptr());
        let ids: Vec<u32> = (0..c[4]).map(|i| vitrail_gpu_image_texture(ui, i)).collect();
        let want = |n: &str| vitrail_texture_id(CString::new(n).unwrap().as_ptr());
        assert!(ids.contains(&want("minimap")), "minimap missing: {ids:?}");
        assert!(ids.contains(&want("portrait")), "portrait (drawn smaller) missing: {ids:?}");
        vitrail_destroy(ui);
    }
}
