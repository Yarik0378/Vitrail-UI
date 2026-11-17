//! calc() in a stylesheet and set from C# (SetStyle): the box lands where a browser puts it.
use std::ffi::CString;
use vitrail::*;

unsafe fn rect(ui: *mut Vitrail, sel: &str) -> [f32; 4] {
    let mut r = [0f32; 4];
    let s = CString::new(sel).unwrap();
    unsafe { vitrail_rect(ui, s.as_ptr(), r.as_mut_ptr()) };
    r
}

#[test]
fn calc_in_stylesheet_and_set_style() {
    let html = r#"<html><head><style>
      body { margin: 0; width: 1000px; height: 200px; position: relative; }
      #a { position: absolute; left: calc(50% - 100px); top: 0; width: 50px; height: 20px; }
      #b { position: absolute; left: 50%; top: 40px; width: 50px; height: 20px; }
      #c { position: absolute; left: 50%; top: 80px; width: 50px; height: 20px; }
    </style></head><body><div id="a"></div><div id="b"></div><div id="c"></div></body></html>"#;
    unsafe {
        let ui = vitrail_create(CString::new(html).unwrap().as_ptr(), CString::new("file:///calc.html").unwrap().as_ptr(), 1000, 200, 1.0);
        let mut out = vec![0u8; 1000 * 200 * 4];
        vitrail_render(ui, out.as_mut_ptr(), out.len());
        let a = rect(ui, "#a");
        let (p, v) = (CString::new("left").unwrap(), CString::new("calc(50% - 100px)").unwrap());
        let sel = CString::new("#b").unwrap();
        vitrail_set_style(ui, sel.as_ptr(), p.as_ptr(), v.as_ptr());
        let (pm, vm) = (CString::new("margin-left").unwrap(), CString::new("-100px").unwrap());
        let selc = CString::new("#c").unwrap();
        vitrail_set_style(ui, selc.as_ptr(), pm.as_ptr(), vm.as_ptr());
        vitrail_render(ui, out.as_mut_ptr(), out.len());
        let b = rect(ui, "#b");
        let c = rect(ui, "#c");
        println!("a {:?}  b {:?}  c {:?}", a, b, c);
        assert!((a[0] - 400.0).abs() < 1.0, "calc in a stylesheet: left {}", a[0]);
        assert!((b[0] - 400.0).abs() < 1.0, "calc set from C#: left {}", b[0]);
        assert!((c[0] - 400.0).abs() < 1.0, "negative margin set from C#: left {}", c[0]);
        vitrail_destroy(ui);
    }
}
