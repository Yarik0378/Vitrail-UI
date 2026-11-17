//! Form controls and the gamepad: `<select>` (the closed box, its list with the mouse and keys, scripts reading and
//! setting it, the game reading it) and moving the focus between elements by direction.
use std::ffi::{CString, c_char};
use vitrail::*;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

unsafe fn events(ui: *mut Vitrail) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let (mut a, mut b) = (vec![0u8; 256], vec![0u8; 1 << 16]);
    while unsafe { vitrail_js_poll(ui, a.as_mut_ptr() as *mut c_char, a.len(), b.as_mut_ptr() as *mut c_char, b.len()) } == 1 {
        let (na, nb) = (a.iter().position(|&x| x == 0).unwrap(), b.iter().position(|&x| x == 0).unwrap());
        out.push((String::from_utf8_lossy(&a[..na]).into_owned(), String::from_utf8_lossy(&b[..nb]).into_owned()));
    }
    out
}

unsafe fn value(ui: *mut Vitrail, sel: &str) -> String {
    let mut buf = vec![0u8; 256];
    assert_eq!(unsafe { vitrail_value(ui, s(sel).as_ptr(), buf.as_mut_ptr() as *mut c_char, buf.len()) }, 1, "{sel} has a value");
    String::from_utf8_lossy(&buf[..buf.iter().position(|&b| b == 0).unwrap()]).into_owned()
}

unsafe fn rect(ui: *mut Vitrail, sel: &str) -> [f32; 4] {
    let mut r = [0f32; 4];
    assert_eq!(unsafe { vitrail_rect(ui, s(sel).as_ptr(), r.as_mut_ptr()) }, 1, "{sel} is on the page");
    r
}

unsafe fn click(ui: *mut Vitrail, x: f32, y: f32) -> (i32, String) {
    unsafe {
        let (mut a, mut i) = (vec![0u8; 128], vec![0u8; 128]);
        vitrail_pointer_move(ui, x, y);
        vitrail_pointer_down(ui, x, y);
        let r = vitrail_pointer_up(ui, x, y, a.as_mut_ptr() as *mut c_char, i.as_mut_ptr() as *mut c_char, 128);
        (r, String::from_utf8_lossy(&a[..a.iter().position(|&b| b == 0).unwrap()]).into_owned())
    }
}

unsafe fn key(ui: *mut Vitrail, k: &str) -> i32 {
    unsafe {
        let r = vitrail_key(ui, 1, s(k).as_ptr(), s(k).as_ptr(), 0, 0);
        vitrail_key(ui, 0, s(k).as_ptr(), s(k).as_ptr(), 0, 0);
        r
    }
}

fn save(ui: *mut Vitrail, w: u32, h: u32, name: &str) {
    let dir = std::env::var("VITRAIL_SHOTS").unwrap_or_default();
    if dir.is_empty() { return; }
    let mut px = vec![0u8; (w * h * 4) as usize];
    unsafe { vitrail_render(ui, px.as_mut_ptr(), px.len()) };
    let f = std::fs::File::create(format!("{dir}/{name}.png")).unwrap();
    let mut e = png::Encoder::new(std::io::BufWriter::new(f), w, h);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header().unwrap().write_image_data(&px).unwrap();
}

const PAGE: &str = r#"<!doctype html><html><body style="margin:0;font:14px sans-serif;background:#ddd">
<div style="padding:20px">
  <select id="s" data-action="pick">
    <option value="a">Apple</option>
    <option value="b" selected>Blackberry jam</option>
    <optgroup label="Later"><option value="c">Cherry</option><option value="d" disabled>Date</option><option value="e">Elder</option></optgroup>
  </select>
  <select id="t"><option>One</option><option>Two</option></select>
</div>
<script>
  const s = document.getElementById('s');
  s.addEventListener('change', () => engine.trigger('change', s.value, s.selectedIndex));
  engine.on('read', () => engine.trigger('read', s.value, s.selectedIndex, s.options.length, s.options[2].text, document.getElementById('t').value));
  engine.on('set', (v) => { s.value = v; engine.trigger('now', s.value); });
</script>
</body></html>"#;

#[test]
fn select_shows_its_choice_and_opens_a_list() {
    unsafe {
        let (w, h) = (500u32, 300u32);
        let ui = vitrail_create(s(PAGE).as_ptr(), s("file:///tmp/forms.html").as_ptr(), w, h, 1.0);
        // The selected attribute picks the first choice; a select without one shows its first option.
        assert_eq!(value(ui, "#s"), "b");
        assert_eq!(value(ui, "#t"), "One");
        vitrail_js_call(ui, s("read").as_ptr(), s("[]").as_ptr());
        assert_eq!(events(ui), vec![("read".to_string(), r#"["b",1,5,"Cherry","One"]"#.to_string())]);
        // The closed box is as wide as its widest option (Blackberry jam), and as tall as one line.
        let r = rect(ui, "#s");
        let t = rect(ui, "#t");
        println!("select boxes: {r:?} {t:?}");
        assert!(r[2] > t[2] + 40.0 && r[3] < 30.0 && r[3] > 14.0, "box {r:?}");
        save(ui, w, h, "select-closed");

        // A press opens the list under it, the choice highlighted; a press on Cherry chooses it.
        click(ui, r[0] + 10.0, r[1] + r[3] / 2.0);
        assert!(rect(ui, "vitrail-popup")[1] >= r[1] + r[3] - 1.0, "the list opens under the box");
        let cur = rect(ui, "vitrail-option[current]");
        let blackberry = rect(ui, "vitrail-option:nth-child(2)");
        assert_eq!(cur, blackberry, "the chosen option is highlighted");
        save(ui, w, h, "select-open");
        let cherry = rect(ui, "vitrail-option:nth-of-type(3)");
        let (clicked, _) = click(ui, cherry[0] + 10.0, cherry[1] + cherry[3] / 2.0);
        assert_eq!(clicked, 0, "choosing in the list is not a click on the page");
        assert!(vitrail_rect(ui, s("vitrail-popup").as_ptr(), std::ptr::null_mut()) == 0, "the list closed");
        assert_eq!(value(ui, "#s"), "c");
        assert_eq!(events(ui), vec![("change".to_string(), r#"["c",2]"#.to_string())]);

        // A disabled option can't be chosen with the mouse; a press outside closes without choosing.
        click(ui, r[0] + 10.0, r[1] + r[3] / 2.0);
        let date = rect(ui, "vitrail-option[disabled]");
        click(ui, date[0] + 10.0, date[1] + date[3] / 2.0);
        assert_eq!(value(ui, "#s"), "c");
        click(ui, 450.0, 280.0);
        assert!(vitrail_rect(ui, s("vitrail-popup").as_ptr(), std::ptr::null_mut()) == 0);

        // Keys: the focused select steps on the arrows (over the disabled Date), opens on Enter, the list moves on the
        // arrows and chooses on Enter; Escape closes without choosing.
        click(ui, r[0] + 10.0, r[1] + r[3] / 2.0);
        key(ui, "Escape");
        assert_eq!(key(ui, "ArrowDown"), 1, "the select takes the arrow");
        assert_eq!(value(ui, "#s"), "e");
        key(ui, "ArrowUp");
        key(ui, "ArrowUp");
        assert_eq!(value(ui, "#s"), "b");
        key(ui, "Enter");
        key(ui, "ArrowUp");
        key(ui, "Enter");
        assert_eq!(value(ui, "#s"), "a");
        key(ui, "Enter");
        key(ui, "End");
        key(ui, "Escape");
        assert_eq!(value(ui, "#s"), "a");
        events(ui);

        // Scripts and the game set it (no change event then).
        vitrail_js_call(ui, s("set").as_ptr(), s(r#"["e"]"#).as_ptr());
        assert_eq!(events(ui), vec![("now".to_string(), r#"["e"]"#.to_string())]);
        assert_eq!(vitrail_select_value(ui, s("#s").as_ptr(), s("b").as_ptr()), 1);
        assert_eq!(value(ui, "#s"), "b");
        save(ui, w, h, "select-after");
        vitrail_destroy(ui);
    }
}

const GRID: &str = r#"<!doctype html><html><head><style>
body { margin: 0; font: 14px sans-serif; }
button { position: absolute; width: 80px; height: 30px; }
button:focus-visible { outline: 3px solid gold; }
.list { position: absolute; left: 300px; top: 10px; width: 120px; height: 100px; overflow-y: auto; }
.list button { position: static; display: block; margin: 4px; }
</style></head><body>
<button id="a" style="left:10px;top:10px" data-action="a">A</button>
<button id="b" style="left:110px;top:10px" data-action="b">B</button>
<button id="c" style="left:10px;top:60px" data-action="c">C</button>
<button id="d" style="left:130px;top:70px" data-action="d">D</button>
<button id="x" style="left:10px;top:120px" tabindex="-1">X</button>
<div class="list"><button id="l1">1</button><button id="l2">2</button><button id="l3">3</button><button id="l4">4</button><button id="l5">5</button></div>
</body></html>"#;

#[test]
fn gamepad_moves_the_focus_by_direction() {
    unsafe {
        let ui = vitrail_create(s(GRID).as_ptr(), s("").as_ptr(), 500, 200, 1.0);
        let mut a = vec![0u8; 64];
        let mut i = vec![0u8; 64];
        let mut act = |ui: *mut Vitrail| -> (i32, String) {
            let r = vitrail_activate(ui, a.as_mut_ptr() as *mut c_char, i.as_mut_ptr() as *mut c_char, 64);
            let t = |b: &[u8]| String::from_utf8_lossy(&b[..b.iter().position(|&x| x == 0).unwrap_or(0)]).into_owned();
            (r, if a[0] != 0 { t(&a) } else { t(&i) })
        };
        assert_eq!(act(ui).0, 0, "nothing focused: nothing to press");
        // The first step focuses the top-left element; then by direction (D is lower than B: down from B reaches it,
        // right from C too); X (tabindex -1) is never reached.
        assert_eq!(vitrail_navigate(ui, 2), 1);
        assert_eq!(act(ui), (1, "a".to_string()));
        vitrail_navigate(ui, 1);
        assert_eq!(act(ui), (1, "b".to_string()));
        vitrail_navigate(ui, 2);
        assert_eq!(act(ui), (1, "d".to_string()));
        vitrail_navigate(ui, 3);
        assert_eq!(act(ui), (1, "c".to_string()));
        vitrail_navigate(ui, 2);
        assert_ne!(act(ui).1, "x", "X (tabindex -1) is never reached");
        // Into the scrolled list: stepping down scrolls it so the focused button shows.
        for _ in 0..6 { vitrail_navigate(ui, 2); println!("step: list {:?} focused {:?}", rect(ui, ".list"), act(ui)); }
        assert_eq!(vitrail_navigate(ui, 2), 0, "nothing below the last button");
        assert_eq!(act(ui), (1, "l5".to_string()));
        let (l5, list) = (rect(ui, "#l5"), rect(ui, ".list"));
        assert!(l5[1] >= list[1] && l5[1] + l5[3] <= list[1] + list[3] + 0.5, "the last button scrolled into view: {l5:?} in {list:?}");
        // B (the gamepad's back) takes the focus away.
        assert_eq!(vitrail_cancel(ui), 1);
        assert_eq!(vitrail_cancel(ui), 0);
        vitrail_destroy(ui);
    }
}

const DND: &str = r#"<!doctype html><html><head><style>
body { margin: 0; } .slot { position: absolute; top: 10px; width: 60px; height: 60px; background: #456; }
#item { width: 40px; height: 40px; margin: 10px; background: gold; }
</style></head><body>
<div class="slot" id="from" style="left:10px"><div id="item" draggable="true"></div></div>
<div class="slot" id="to" style="left:110px"></div>
<div class="slot" id="nope" style="left:210px"></div>
<script>
  const item = document.getElementById('item'), to = document.getElementById('to');
  const log = [];
  item.addEventListener('dragstart', (e) => { e.dataTransfer.setData('text/plain', 'sword'); log.push('start'); });
  item.addEventListener('dragend', (e) => { log.push('end:' + e.dataTransfer.dropEffect); engine.trigger('log', log.join(' ')); log.length = 0; });
  item.addEventListener('click', () => log.push('click'));
  to.addEventListener('dragenter', () => log.push('enter'));
  to.addEventListener('dragleave', () => log.push('leave'));
  to.addEventListener('dragover', (e) => e.preventDefault());
  to.addEventListener('drop', (e) => { to.appendChild(item); log.push('drop:' + e.dataTransfer.getData('text/plain')); });
</script></body></html>"#;

#[test]
fn drag_and_drop_moves_an_item_between_slots() {
    unsafe {
        let ui = vitrail_create(s(DND).as_ptr(), s("").as_ptr(), 400, 100, 1.0);
        let drag = |pts: &[(f32, f32)]| {
            vitrail_pointer_move(ui, pts[0].0, pts[0].1);
            vitrail_pointer_down(ui, pts[0].0, pts[0].1);
            for &(x, y) in &pts[1..] { vitrail_pointer_move(ui, x, y); }
            let (x, y) = pts[pts.len() - 1];
            vitrail_pointer_up(ui, x, y, std::ptr::null_mut(), std::ptr::null_mut(), 0);
        };
        // Onto a slot that refuses: no drop, nothing moves.
        drag(&[(40.0, 40.0), (60.0, 40.0), (150.0, 40.0), (240.0, 40.0)]);
        assert_eq!(events(ui), vec![("log".to_string(), r#"["start enter leave end:none"]"#.to_string())]);
        // Onto the slot that takes it: dropped, and the release's click is swallowed.
        drag(&[(40.0, 40.0), (60.0, 40.0), (140.0, 40.0)]);
        assert_eq!(events(ui), vec![("log".to_string(), r#"["start enter drop:sword end:move"]"#.to_string())]);
        let r = rect(ui, "#item");
        assert!(r[0] > 110.0 && r[0] < 170.0, "the item is in the second slot: {r:?}");
        assert_eq!(vitrail_rect(ui, s("[draggable] + [draggable]").as_ptr(), std::ptr::null_mut()), 0, "the dragged copy is gone");
        // A plain click (no move) is still a click.
        drag(&[(140.0, 40.0)]);
        vitrail_js_eval(ui, s("engine.trigger('log', log.join(' '))").as_ptr());
        assert_eq!(events(ui), vec![("log".to_string(), r#"["click"]"#.to_string())]);
        vitrail_destroy(ui);
    }
}
