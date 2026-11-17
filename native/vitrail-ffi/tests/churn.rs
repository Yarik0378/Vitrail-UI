//! A page whose content is rebuilt again and again (innerHTML from a game's state, as a card refreshes) must not grow:
//! the live heap after many rebuilds stays where it was after the first few.
use std::alloc::{GlobalAlloc, Layout, System};
use std::ffi::CString;
use std::sync::atomic::{AtomicIsize, Ordering};
use vitrail::*;

struct Counting;
static LIVE: AtomicIsize = AtomicIsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(l) };
        if !p.is_null() { LIVE.fetch_add(l.size() as isize, Ordering::Relaxed); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = unsafe { System.realloc(p, l, n) };
        if !q.is_null() { LIVE.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed); }
        q
    }
}

#[global_allocator]
static A: Counting = Counting;

fn s(v: &str) -> CString { CString::new(v).unwrap() }

/// A card page as a game draws one: the whole content rebuilt from a JSON state with innerHTML.
const PAGE: &str = r#"<!doctype html><html><head><style>
body { margin: 0; font: 14px sans-serif; color: #eee; }
.card { position: absolute; right: 10px; bottom: 12px; width: 330px; padding: 12px; border-radius: 14px; background: rgba(20,22,21,.94); }
.row { display: flex; gap: 8px; align-items: center; } .row img { width: 24px; height: 24px; }
.bar { height: 6px; background: rgba(255,255,255,.1); } .bar i { display: block; height: 6px; background: #8fbf5e; }
</style></head><body><div class="card" id="card"></div><script>
engine.on('card', s => {
  document.getElementById('card').innerHTML = `<h2>${s.title}</h2><div class="bar"><i style="width:${s.frac * 100}%"></i></div>` +
    s.rows.map(r => `<div class="row"><img src="icon.png"><b>${r.name}</b><span>${r.sub}</span><span>${r.right}</span></div>`).join('');
});
</script></body></html>"#;

fn state(k: usize) -> String {
    let rows: String = (0..12).map(|i| format!(r#"{{"name":"Row {i} {k}","sub":"line {}","right":"+{i}"}}"#, k % 7)).collect::<Vec<_>>().join(",");
    format!(r#"[{{"title":"Card {k}","frac":0.{},"rows":[{rows}]}}]"#, k % 10)
}

#[test]
fn rebuilt_content_does_not_grow_the_heap() {
    unsafe {
        let ui = vitrail_create(s(PAGE).as_ptr(), s("file:///churn.html").as_ptr(), 886, 1189, 2.5);
        vitrail_set_gpu(ui, 2);
        let mut counts = [0u32; 7];
        let mut t = 0.0;
        let mut step = |k: usize| {
            let json = s(&state(k));
            vitrail_js_call(ui, s("card").as_ptr(), json.as_ptr());
            t += 0.016;
            vitrail_tick(ui, t);
            vitrail_gpu_build(ui, counts.as_mut_ptr());
        };
        for k in 0..200 { step(k); }
        let base = LIVE.load(Ordering::Relaxed);
        for k in 200..1200 { step(k); }
        let after = LIVE.load(Ordering::Relaxed);
        let per = (after - base) as f64 / 1000.0;
        println!("live heap after 200 rebuilds {:.1} MB, after 1200 {:.1} MB: {:.1} KB a rebuild", base as f64 / 1048576.0, after as f64 / 1048576.0, per / 1024.0);
        vitrail_destroy(ui);
        assert!(per < 2048.0, "the heap grows {:.1} KB a rebuild", per / 1024.0);
    }
}
