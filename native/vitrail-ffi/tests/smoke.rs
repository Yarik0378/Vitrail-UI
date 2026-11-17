//! The bridge end to end, as the game drives it: a page, the pointer, the wheel, painting on a worker, changed rects.
use std::ffi::CString;
use vitrail::*;

const PAGE: &str = r#"<!doctype html><html><head><style>
html,body{margin:0}
.btn{position:absolute;left:10px;top:10px;width:40px;height:30px;background:#335;cursor:pointer}
.btn:hover{background:#88c}
.list{position:absolute;left:100px;top:0;width:100px;height:60px;overflow-y:scroll}
.list div{height:30px;background:#444;border-bottom:1px solid #999}
#t{position:absolute;left:220px;top:10px;color:#fff;font:16px sans-serif}
</style></head><body>
<div class="btn" data-action="go" id="b"></div>
<div class="list"><div>1</div><div>2</div><div>3</div><div>4</div><div>5</div></div>
<div id="t">old</div></body></html>"#;

fn c(s: &str) -> CString { CString::new(s).unwrap() }

unsafe fn paint(ui: *mut Vitrail, w: u32, h: u32) -> (i32, [u32; 4]) {
    let mut out = vec![0u8; (w * h * 4) as usize];
    let mut mask = vec![0u8; (w * h * 4) as usize];
    let mut rect = [0u32; 4];
    let mut params = [0f32; 21 * 8];
    let mut regions = 0u32;
    assert_eq!(unsafe { vitrail_paint_start(ui) }, 1, "a paint should start");
    loop {
        let f = unsafe { vitrail_paint_poll(ui, out.as_mut_ptr(), out.len(), rect.as_mut_ptr(), mask.as_mut_ptr(), mask.len(), params.as_mut_ptr(), 8, &mut regions) };
        if f != 0 { return (f, rect); }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn drives_a_page() {
    unsafe {
        let (w, h) = (300u32, 80u32);
        let ui = vitrail_create(c(PAGE).as_ptr(), std::ptr::null(), w, h, 1.0);
        assert!(!ui.is_null());
        // First paint: the whole page.
        let (f, r) = paint(ui, w, h);
        assert_eq!(f & 2, 2);
        assert_eq!(r, [0, 0, w, h]);
        // Nothing changed: nothing to paint.
        assert_eq!(vitrail_needs_paint(ui), 0);
        assert_eq!(vitrail_paint_start(ui), 0);
        // Hover the button: the hand, and only the button repaints.
        assert_eq!(vitrail_pointer_move(ui, 20.0, 20.0), 1);
        assert_eq!(vitrail_cursor(ui), 1, "cursor: pointer gives the hand");
        let (f, r) = paint(ui, w, h);
        assert_eq!(f & 2, 2);
        assert!(r[0] >= 10 && r[1] >= 10 && r[2] <= 40 && r[3] <= 30, "only the button changed, got {r:?}");
        // A text change repaints its own patch only.
        assert_eq!(vitrail_set_text(ui, c("#t").as_ptr(), c("new and longer").as_ptr()), 1);
        let (_, r) = paint(ui, w, h);
        assert!(r[0] >= 200, "only the text changed, got {r:?}");
        // The wheel over the list scrolls it.
        assert_eq!(vitrail_scroll(ui, 150.0, 30.0, 0.0, 30.0), 1, "the list scrolls");
        let (f, r) = paint(ui, w, h);
        assert_eq!(f & 2, 2);
        assert!(r[0] >= 100 && r[0] + r[2] <= 200, "only the list changed, got {r:?}");
        // A click on the button reports its action.
        vitrail_pointer_down(ui, 20.0, 20.0);
        let (mut a, mut i) = ([0i8; 64], [0i8; 64]);
        assert_eq!(vitrail_pointer_up(ui, 20.0, 20.0, a.as_mut_ptr(), i.as_mut_ptr(), 64), 1);
        assert_eq!(std::ffi::CStr::from_ptr(a.as_ptr()).to_str().unwrap(), "go");
        vitrail_destroy(ui);
    }
}
