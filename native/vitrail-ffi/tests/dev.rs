//! Developer tools: the element tree, one element's box and computed styles, picking and live style edits; and
//! getComputedStyle in scripts.
use std::ffi::{CString, c_char};
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn read(f: impl Fn(*mut c_char, usize) -> i32) -> String {
    let mut cap = 256;
    loop {
        let mut buf = vec![0u8; cap];
        let r = f(buf.as_mut_ptr() as *mut c_char, cap);
        if r < 0 { cap = (-r) as usize; continue; }
        assert_eq!(r, 1);
        return String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == 0).unwrap()]).into_owned();
    }
}

const PAGE: &str = r#"<!doctype html><html><head><style>
body { margin: 0; } .card { position: absolute; left: 20px; top: 10px; width: 100px; height: 50px; padding: 5px; border: 2px solid red; color: rgb(10, 20, 30); }
</style></head><body><div class="card big" id="c">Hello <b>world</b></div>
<script>engine.on('cs', () => engine.trigger('cs', getComputedStyle(document.getElementById('c')).color, getComputedStyle(document.getElementById('c')).getPropertyValue('padding-top')));</script>
</body></html>"#;

#[test]
fn inspector_reads_the_tree_boxes_and_styles() {
    unsafe {
        let ui = vitrail_create(s(PAGE).as_ptr(), s("").as_ptr(), 400, 200, 2.0);
        let tree = read(|b, c| vitrail_dev_tree(ui, b, c));
        println!("{tree}");
        let card = tree.lines().find(|l| l.ends_with("div#c.card.big")).expect("the card is in the tree");
        assert!(tree.lines().any(|l| l.ends_with("\tt\tHello")), "text nodes too");
        let id: u64 = card.split('\t').next().unwrap().parse().unwrap();
        let node = read(|b, c| vitrail_dev_node(ui, id, b, c));
        assert!(node.contains("rect\t40 20 228 128"), "the border box in device px: {node}");
        assert!(node.contains("padding\t5 5 5 5") && node.contains("border\t2 2 2 2"));
        assert!(node.contains("attr\tclass\tcard big"));
        assert!(node.contains("style\tcolor\trgb(10, 20, 30)"), "computed styles");
        // Picking: the element under a point; the bold word is its own element.
        let picked = vitrail_dev_pick(ui, 60.0, 40.0);
        assert!(picked == id || tree.lines().any(|l| l.starts_with(&format!("{picked}\t")) && l.contains("\tb")), "picked {picked}");
        // A live edit moves it.
        assert_eq!(vitrail_dev_set_style(ui, id, s("left").as_ptr(), s("50px").as_ptr()), 1);
        let mut r = [0f32; 4];
        vitrail_dev_rect(ui, id, r.as_mut_ptr());
        assert_eq!(r[0], 100.0);
        // getComputedStyle in scripts.
        vitrail_js_call(ui, s("cs").as_ptr(), s("[]").as_ptr());
        let mut a = vec![0u8; 64];
        let mut j = vec![0u8; 256];
        assert_eq!(vitrail_js_poll(ui, a.as_mut_ptr() as *mut c_char, 64, j.as_mut_ptr() as *mut c_char, 256), 1);
        let json = String::from_utf8_lossy(&j[..j.iter().position(|&b| b == 0).unwrap()]).into_owned();
        assert_eq!(json, r#"["rgb(10, 20, 30)","5px"]"#);
        vitrail_destroy(ui);
    }
}
