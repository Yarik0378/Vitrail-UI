//! vitrail-ui: a C interface to Blitz (HTML + CSS without a browser) for Unity. A document is created from HTML text,
//! laid out for a viewport and painted on the CPU into a premultiplied RGBA8 buffer (sRGB bytes) that Unity uploads.
mod tiles;
mod js;
mod forms;
mod nav;
mod dev;
pub use dev::*;
pub mod gpu;
use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::net::{Bytes, NetHandler, NetProvider, Request};
use blitz_paint::paint_scene;
use blitz_traits::shell::{ColorScheme, Viewport};
use std::ffi::{CStr, c_char};
use std::sync::Arc;
use anyrender::recording::RenderCommand;
use anyrender::{PaintScene, Scene};
use anyrender::filters::FilterEffect;
use peniko::{Color, Fill, kurbo::Affine};
use blitz_dom::{BaseDocument, LocalName, NodeId, QualName, ns};

/// The last panic inside the library (message and place), for the game to show; a panic never crosses into the game.
static LAST_ERROR: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Runs an exported function so that a panic inside Blitz becomes an error message and a neutral return value instead
/// of unwinding into Unity (which would take the editor down).
fn guard<T>(name: &str, fallback: T, f: impl FnOnce() -> T) -> T {
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            previous(info);
            let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
            let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into());
            *LAST_ERROR.lock().unwrap() = format!("{msg}{at}");
        }));
    });
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(_) => {
            let mut e = LAST_ERROR.lock().unwrap();
            *e = format!("{name}: {e}");
            fallback
        }
    }
}

/// Copies the last panic's message (NUL-terminated) into `buf` and clears it; returns its length, 0 when none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_last_error(buf: *mut c_char, cap: usize) -> usize {
    let mut e = LAST_ERROR.lock().unwrap();
    let n = e.len();
    if n > 0 { unsafe { write_c(buf, cap, &e) }; e.clear(); }
    n
}

pub struct Vitrail {
    doc: HtmlDocument,
    /// The open `<select>` list, if any.
    popup: Option<forms::Popup>,
    /// The document changed in a way that may leave a select without its one chosen option.
    check_selects: bool,
    width: u32,
    height: u32,
    scale: f32,
    /// Where the page asks to blur what lies under it (`backdrop-filter`): coverage in alpha, painted with the page.
    mask: Vec<u8>,
    /// Each backdrop element's blur (σ in device pixels) and colour matrix (4×5, row-major), in paint order; the mask's
    /// red channel holds the element's number (1-based) under its coverage.
    backdrop: Vec<[f32; 21]>,
    /// Something changed since the last paint (a mutation, hover or a resize).
    dirty: bool,
    /// The element under the pressed pointer, for telling a click from a drag-off.
    pressed: Option<NodeId>,
    /// The mask scene the mask buffer was painted from: a repaint with the same one (hover, a number) skips it.
    mask_scene: Option<Scene>,
    /// A paint running on a worker thread (`vitrail_paint_start` / `vitrail_paint_poll`).
    job: Option<std::sync::mpsc::Receiver<PaintResult>>,
    /// The page's worker thread (started with the first paint): each paint's second half runs there.
    worker: Option<Worker>,
    /// The last finished page, which the next paint is compared with to find what changed.
    front: Vec<u8>,
    /// The last paint's commands per tile (tiled repaint).
    tiles: Option<tiles::TileState>,
    /// The last frame built for the GPU painter.
    gpu: gpu::Frame,
    /// Its glyph atlas, kept between frames (moved to the worker while a frame is built there).
    glyphs: gpu::GlyphCache,
    /// Paint for the game's GPU (triangles) instead of pixels.
    gpu_mode: bool,
    /// GPU edges antialiased in the triangles (no multisampling needed); off = the game multisamples.
    gpu_aa: bool,
    /// The clock CSS animations and transitions run on (seconds), set by `vitrail_tick`.
    time: f64,
    /// Set while an animation is played by moving composited layers only (its end paints the page once, crisp).
    compositing: bool,
    /// The page's JavaScript (only when it has scripts), taken out while it runs (`with_js`).
    js: Option<js::Js>,
    /// What the scripts left for the game (events, log) and the library (timers, listened event types).
    js_state: js::JsState,
    /// The page's URL (scripts' `src` are read relative to it).
    base: String,
    /// The element the scripts last heard the pointer over (for mouseover/out, mouseenter/leave).
    js_hover: Option<NodeId>,
    /// The last `vitrail_gpu_build`: ms for styles and layout, the draw list, the GPU frame, and the draw commands.
    profile: [f64; 4],
    /// The page's requests to the game (clipboard, IME).
    shell: Arc<GameShell>,
    /// The text field a press started in: the pointer drives it (caret, selection) until released.
    field_press: Option<NodeId>,
}

/// What the page asks of the "window" (the game): text it copied, a paste, the IME. Blitz calls these from its
/// default actions (Ctrl/Cmd+C, V in a text field, focusing a field); the game takes them each frame.
#[derive(Default)]
struct GameShell {
    copied: std::sync::Mutex<Option<String>>,
    paste: std::sync::atomic::AtomicBool,
    /// The IME: on while a text field has the focus, and where its caret is (CSS px: x, y, width, height).
    ime: std::sync::Mutex<(bool, [f32; 4])>,
}

impl blitz_traits::shell::ShellProvider for GameShell {
    fn request_clipboard_text(&self) -> Result<(), blitz_traits::shell::ClipboardError> {
        self.paste.store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    fn set_clipboard_text(&self, text: String) -> Result<(), blitz_traits::shell::ClipboardError> {
        *self.copied.lock().unwrap() = Some(text);
        Ok(())
    }
    fn set_ime_enabled(&self, on: bool) { self.ime.lock().unwrap().0 = on; }
    fn set_ime_cursor_area(&self, x: f32, y: f32, w: f32, h: f32) { self.ime.lock().unwrap().1 = [x, y, w, h]; }
}

/// One long-lived thread per page for the second half of each paint (a thread started per paint cost the main
/// thread ≈0.05 ms a frame while animating).
struct Worker {
    tx: std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

impl Vitrail {
    /// Runs `f` on the page's worker; its result comes back on the returned channel (nothing if it panicked, which is
    /// reported like any library panic).
    fn run(&mut self, f: impl FnOnce() -> PaintResult + Send + 'static) -> std::sync::mpsc::Receiver<PaintResult> {
        let worker = self.worker.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
            std::thread::Builder::new().name("vitrail paint".into()).spawn(move || { for job in rx { job(); } }).expect("vitrail: worker thread");
            Worker { tx }
        });
        let (done, result) = std::sync::mpsc::sync_channel(1);
        let job: Box<dyn FnOnce() + Send> = Box::new(move || {
            if let Some(r) = guard("paint worker", None, || Some(f())) { let _ = done.send(r); }
        });
        let _ = worker.tx.send(job);
        result
    }
}

/// Runs `f` with the page's JavaScript (nothing when the page has none). The runtime is taken out of the page for
/// the duration, so its native operations may reach the page through the pointer.
fn with_js<R>(ui: &mut Vitrail, f: impl FnOnce(&js::Js, *mut Vitrail) -> R) -> Option<R> {
    let js = ui.js.take()?;
    let r = f(&js, ui as *mut Vitrail);
    ui.js = Some(js);
    Some(r)
}

/// A file next to the page (`url` relative to its URL, or a file: URL / absolute path), read as text.
pub(crate) fn read_relative(ui: &Vitrail, url: &str) -> Option<String> {
    let path = if let Some(p) = url.strip_prefix("file://") { std::path::PathBuf::from(p) }
        else if url.starts_with('/') { std::path::PathBuf::from(url) }
        else {
            let base = ui.base.strip_prefix("file://").unwrap_or(&ui.base);
            std::path::Path::new(base).parent()?.join(url.split(['?', '#']).next().unwrap_or(url))
        };
    std::fs::read_to_string(path).ok()
}

/// Runs the page's `<script>`s in document order (inline text, or `src` read next to the page), then tells the page
/// it has loaded (DOMContentLoaded and load). A page without scripts gets no JavaScript runtime.
fn start_scripts(ui: &mut Vitrail) {
    // (kind, name, source): classic scripts run first in document order, then modules (deferred, as in a browser).
    let page_path = ui.base.strip_prefix("file://").unwrap_or(&ui.base).to_string();
    let page_dir = std::path::Path::new(&page_path).parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let mut import_map = String::new();
    let mut scripts: Vec<(bool, String, String)> = {
        let doc = &ui.doc;
        let mut inline = 0;
        select(doc, "script").into_iter().filter_map(|id| {
            let node = doc.get_node(id)?;
            let kind = node.attr(LocalName::from("type")).unwrap_or("").to_ascii_lowercase();
            if kind == "importmap" { import_map.push_str(&node.text_content()); return None; }
            let module = kind == "module";
            if !(kind.is_empty() || kind.contains("javascript") || module) { return None; }
            match node.attr(LocalName::from("src")) {
                Some(src) => {
                    let name = if module { page_dir.join(src).to_string_lossy().into_owned() } else { src.to_string() };
                    Some((module, name, read_relative(ui, src).unwrap_or_else(|| format!("console.error('vitrail: cannot read script {}')", src.replace('\'', "")))))
                }
                None => { inline += 1; Some((module, if module { format!("{page_path}#module{inline}") } else { "inline script".to_string() }, node.text_content())) }
            }
        }).collect()
    };
    scripts.sort_by_key(|s| s.0);
    if scripts.is_empty() { return; }
    let Some(js) = js::Js::new() else {
        ui.js_state.log.push_back((2, "vitrail: the JavaScript engine could not start".into()));
        return;
    };
    ui.js = Some(js);
    ui.js_state.wake = f64::INFINITY;
    with_js(ui, |js, p| {
        if !import_map.is_empty() { js.set_import_map(&import_map, &page_dir); }
        for (module, name, src) in &scripts { if *module { js.run_module(p, name, src) } else { js.run(p, name, src) } }
        js.run(p, "load", "globalThis.__ready = true; document.dispatchEvent(new Event('DOMContentLoaded', { bubbles: true })); window.dispatchEvent(new Event('load'));");
    });
}

/// Sends an input event to the scripts when one of them listens for it: `type` at `target`, at (x, y) CSS px.
fn js_event(ui: &mut Vitrail, kind: &str, target: NodeId, x: f32, y: f32, button: i32) {
    if ui.js.is_none() || !ui.js_state.listening.contains(kind) { return; }
    with_js(ui, |js, p| js.call(p, "__dispatch", (kind, (target.as_u64() + 1) as f64, x as f64, y as f64, button)));
}

/// CSS animations and transitions that started, repeated, ended or were canceled since the last look: their DOM events
/// (transitionrun/start/end/cancel, animationstart/iteration/end/cancel) on their elements.
fn js_animation_events(ui: &mut Vitrail, seconds: f64) {
    const KINDS: [&str; 8] = ["transitionrun", "transitionstart", "transitionend", "transitioncancel", "animationstart", "animationiteration", "animationend", "animationcancel"];
    if ui.js.is_none() || !KINDS.iter().any(|k| ui.js_state.listening.contains(*k)) { return; }
    if ui.js_state.anims.is_empty() && !ui.doc.is_animating() { return; }
    let now = ui.doc.animation_snapshot(seconds);
    let mut fire: Vec<(&'static str, NodeId, String)> = Vec::new();
    let mut next = std::collections::HashMap::with_capacity(now.len());
    for (node, transition, name, state, iteration, elapsed) in now {
        let key = (node.as_u64(), transition, name.clone());
        let prev = ui.js_state.anims.get(&key).copied();
        let detail = |n: &str| format!("{{\"{}\":{},\"elapsedTime\":{elapsed},\"pseudoElement\":\"\"}}", if transition { "propertyName" } else { "animationName" }, json_str(n));
        let ended = |p: Option<(u8, f64)>, s: u8| (s == 3 || s == 4) && p.map(|p| p.0 != s).unwrap_or(true);
        if transition {
            if prev.is_none() { fire.push(("transitionrun", node, detail(&name))); }
            if state == 1 && prev.map(|p| p.0 == 0).unwrap_or(true) { fire.push(("transitionstart", node, detail(&name))); }
            if ended(prev, state) { fire.push((if state == 3 { "transitionend" } else { "transitioncancel" }, node, detail(&name))); }
        } else {
            if state == 1 && prev.map(|p| p.0 == 0).unwrap_or(true) { fire.push(("animationstart", node, detail(&name))); }
            if state == 1 && prev.map(|p| p.0 == 1 && iteration.floor() > p.1.floor()).unwrap_or(false) { fire.push(("animationiteration", node, detail(&name))); }
            if ended(prev, state) { fire.push((if state == 3 { "animationend" } else { "animationcancel" }, node, detail(&name))); }
        }
        next.insert(key, (state, iteration));
    }
    // Gone without being seen to finish (Stylo drops finished ones): ended now.
    for ((node, transition, name), (state, _)) in &ui.js_state.anims {
        if *state <= 2 && !next.contains_key(&(*node, *transition, name.clone())) {
            let kind = if *transition { "transitionend" } else { "animationend" };
            fire.push((kind, NodeId::from_u64(*node), format!("{{\"{}\":{},\"elapsedTime\":0,\"pseudoElement\":\"\"}}", if *transition { "propertyName" } else { "animationName" }, json_str(name))));
        }
    }
    ui.js_state.anims = next;
    for (kind, node, extra) in fire {
        if !ui.js_state.listening.contains(kind) { continue; }
        with_js(ui, |js, p| js.call(p, "__dispatch", (kind, (node.as_u64() + 1) as f64, 0.0, 0.0, -1, extra)));
    }
}

/// Whether an element takes focus when pressed: a form control, a link, or one with `tabindex` or `contenteditable`.
fn focusable(doc: &BaseDocument, id: NodeId) -> bool {
    let Some(node) = doc.get_node(id) else { return false };
    let Some(el) = node.element_data() else { return false };
    let tag = el.name.local.as_ref();
    matches!(tag, "input" | "textarea" | "select" | "button") && node.attr(LocalName::from("disabled")).is_none()
        || tag == "a" && node.attr(LocalName::from("href")).is_some()
        || node.attr(LocalName::from("tabindex")).is_some()
        || node.attr(LocalName::from("contenteditable")).is_some()
}

/// Moves the focus (None: nowhere) and tells the scripts: blur/focusout on the old element, focus/focusin on the new.
fn set_focus(ui: &mut Vitrail, to: Option<NodeId>) {
    let from = ui.doc.get_focussed_node_id();
    if from == to { return; }
    match to { Some(n) => { ui.doc.set_focus_to(n); } None => ui.doc.clear_focus() }
    ui.dirty = true;
    if let Some(f) = from { for k in ["blur", "focusout"] { js_event(ui, k, f, 0.0, 0.0, -1); } }
    if let Some(t) = to { for k in ["focus", "focusin"] { js_event(ui, k, t, 0.0, 0.0, -1); } }
}

/// Collects the DOM events Blitz's default actions produce (a text field's `input`, focus moving with Tab), for the
/// scripts.
struct Collect(std::rc::Rc<std::cell::RefCell<Vec<(NodeId, &'static str)>>>);

impl blitz_dom::EventHandler for Collect {
    fn handle_event(&mut self, _: &[NodeId], event: &mut blitz_traits::events::DomEvent, _: &mut dyn blitz_dom::Document, _: &mut blitz_traits::events::EventState) {
        use blitz_traits::events::DomEventData as D;
        let kind = match &event.data { D::Input(_) => "input", D::Focus(_) => "focus", D::Blur(_) => "blur", D::FocusIn(_) => "focusin", D::FocusOut(_) => "focusout", _ => return };
        self.0.borrow_mut().push((event.target, kind));
    }
}

/// Runs a UI event through Blitz's own handling (its default actions: editing a text field, placing the caret,
/// selecting, Tab), then tells the scripts what came of it (input, focus, blur).
fn drive(ui: &mut Vitrail, event: blitz_traits::events::UiEvent) {
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let before = ui.doc.get_focussed_node_id();
    {
        let doc: &mut dyn blitz_dom::Document = &mut ui.doc;
        let mut guard = doc.inner_mut();
        let mut driver = blitz_dom::EventDriver::new(&mut *guard, Collect(events.clone()));
        driver.handle_ui_event(event);
    }
    ui.dirty = true;
    let events = events.take();
    let _ = before;
    for (n, k) in events { js_event(ui, k, n, 0.0, 0.0, -1); }
}

/// The text field (an element with an editor) holding `id`, if any.
fn text_field_of(doc: &BaseDocument, id: NodeId) -> Option<NodeId> {
    let mut c = Some(id);
    while let Some(n) = c {
        let node = doc.get_node(n)?;
        if node.element_data().is_some_and(|e| e.text_input_data().is_some()) { return Some(n); }
        c = node.parent;
    }
    None
}

/// A mouse event for Blitz at (x, y) CSS px.
fn mouse(x: f32, y: f32, held: bool) -> blitz_traits::events::BlitzPointerEvent {
    use blitz_traits::events::*;
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse, is_primary: true,
        coords: PointerCoords { page_x: x, page_y: y, screen_x: x, screen_y: y, client_x: x, client_y: y },
        button: MouseEventButton::Main,
        buttons: if held { MouseEventButtons::Primary } else { MouseEventButtons::None },
        mods: keyboard_types::Modifiers::empty(),
        details: PointerDetails::default(),
        element: Default::default(),
        active_pointers: Default::default(),
    }
}

/// `s` as a JSON string literal.
fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() { match c { '"' => o.push_str("\\\""), '\\' => o.push_str("\\\\"), c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)), c => o.push(c) } }
    o.push('"');
    o
}

/// The pointer is over another element: mouseout/over (bubbling) and mouseleave/enter (each element left or entered).
fn js_hover(ui: &mut Vitrail, x: f32, y: f32) {
    if ui.js.is_none() { return; }
    let now = ui.doc.get_hover_node_id();
    if now == ui.js_hover { return; }
    let was = std::mem::replace(&mut ui.js_hover, now);
    let path = |ui: &Vitrail, n: Option<NodeId>| { let mut v = Vec::new(); let mut c = n; while let Some(id) = c { v.push(id); c = ui.doc.get_node(id).and_then(|nd| nd.parent); } v };
    let (old, new) = (path(ui, was), path(ui, now));
    if let Some(w) = was { for k in ["pointerout", "mouseout"] { js_event(ui, k, w, x, y, -1); } }
    for &n in old.iter().filter(|n| !new.contains(n)) { for k in ["pointerleave", "mouseleave"] { js_event(ui, k, n, x, y, -1); } }
    if let Some(n) = now { for k in ["pointerover", "mouseover"] { js_event(ui, k, n, x, y, -1); } }
    for &n in new.iter().rev().filter(|n| !old.contains(n)) { for k in ["pointerenter", "mouseenter"] { js_event(ui, k, n, x, y, -1); } }
}

/// What a worker thread hands back: the new page, the rectangle that differs from the last one, the mask if its shapes
/// changed, and the backdrop elements' parameters.
struct PaintResult {
    /// In GPU mode: the frame and the glyph cache coming back from the worker.
    gpu: Option<(gpu::Frame, gpu::GlyphCache)>,
    page: Vec<u8>,
    tiles: Option<tiles::TileState>,
    dirty: Option<[u32; 4]>,
    mask: Option<Vec<u8>>,
    mask_scene: Option<Scene>,
    backdrop: Vec<[f32; 21]>,
}

/// The bounding box of the pixels that differ between two pages of w×h RGBA8 (x, y, width, height), or None.
pub(crate) fn diff_rect(old: &[u8], new: &[u8], w: usize, h: usize) -> Option<[u32; 4]> {
    if old.len() != new.len() { return Some([0, 0, w as u32, h as u32]); }
    let row = w * 4;
    let (mut y0, mut y1) = (usize::MAX, 0usize);
    let (mut x0, mut x1) = (usize::MAX, 0usize);
    for y in 0..h {
        let (a, b) = (&old[y * row..(y + 1) * row], &new[y * row..(y + 1) * row]);
        if a == b { continue; }
        y0 = y0.min(y);
        y1 = y;
        // The changed span of this row, word by word.
        let first = a.chunks_exact(4).zip(b.chunks_exact(4)).position(|(p, q)| p != q).unwrap_or(0);
        let last = w - 1 - a.chunks_exact(4).rev().zip(b.chunks_exact(4).rev()).position(|(p, q)| p != q).unwrap_or(0);
        x0 = x0.min(first);
        x1 = x1.max(last);
    }
    if y0 == usize::MAX { None } else { Some([x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32]) }
}

unsafe fn text(p: *const c_char) -> String {
    if p.is_null() { return String::new(); }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Loads linked styles, fonts and images straight from disk (file: URLs) or from data: URLs, synchronously. A game's
/// pages live next to it, so there is no network stack (and no TLS code to build for every platform).
/// How much content (CSS px) beyond each side of a scrollport a GPU frame holds, so scrolling that far needs no paint.
const GPU_SCROLL_MARGIN: f64 = 600.0;

struct FileProvider;

impl NetProvider for FileProvider {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.clone();
        match url.scheme() {
            "file" => match url.to_file_path().ok().and_then(|p| std::fs::read(p).ok()) {
                Some(bytes) => handler.bytes(url.to_string(), Bytes::from(bytes)),
                None => handler.error(url.to_string(), "file not found".into()),
            },
            "data" => match decode_data_url(url.as_str()) {
                Some(bytes) => handler.bytes(url.to_string(), Bytes::from(bytes)),
                None => handler.error(url.to_string(), "bad data url".into()),
            },
            // A game texture: a transparent placeholder of the asked size whose first pixels name it (see
            // `texture_placeholder`); the GPU painter draws the game's texture in its place.
            "texture" => {
                let (mut w, mut h) = (64u32, 64u32);
                for (k, v) in url.query_pairs() {
                    if k == "w" { w = v.parse().unwrap_or(w).clamp(1, 4096); }
                    if k == "h" { h = v.parse().unwrap_or(h).clamp(1, 4096); }
                }
                handler.bytes(url.to_string(), Bytes::from(texture_placeholder(url.path(), w, h)));
            }
            _ => handler.error(url.to_string(), "only file:, data: and texture: URLs are loaded".into()),
        }
    }
}

/// The name of a game texture as a number both sides agree on (FNV-1a, never 0).
pub fn texture_id(name: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for b in name.bytes() { h = (h ^ b as u32).wrapping_mul(0x01000193); }
    h.max(1)
}

/// Marks a texture placeholder: the first pixel's colour with zero alpha (invisible however it is drawn).
const TEXTURE_MAGIC: [u8; 4] = [0x57, 0x45, 0x42, 0x00];

/// A w×h PNG, fully transparent: pixel 0 is the marker, pixels 1 and 2 hold the texture's id in their colour (alpha
/// stays zero, so the CPU painter draws nothing).
fn texture_placeholder(name: &str, w: u32, h: u32) -> Vec<u8> {
    let id = texture_id(name.trim_start_matches('/')).to_le_bytes();
    let mut img = image::RgbaImage::new(w.max(3), h);
    img.put_pixel(0, 0, image::Rgba(TEXTURE_MAGIC));
    img.put_pixel(1, 0, image::Rgba([id[0], id[1], id[2], 0]));
    img.put_pixel(2, 0, image::Rgba([id[3], 0, 0, 0]));
    let mut out = std::io::Cursor::new(Vec::new());
    let _ = img.write_to(&mut out, image::ImageFormat::Png);
    out.into_inner()
}

/// The game texture an image stands for (its id), or 0 for an ordinary image.
fn texture_of(img: &peniko::ImageData) -> u32 {
    let d = img.data.data();
    if img.width < 3 || d.len() < 12 || d[0..4] != TEXTURE_MAGIC { return 0; }
    u32::from_le_bytes([d[4], d[5], d[6], d[8]])
}

/// The id the GPU painter uses for the game texture `name` (as in `<img src="texture:name">`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_texture_id(name: *const c_char) -> u32 {
    if name.is_null() { return 0; }
    texture_id(&unsafe { CStr::from_ptr(name) }.to_string_lossy())
}

/// Which game texture image `index` of the last frame stands for (its id), or 0 for an ordinary image.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_image_texture(ui: *mut Vitrail, index: u32) -> u32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    ui.gpu.images.get(index as usize).map(texture_of).unwrap_or(0)
}

/// `data:[<type>][;base64],<payload>`
fn decode_data_url(s: &str) -> Option<Vec<u8>> {
    let rest = s.strip_prefix("data:")?;
    let (meta, payload) = rest.split_once(',')?;
    if meta.ends_with(";base64") {
        let mut out = Vec::with_capacity(payload.len() * 3 / 4);
        let (mut acc, mut bits) = (0u32, 0u32);
        for c in payload.bytes() {
            let v = match c { b'A'..=b'Z' => c - b'A', b'a'..=b'z' => c - b'a' + 26, b'0'..=b'9' => c - b'0' + 52, b'+' | b'-' => 62, b'/' | b'_' => 63, _ => continue } as u32;
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 { bits -= 8; out.push((acc >> bits) as u8); acc &= (1 << bits) - 1; }
        }
        Some(out)
    } else {
        let b = payload.as_bytes();
        let mut out = Vec::with_capacity(b.len());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Ok(v) = u8::from_str_radix(&payload[i + 1..i + 3], 16) { out.push(v); i += 3; continue; }
            }
            out.push(b[i]);
            i += 1;
        }
        Some(out)
    }
}

impl Vitrail {
    /// Before painting for the GPU: elements whose running animations touch only transform and opacity become layers
    /// of their own (the GPU painter keeps them in textures and moves them).
    fn choose_composited(&mut self) {
        let set: std::collections::HashSet<NodeId> = if self.gpu_mode { self.doc.as_ref().compositor_animations(self.time).unwrap_or_default().into_iter().collect() } else { Default::default() };
        // Elements joining or leaving the composited set paint differently (between markers, at full opacity) without
        // any damage: retained painting must repaint them.
        let doc = self.doc.as_mut();
        if set != doc.composited {
            let mut dirty = doc.paint_dirty.lock().unwrap();
            for n in set.symmetric_difference(&doc.composited) { dirty.insert(*n); }
        }
        doc.composited = set;
    }

    /// Lays out; files arrive synchronously, so a few passes pick up styles, then the fonts and images they link.
    fn settle(&mut self) {
        for _ in 0..4 { self.doc.resolve(self.time); }
        self.doc.as_mut().resolve(self.time);
    }
}

/// Creates a document. `base_url` (file:// or http://) resolves relative links. Width and height in device pixels.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_create(html: *const c_char, base_url: *const c_char, width: u32, height: u32, scale: f32) -> *mut Vitrail {
    guard("vitrail_create", std::ptr::null_mut(), || {
    let (html, base) = unsafe { (text(html), text(base_url)) };
    let base_copy = base.clone();
    let shell = Arc::new(GameShell::default());
    let doc = HtmlDocument::from_html(&html, DocumentConfig {
        shell_provider: Some(shell.clone() as _),
        base_url: if base.is_empty() { None } else { Some(base) },
        net_provider: Some(Arc::new(FileProvider) as _),
        viewport: Some(Viewport::new(width, height, scale, ColorScheme::Light)),
        // Without a parser `innerHTML = …` parsed nothing (Blitz's default is a dummy).
        html_parser_provider: Some(Arc::new(blitz_html::HtmlProvider) as _),
        ..Default::default()
    });
    let mut ui = Box::new(Vitrail { doc, popup: None, check_selects: true, width, height, scale, mask: Vec::new(), backdrop: Vec::new(), dirty: true, pressed: None, mask_scene: None, job: None, worker: None, front: Vec::new(), tiles: None, gpu: gpu::Frame::default(), glyphs: gpu::GlyphCache::default(), gpu_mode: false, gpu_aa: false, time: 0.0, compositing: false, js: None, js_state: js::JsState::default(), base: base_copy, js_hover: None, profile: [0.0; 4], shell, field_press: None });
    ui.settle();
    start_scripts(&mut ui);
    if ui.js.is_some() { ui.settle(); ui.dirty = true; }
    Box::into_raw(ui)
    })
}

/// Changes the viewport (device pixels) and lays out again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_resize(ui: *mut Vitrail, width: u32, height: u32, scale: f32) {
    guard("vitrail_resize", (), || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    finish_job(ui);
    ui.front = Vec::new();
    ui.tiles = None;
    ui.width = width; ui.height = height; ui.scale = scale;
    ui.doc.as_mut().set_viewport(Viewport::new(width, height, scale, ColorScheme::Light));
    ui.settle();
    ui.dirty = true;
    })
}

/// Paints into `out` (width*height*4 bytes, premultiplied RGBA8, sRGB, top row first). Returns 1 on success. Also
/// paints the backdrop mask (see `vitrail_backdrop`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_render(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    guard("vitrail_render", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    forms::sync(ui);
    let (w, h, s) = (ui.width, ui.height, ui.scale as f64);
    if out.is_null() || len < (w * h * 4) as usize { return 0; }
    ui.doc.as_mut().resolve(ui.time);
    let mut rec = Scene::new();
    paint_scene(&mut rec, ui.doc.as_mut(), s, w, h, 0, 0);
    let (page, mask, backdrop) = split_backdrop(rec);
    let buf = render_to_buffer::<VelloCpuImageRenderer, _>(|scene| scene.append_scene(page, Affine::IDENTITY), w, h);
    unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), out, buf.len().min(len)) };
    if backdrop.is_empty() {
        ui.mask = Vec::new();
        ui.mask_scene = None;
    } else if ui.mask_scene.as_ref() != Some(&mask) || ui.mask.len() != (w * h * 4) as usize {
        ui.mask = render_to_buffer::<VelloCpuImageRenderer, _>(|scene| scene.append_scene(mask.clone(), Affine::IDENTITY), w, h);
        ui.mask_scene = Some(mask);
    }
    ui.backdrop = backdrop;
    ui.dirty = false;
    1
    })
}

/// 1 when the page changed since the last paint and needs painting again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_needs_paint(ui: *mut Vitrail) -> i32 {
    guard("vitrail_needs_paint", 0, || {
    unsafe { ui.as_ref() }.map(|ui| ui.dirty as i32).unwrap_or(0)
    })
}

/// Starts painting the page on a worker thread: styles, layout and the list of what to draw are made here (they need
/// the document), the pixels there. Returns 1 when a paint started, 0 when there was nothing to paint or one is still
/// running. The game keeps showing the last page until `vitrail_paint_poll` hands over the new one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_paint_start(ui: *mut Vitrail) -> i32 {
    guard("vitrail_paint_start", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    if ui.job.is_some() || !ui.dirty { return 0; }
    forms::sync(ui);
    let (w, h, s) = (ui.width, ui.height, ui.scale as f64);
    ui.doc.as_mut().resolve(ui.time);
    let mut rec = Scene::new();
    ui.choose_composited();
    paint_scene(&mut rec, ui.doc.as_mut(), s, w, h, 0, 0);
    ui.dirty = false;
    if ui.gpu_mode {
        // Triangles for the game's GPU, built on the worker with the glyph cache lent to it. backdrop-filter stays in
        // the frame as its own step (the GPU painter blurs the scene and the page under the element); no mask.
        let mut glyphs = std::mem::take(&mut ui.glyphs);
        let aa = ui.gpu_aa;
        // The last frame's buffers go along (the game has its copy): the new frame says what changed against them.
        let (old_v, old_i) = (std::mem::take(&mut ui.gpu.vertices), std::mem::take(&mut ui.gpu.indices));
        ui.job = Some(ui.run(move || {
            glyphs.overflowed = false;
            let mut frame = gpu::build(&rec, &mut glyphs, aa, old_v);
            let mut all = false;
            if glyphs.overflowed { glyphs.overflowed = false; frame = gpu::build(&rec, &mut glyphs, aa, std::mem::take(&mut frame.vertices)); all = true; }
            // Vertices: the span the pool wrote; indices: from their first difference.
            let (i0, i1) = gpu::changed_range(&old_i, &frame.indices);
            frame.changed[2] = i0;
            frame.changed[3] = i1;
            if all { frame.changed = [0, frame.vertices.len() as u32, 0, frame.indices.len() as u32]; frame.vertex_spans = vec![(0, frame.vertices.len() as u32)]; frame.dirty = vec![[0.0, 0.0, f32::MAX, f32::MAX]]; }
            PaintResult { gpu: Some((frame, glyphs)), page: Vec::new(), tiles: None, dirty: None, mask: None, mask_scene: None, backdrop: Vec::new() }
        }));
        return 1;
    }
    let (page, mask, backdrop) = split_backdrop(rec);
    let need_mask = !backdrop.is_empty() && (ui.mask_scene.as_ref() != Some(&mask) || ui.mask.len() != (w * h * 4) as usize);
    let mut buf = std::mem::take(&mut ui.front);
    let prev_tiles = ui.tiles.take();
    ui.job = Some(ui.run(move || {
        let (state, dirty) = tiles::paint(page, prev_tiles, &mut buf, w, h);
        let (mask_buf, mask_scene) = if need_mask {
            (Some(render_to_buffer::<VelloCpuImageRenderer, _>(|scene| scene.append_scene(mask.clone(), Affine::IDENTITY), w, h)), Some(mask))
        } else {
            (None, None)
        };
        PaintResult { gpu: None, page: buf, tiles: Some(state), dirty, mask: mask_buf, mask_scene, backdrop }
    }));
    1
    })
}

/// Takes a finished paint, if there is one. Copies the changed rectangle of the page, packed row after row, into `out`
/// (needs width*height*4 of that rectangle) and its x, y, width, height into `rect`; when the backdrop mask changed it is
/// copied whole into `mask` (needs the page's width*height*4). `params` gets 21 floats per backdrop element, at most
/// `max_regions`; `regions` their count. Returns 0 while the paint still runs (or none was started), otherwise
/// 1 | 2 (page changed) | 4 (mask changed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_paint_poll(
    ui: *mut Vitrail, out: *mut u8, out_len: usize, rect: *mut u32, mask: *mut u8, mask_len: usize,
    params: *mut f32, max_regions: u32, regions: *mut u32,
) -> i32 {
    guard("vitrail_paint_poll", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some(rx) = ui.job.as_ref() else { return 0 };
    let r = match rx.try_recv() {
        Ok(r) => r,
        Err(std::sync::mpsc::TryRecvError::Empty) => return 0,
        // The paint panicked on the worker (reported there): nothing to take.
        Err(std::sync::mpsc::TryRecvError::Disconnected) => { ui.job = None; ui.dirty = true; return 0; }
    };
    ui.job = None;
    let w = ui.width as usize;
    let mut flags = 1;
    let mut r = r;
    if let Some((frame, glyphs)) = r.gpu.take() {
        ui.gpu = frame;
        ui.glyphs = glyphs;
        flags |= 8;
    }
    if let Some([x, y, rw, rh]) = r.dirty {
        let need = (rw * rh * 4) as usize;
        if !out.is_null() && out_len >= need && !rect.is_null() {
            for row in 0..rh as usize {
                let from = ((y as usize + row) * w + x as usize) * 4;
                unsafe { std::ptr::copy_nonoverlapping(r.page.as_ptr().add(from), out.add(row * rw as usize * 4), rw as usize * 4) };
            }
            unsafe { std::ptr::copy_nonoverlapping([x, y, rw, rh].as_ptr(), rect, 4) };
            flags |= 2;
        }
    }
    if let Some(m) = r.mask {
        if !mask.is_null() && mask_len >= m.len() {
            unsafe { std::ptr::copy_nonoverlapping(m.as_ptr(), mask, m.len()) };
            flags |= 4;
        }
        ui.mask = m;
        ui.mask_scene = r.mask_scene;
    } else if r.backdrop.is_empty() {
        ui.mask = Vec::new();
        ui.mask_scene = None;
    }
    let n = r.backdrop.len().min(max_regions as usize);
    if !params.is_null() {
        for (i, p) in r.backdrop.iter().take(n).enumerate() {
            unsafe { std::ptr::copy_nonoverlapping(p.as_ptr(), params.add(i * 21), 21) };
        }
    }
    if !regions.is_null() { unsafe { *regions = r.backdrop.len() as u32 }; }
    ui.backdrop = r.backdrop;
    if !ui.gpu_mode {
        ui.front = r.page;
        ui.tiles = r.tiles;
    }
    flags
    })
}

/// Builds the page as triangles for the GPU painter (styles and layout first). Writes the counts of vertices, indices
/// and batches into `counts`; `vitrail_gpu_copy` then copies them out. Returns 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_build(ui: *mut Vitrail, counts: *mut u32) -> i32 {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (w, h, s) = (ui.width, ui.height, ui.scale as f64);
    forms::sync(ui);
    let prof = std::env::var_os("VITRAIL_PROF").is_some();
    let t0 = std::time::Instant::now();
    ui.doc.as_mut().resolve(ui.time);
    let t1 = t0.elapsed();
    let mut rec = Scene::new();
    ui.choose_composited();
    paint_scene(&mut rec, ui.doc.as_mut(), s, w, h, 0, 0);
    let page = rec;
    let t2 = t0.elapsed();
    ui.glyphs.overflowed = false;
    ui.gpu = gpu::build(&page, &mut ui.glyphs, ui.gpu_aa, std::mem::take(&mut ui.gpu.vertices));
    ui.profile = [t1.as_secs_f64() * 1e3, (t2 - t1).as_secs_f64() * 1e3, (t0.elapsed() - t2).as_secs_f64() * 1e3, page.commands.len() as f64];
    if prof { eprintln!("resolve {:.3} paint {:.3} build {:.3} ms, {} commands, {} new shapes", t1.as_secs_f64() * 1e3, (t2 - t1).as_secs_f64() * 1e3, (t0.elapsed() - t2).as_secs_f64() * 1e3, page.commands.len(), ui.glyphs.tess.misses); }
    // The atlas filled up midway: build once more on the fresh atlas so no quad points at a glyph that was dropped.
    if ui.glyphs.overflowed { ui.glyphs.overflowed = false; ui.gpu = gpu::build(&page, &mut ui.glyphs, ui.gpu_aa, std::mem::take(&mut ui.gpu.vertices)); }
    // Built here without the last frame: everything counts as changed.
    ui.gpu.changed = [0, ui.gpu.vertices.len() as u32, 0, ui.gpu.indices.len() as u32];
    ui.gpu.vertex_spans = vec![(0, ui.gpu.vertices.len() as u32)];
    ui.gpu.dirty = vec![[0.0, 0.0, f32::MAX, f32::MAX]];
    ui.dirty = false;
    if !counts.is_null() {
        let c = [ui.gpu.vertices.len() as u32, ui.gpu.indices.len() as u32, ui.gpu.batches.len() as u32, ui.gpu.ramps.len() as u32, ui.gpu.images.len() as u32, ui.gpu.ops.len() as u32, ui.glyphs.dirty as u32];
        unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), counts, 7) };
    }
    1
}

/// Switches between painting pixels on the CPU (0, the default) and triangles for the game's GPU (1 multisampled by the game, 2 with
/// antialiased edges in the triangles, drawn without multisampling). In GPU mode
/// `vitrail_paint_poll` returns 8 in its flags when a frame is ready; its parts are read with `vitrail_gpu_counts` and the
/// other `vitrail_gpu_*` functions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_set_gpu(ui: *mut Vitrail, on: i32) {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    finish_job(ui);
    ui.gpu_mode = on != 0;
    ui.gpu_aa = on == 2;
    // Scroll containers are painted unscrolled with room around them, and scrolled on the GPU.
    ui.doc.as_mut().gpu_scroll_margin = if ui.gpu_mode { Some(GPU_SCROLL_MARGIN) } else { None };
    ui.dirty = true;
    ui.front = Vec::new();
    ui.tiles = None;
}

/// The counts of the last frame (vertices, indices, batches, ramps, images, ops, atlas changed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_counts(ui: *mut Vitrail, counts: *mut u32) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    if counts.is_null() { return 0; }
    let c = [ui.gpu.vertices.len() as u32, ui.gpu.indices.len() as u32, ui.gpu.batches.len() as u32, ui.gpu.ramps.len() as u32, ui.gpu.images.len() as u32, ui.gpu.ops.len() as u32, ui.glyphs.dirty as u32];
    unsafe { std::ptr::copy_nonoverlapping(c.as_ptr(), counts, 7) };
    1
}

/// Copies the glyph atlas (ATLAS×ATLAS coverage bytes) when it changed since the last copy; returns 1 when it did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_atlas(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    if !ui.glyphs.dirty || out.is_null() || len < ui.glyphs.atlas.len() { return 0; }
    unsafe { std::ptr::copy_nonoverlapping(ui.glyphs.atlas.as_ptr(), out, ui.glyphs.atlas.len()) };
    ui.glyphs.dirty = false;
    1
}

/// Copies the ops of the last built frame (20 bytes each). Returns 1 when `out` was big enough.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_ops(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.ops.len() * std::mem::size_of::<gpu::Op>();
    if len < n || out.is_null() { return 0; }
    unsafe { std::ptr::copy_nonoverlapping(ui.gpu.ops.as_ptr() as *const u8, out, n) };
    1
}

/// Advances the clock of CSS animations and transitions to `seconds` (any steadily rising clock, e.g. the game's
/// unscaled time). Returns 1 while something animates: the page is then marked to paint again, so calling this every
/// frame before `vitrail_paint_start` plays them. Returns 0 when nothing moves (no work is done then).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_tick(ui: *mut Vitrail, seconds: f64) -> i32 {
    guard("vitrail_tick", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    ui.time = seconds;
    // The scripts' timers and animation frames that are due run on this clock.
    if ui.js.is_some() && ui.js_state.wake <= seconds * 1000.0 {
        with_js(ui, |js, p| js.call(p, "__tick", (seconds * 1000.0,)));
    }
    js_animation_events(ui, seconds);
    if !ui.doc.is_animating() {
        // An animation played on layers just ended: paint once so the page is drawn as it now stands (crisp).
        if ui.compositing { ui.compositing = false; ui.dirty = true; }
        return 0;
    }
    // Only layers move (every running animation touches transform or opacity of an element composited in the frame
    // on screen): restyle for the new time and let the GPU move them (vitrail_layer_states); no paint.
    if ui.gpu_mode && !ui.dirty {
        if let Some(nodes) = ui.doc.as_ref().compositor_animations(seconds) {
            let layered = |n: &blitz_dom::NodeId| ui.gpu.layers.iter().any(|l| l.id == n.as_u64() && l.inline == 0);
            if !nodes.is_empty() && nodes.iter().all(layered) {
                ui.doc.as_mut().resolve(seconds);
                ui.compositing = true;
                return 2;
            }
        }
    }
    ui.dirty = true;
    1
    })
}

/// Where each composited layer of the frame on screen is now: 7 floats per layer, in the order of `vitrail_gpu_layers`:
/// the matrix (a b c d e f) that takes the layer as painted to where it is now, and its opacity. Returns the number of
/// layers (call with a null `out` to learn it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_layer_states(ui: *mut Vitrail, out: *mut f32, len: usize) -> i32 {
    guard("vitrail_layer_states", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.layers.len();
    if out.is_null() || len < n * 7 { return n as i32; }
    let doc = ui.doc.as_ref();
    for (i, l) in ui.gpu.layers.iter().enumerate() {
        let mut state = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
        if let Some(node) = doc.get_node(blitz_dom::NodeId::from_u64(l.id)) {
            let aff = |c: [f32; 6]| Affine::new([c[0] as f64, c[1] as f64, c[2] as f64, c[3] as f64, c[4] as f64, c[5] as f64]);
            let (world, local) = (aff(l.world), aff(l.local));
            let now = node.transform().as_deref().copied().unwrap_or_default();
            // world = A · local, so where it is now: A · now, and the move from as painted: A · now · local⁻¹ · A⁻¹.
            let a = world * local.inverse();
            let delta = a * now * local.inverse() * a.inverse();
            let c = delta.as_coeffs();
            let opacity = node.primary_styles().map(|s| s.get_effects().opacity).unwrap_or(1.0);
            state = [c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32, c[4] as f32, c[5] as f32, opacity];
        }
        unsafe { std::ptr::copy_nonoverlapping(state.as_ptr(), out.add(i * 7), 7) };
    }
    n as i32
    })
}

/// The page rectangles the last frame changed (up to four; x0 y0 x1 y1 device px each, clamped to the page) into `out`
/// (room for `max`). Returns how many: the game may redraw only those rectangles of what it keeps; 0 when nothing changed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_dirty(ui: *mut Vitrail, out: *mut f32, max: usize) -> i32 {
    guard("vitrail_gpu_dirty", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    if out.is_null() { return 0; }
    let (w, h) = (ui.width as f32, ui.height as f32);
    let mut n = 0;
    for d in ui.gpu.dirty.iter().take(max) {
        let r = [d[0].clamp(0.0, w), d[1].clamp(0.0, h), d[2].clamp(0.0, w), d[3].clamp(0.0, h)];
        if r[0] >= r[2] || r[1] >= r[3] { continue; }
        unsafe { std::ptr::copy_nonoverlapping(r.as_ptr(), out.add(n * 4), 4) };
        n += 1;
    }
    n as i32
    })
}

/// The vertex spans the last frame wrote (start, end pairs, at most 32) into `out` (2 u32 each); returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_vertex_spans(ui: *mut Vitrail, out: *mut u32, max: usize) -> i32 {
    guard("vitrail_gpu_vertex_spans", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.vertex_spans.len().min(max);
    for (i, s) in ui.gpu.vertex_spans.iter().take(n).enumerate() {
        unsafe { *out.add(i * 2) = s.0; *out.add(i * 2 + 1) = s.1; }
    }
    n as i32
    })
}

/// What the last frame changed against the one before (vertices first..end, indices first..end) into `out[4]`: the
/// game may upload only those ranges when the counts stayed the same. Returns 1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_changed(ui: *mut Vitrail, out: *mut u32) -> i32 {
    guard("vitrail_gpu_changed", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    if out.is_null() { return 0; }
    unsafe { std::ptr::copy_nonoverlapping(ui.gpu.changed.as_ptr(), out, 4) };
    1
    })
}

/// Copies the image atlas (IMAGE_ATLAS² RGBA8, straight or premultiplied per image as `Vertex::p` says) when it changed
/// since the last copy; returns 1 when it did.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_image_atlas(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    guard("vitrail_gpu_image_atlas", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let a = &mut ui.glyphs.tess.image_atlas;
    if !a.dirty || out.is_null() || len < a.pixels.len() { return 0; }
    unsafe { std::ptr::copy_nonoverlapping(a.pixels.as_ptr(), out, a.pixels.len()) };
    a.dirty = false;
    1
    })
}

/// Copies the shader clips of the last built frame (16 floats each: x0 y0 x1 y1, rx ry per corner from top-left, the
/// scroll container it moves with, three unused) when `out` holds them all. Returns how many there are.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_clips(ui: *mut Vitrail, out: *mut f32, len: usize) -> i32 {
    guard("vitrail_gpu_clips", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.clips.len();
    if !out.is_null() && len >= n * 16 { unsafe { std::ptr::copy_nonoverlapping(ui.gpu.clips.as_ptr() as *const f32, out, n * 16) }; }
    n as i32
    })
}

/// Copies the composited layers of the last built frame (`gpu::CompLayer`, 80 bytes each) when `out` holds them all.
/// Returns how many there are.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_layers(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    guard("vitrail_gpu_layers", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.layers.len();
    let bytes = n * std::mem::size_of::<gpu::CompLayer>();
    if !out.is_null() && len >= bytes { unsafe { std::ptr::copy_nonoverlapping(ui.gpu.layers.as_ptr() as *const u8, out, bytes) }; }
    n as i32
    })
}

/// Copies the filter steps of the last built frame (`gpu::FilterStep`, 128 bytes each) when `out` holds them all.
/// Returns how many there are (call with a null `out` to learn the count).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_filters(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    guard("vitrail_gpu_filters", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.filters.len();
    let bytes = n * std::mem::size_of::<gpu::FilterStep>();
    if !out.is_null() && len >= bytes { unsafe { std::ptr::copy_nonoverlapping(ui.gpu.filters.as_ptr() as *const u8, out, bytes) }; }
    n as i32
    })
}

/// Copies the gradient ramps of the last built frame (256 RGBA8 per row). Returns 1 when `out` was big enough.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_ramps(ui: *mut Vitrail, out: *mut u32, len: usize) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.ramps.len() * gpu::RAMP;
    if len < n || out.is_null() { return 0; }
    for (i, r) in ui.gpu.ramps.iter().enumerate() { unsafe { std::ptr::copy_nonoverlapping(r.as_ptr(), out.add(i * gpu::RAMP), gpu::RAMP) }; }
    1
}

/// An image of the last built frame (batch.texture = index + 1): its id (the same pixels keep it), size and whether its
/// colours are premultiplied, in `info` (id low, id high, width, height, premultiplied); with `out`, its RGBA8 pixels.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_image(ui: *mut Vitrail, index: u32, info: *mut u32, out: *mut u8, len: usize) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let Some(img) = ui.gpu.images.get(index as usize) else { return 0 };
    let id = img.data.id();
    if !info.is_null() {
        let v = [id as u32, (id >> 32) as u32, img.width, img.height, matches!(img.alpha_type, peniko::ImageAlphaType::AlphaPremultiplied) as u32];
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), info, 5) };
    }
    if !out.is_null() {
        let bytes = img.data.data();
        let n = (img.width * img.height * 4) as usize;
        if len < n || bytes.len() < n { return 0; }
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, n) };
    }
    1
}

/// Copies the last built frame: vertices (36 bytes each), indices (u32) and batches (20 bytes each). Returns 1 when the
/// buffers were big enough.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_copy(ui: *mut Vitrail, vertices: *mut u8, vertices_len: usize, indices: *mut u32, indices_len: usize, batches: *mut u8, batches_len: usize) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let f = &ui.gpu;
    let vb = f.vertices.len() * std::mem::size_of::<gpu::Vertex>();
    let bb = f.batches.len() * std::mem::size_of::<gpu::Batch>();
    if (!vertices.is_null() && vertices_len < vb) || indices_len < f.indices.len() || batches_len < bb { return 0; }
    unsafe {
        // A null `vertices` skips them (the game copies only the written spans with vitrail_gpu_vertex_range).
        if !vertices.is_null() { std::ptr::copy_nonoverlapping(f.vertices.as_ptr() as *const u8, vertices, vb); }
        std::ptr::copy_nonoverlapping(f.indices.as_ptr(), indices, f.indices.len());
        std::ptr::copy_nonoverlapping(f.batches.as_ptr() as *const u8, batches, bb);
    }
    1
}

/// Copies vertices start..end of the last frame to the same place in `out` (a mirror of the whole vertex buffer,
/// `len` bytes). Returns 1 when it fit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_vertex_range(ui: *mut Vitrail, out: *mut u8, len: usize, start: u32, end: u32) -> i32 {
    guard("vitrail_gpu_vertex_range", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let size = std::mem::size_of::<gpu::Vertex>();
    let (a, b) = (start as usize, (end as usize).min(ui.gpu.vertices.len()));
    if out.is_null() || a >= b || len < b * size { return 0; }
    unsafe { std::ptr::copy_nonoverlapping((ui.gpu.vertices.as_ptr() as *const u8).add(a * size), out.add(a * size), (b - a) * size) };
    1
    })
}

/// Waits for a running paint (before the document goes away or changes size).
fn finish_job(ui: &mut Vitrail) {
    if let Some(rx) = ui.job.take() { let _ = rx.recv(); }
}

fn doc_of(ui: &mut Vitrail) -> &mut BaseDocument { &mut ui.doc }

/// Every element matching `selector` (CSS), in document order.
fn select(doc: &BaseDocument, selector: &str) -> Vec<NodeId> {
    // `#id` from the document's id index (a full query walks every node: ≈0.1 ms on a 2000-cell page).
    if let Some(id) = selector.strip_prefix('#').filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')) {
        return doc.get_element_by_id(id).into_iter().collect();
    }
    doc.query_selector_all(selector).map(|ids| ids.into_iter().collect()).unwrap_or_default()
}

fn qual(name: &str) -> QualName { QualName::new(None, ns!(), LocalName::from(name)) }

/// Replaces the text of every element matching `selector`. Returns how many matched.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_set_text(ui: *mut Vitrail, selector: *const c_char, value: *const c_char) -> i32 {
    guard("vitrail_set_text", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (sel, value) = unsafe { (text(selector), text(value)) };
    let ids = select(doc_of(ui), &sel);
    let mut changed = false;
    {
        let doc = doc_of(ui);
        for &id in &ids {
            let node = doc.get_node(id);
            let same = node.map(|n| n.children.len() == 1 && doc.get_node(n.children[0]).and_then(|c| c.text_data()).map(|t| t.content == value).unwrap_or(false)).unwrap_or(false);
            if same { continue; }
            let mut m = doc.mutate();
            m.remove_and_drop_all_children(id);
            let t = m.create_text_node(&value);
            m.append_children(id, &[t]);
            changed = true;
        }
    }
    if changed { ui.dirty = true; }
    ids.len() as i32
    })
}

/// Sets (or, with a null value, removes) an attribute on every element matching `selector`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_set_attr(ui: *mut Vitrail, selector: *const c_char, name: *const c_char, value: *const c_char) -> i32 {
    guard("vitrail_set_attr", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (sel, name) = unsafe { (text(selector), text(name)) };
    let remove = value.is_null();
    let value = unsafe { text(value) };
    let ids = select(doc_of(ui), &sel);
    let mut changed = false;
    {
        let doc = doc_of(ui);
        for &id in &ids {
            let current = doc.get_node(id).and_then(|n| n.attr(LocalName::from(name.as_str()))).map(|s| s.to_string());
            if remove {
                if current.is_none() { continue; }
                doc.mutate().clear_attribute(id, qual(&name));
            } else {
                if current.as_deref() == Some(value.as_str()) { continue; }
                doc.mutate().set_attribute(id, qual(&name), &value);
            }
            changed = true;
        }
    }
    if changed { ui.dirty = true; }
    ids.len() as i32
    })
}

/// Adds (on = 1) or removes (on = 0) a class on every element matching `selector`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_set_class(ui: *mut Vitrail, selector: *const c_char, class: *const c_char, on: i32) -> i32 {
    guard("vitrail_set_class", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (sel, class) = unsafe { (text(selector), text(class)) };
    let ids = select(doc_of(ui), &sel);
    let mut changed = false;
    {
        let doc = doc_of(ui);
        for &id in &ids {
            let current = doc.get_node(id).and_then(|n| n.attr(LocalName::from("class"))).unwrap_or("").to_string();
            let mut classes: Vec<&str> = current.split_whitespace().collect();
            let has = classes.contains(&class.as_str());
            if (on != 0) == has { continue; }
            if on != 0 { classes.push(&class); } else { classes.retain(|c| *c != class); }
            let joined = classes.join(" ");
            doc.mutate().set_attribute(id, qual("class"), &joined);
            changed = true;
        }
    }
    if changed { ui.dirty = true; }
    ids.len() as i32
    })
}

/// Sets one inline style property (`width`, `--progress`, …) on every element matching `selector`; an empty value
/// removes it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_set_style(ui: *mut Vitrail, selector: *const c_char, property: *const c_char, value: *const c_char) -> i32 {
    guard("vitrail_set_style", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (sel, prop, value) = unsafe { (text(selector), text(property), text(value)) };
    let ids = select(doc_of(ui), &sel);
    {
        let doc = doc_of(ui);
        for &id in &ids {
            if value.is_empty() { doc.mutate().remove_style_property(id, &prop); } else { doc.mutate().set_style_property(id, &prop, &value); }
        }
    }
    if !ids.is_empty() { ui.dirty = true; }
    ids.len() as i32
    })
}

/// The pointer moved to (x, y) in device pixels. Returns 1 when the hovered element changed (`:hover` restyles).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_pointer_move(ui: *mut Vitrail, x: f32, y: f32) -> i32 {
    guard("vitrail_pointer_move", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let s = ui.scale.max(0.01);
    ui.doc.as_mut().resolve(ui.time);
    let changed = doc_of(ui).set_hover_to(x / s, y / s);
    if changed { ui.dirty = true; }
    if ui.field_press.is_some() { drive(ui, blitz_traits::events::UiEvent::PointerMove(mouse(x / s, y / s, true))); }
    if ui.popup.is_some() {
        if let Some(i) = ui.doc.get_hover_node_id().and_then(|h| forms::row_at(ui, h)) { forms::highlight(ui, i); }
    }
    if ui.js.is_some() {
        js_hover(ui, x / s, y / s);
        if let Some(h) = ui.doc.get_hover_node_id() { for k in ["pointermove", "mousemove"] { js_event(ui, k, h, x / s, y / s, -1); } }
    }
    changed as i32
    })
}

/// The wheel turned over (x, y) (device px) by (dx, dy) device px (positive dy scrolls down): the scrollable element
/// under the pointer scrolls, then its ancestors once it can't. Returns 1 when something moved.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_scroll(ui: *mut Vitrail, x: f32, y: f32, dx: f32, dy: f32) -> i32 {
    guard("vitrail_scroll", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let s = ui.scale.max(0.01);
    // Layout is current unless something changed since the last paint.
    if ui.dirty { ui.doc.as_mut().resolve(ui.time); }
    let doc = doc_of(ui);
    let Some(node) = doc.element_from_point(x / s, y / s) else { return 0 };
    let moved = doc.scroll_node_by_has_changed(node, -(dx / s) as f64, -(dy / s) as f64, |_| {});
    if !moved { return 0; }
    // GPU-scrolled content is painted unscrolled, so the GPU moves it at once whatever was painted (2: read
    // vitrail_scroll_states): scrolling never waits for a paint. Once less than half the held content is left ahead, the
    // page paints again around the new offset on the worker, so the new content is there before the edge is reached
    // (a fast fling past it shows the edge for a frame or two rather than stopping). Other scrolling paints (1).
    if !cfg!(feature = "scrollbars") && ui.gpu_mode && !ui.gpu.scrolls.is_empty() {
        if !gpu_scroll_holds(ui, 0.5) { ui.dirty = true; }
        return 2;
    }
    ui.dirty = true;
    1
    })
}

/// Each GPU-scrolled container's scroll offset now, with its enclosing ones' (device px).
fn scroll_offsets(ui: &Vitrail) -> Vec<[f32; 2]> {
    let doc = ui.doc.as_ref();
    let s = ui.scale as f64;
    let mut out: Vec<[f32; 2]> = Vec::with_capacity(ui.gpu.scrolls.len());
    for sc in &ui.gpu.scrolls {
        let own = doc.get_node(blitz_dom::NodeId::from_u64(sc.id)).map(|n| { let p = n.scroll_offset(); (p.x, p.y) }).unwrap_or((0.0, 0.0));
        // Never past the content the frame holds: a fling faster than the worker paints waits at its edge for the next
        // frame instead of showing blank space (as the browser keeps checkerboarding rare, this keeps it out).
        let m = (GPU_SCROLL_MARGIN * s) as f32 - 1.0;
        let held = |v: f64, at: f32| ((v * s) as f32).clamp(at - m, at + m);
        let mut o = [held(own.0, sc.offset[0]), held(own.1, sc.offset[1])];
        if sc.parent > 0 { let p = out[sc.parent as usize - 1]; o[0] += p[0]; o[1] += p[1]; }
        out.push(o);
    }
    out
}

/// Whether every GPU-scrolled container is still within the content the frame holds around it.
fn gpu_scroll_holds(ui: &Vitrail, share: f32) -> bool {
    let doc = ui.doc.as_ref();
    let m = (GPU_SCROLL_MARGIN * ui.scale as f64) as f32 * share - 1.0;
    ui.gpu.scrolls.iter().all(|sc| {
        let own = doc.get_node(blitz_dom::NodeId::from_u64(sc.id)).map(|n| { let p = n.scroll_offset(); (p.x, p.y) }).unwrap_or((0.0, 0.0));
        let (x, y) = ((own.0 * ui.scale as f64) as f32, (own.1 * ui.scale as f64) as f32);
        (x - sc.offset[0]).abs() <= m && (y - sc.offset[1]).abs() <= m
    })
}

/// The scroll offsets the GPU moves each scrolled container's content by (2 floats each, device px, its enclosing
/// containers' included), in the order of `vitrail_gpu_scrolls`. Returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_scroll_states(ui: *mut Vitrail, out: *mut f32, max: usize) -> i32 {
    guard("vitrail_scroll_states", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let o = scroll_offsets(ui);
    let n = o.len().min(max);
    if !out.is_null() { unsafe { std::ptr::copy_nonoverlapping(o.as_ptr() as *const f32, out, n * 2) }; }
    n as i32
    })
}

/// What the overlay scrollbars of the GPU-scrolled containers need, in the order of `vitrail_gpu_scrolls`: 6 floats each
/// (device px): the container's own scroll offset (x, y), its visible size (client width, height) and its content's
/// size (scroll width, height). Returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_scroll_extents(ui: *mut Vitrail, out: *mut f32, max: usize) -> i32 {
    guard("vitrail_scroll_extents", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let doc = ui.doc.as_ref();
    let s = ui.scale;
    let n = ui.gpu.scrolls.len().min(max);
    if out.is_null() { return n as i32; }
    for (i, sc) in ui.gpu.scrolls.iter().take(n).enumerate() {
        let v = doc.get_node(blitz_dom::NodeId::from_u64(sc.id)).map(|nd| {
            let o = nd.scroll_offset();
            [o.x as f32 * s, o.y as f32 * s, nd.client_width() * s, nd.client_height() * s, nd.scroll_width() * s, nd.scroll_height() * s]
        }).unwrap_or([0.0; 6]);
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out.add(i * 6), 6) };
    }
    n as i32
    })
}

/// Copies the GPU-scrolled containers of the last frame (`gpu::ScrollInfo`, 40 bytes: node, enclosing index, offset
/// as painted, clip box) when `out` holds them; returns how many.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_gpu_scrolls(ui: *mut Vitrail, out: *mut u8, len: usize) -> i32 {
    guard("vitrail_gpu_scrolls", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.gpu.scrolls.len();
    let bytes = n * std::mem::size_of::<gpu::ScrollInfo>();
    if !out.is_null() && len >= bytes { unsafe { std::ptr::copy_nonoverlapping(ui.gpu.scrolls.as_ptr() as *const u8, out, bytes) }; }
    n as i32
    })
}

/// The cursor the page asks for at the pointer (CSS `cursor`): 0 default, 1 pointer (a hand), 2 text, 3 another one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_cursor(ui: *mut Vitrail) -> i32 {
    guard("vitrail_cursor", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    match doc_of(ui).get_cursor() {
        None => 0,
        Some(c) => match format!("{c:?}").as_str() {
            "Default" => 0,
            "Pointer" => 1,
            "Text" | "VerticalText" => 2,
            _ => 3,
        },
    }
    })
}

/// 1 when (x, y) in device pixels is over a part of the page that takes the pointer (anything but the bare page and
/// elements with `pointer-events: none`), so the game knows whether a click is the HUD's or the world's.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_hit(ui: *mut Vitrail, x: f32, y: f32) -> i32 {
    guard("vitrail_hit", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let s = ui.scale.max(0.01);
    let doc = doc_of(ui);
    match doc.element_from_point(x / s, y / s) {
        Some(id) => {
            let n = doc.get_node(id);
            let tag = n.and_then(|n| n.element_data()).map(|e| e.name.local.to_string()).unwrap_or_default();
            (tag != "html" && tag != "body") as i32
        }
        None => 0,
    }
    })
}

/// The pointer was pressed at (x, y) in device pixels (`:active` on the element under it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_pointer_down(ui: *mut Vitrail, x: f32, y: f32) {
    guard("vitrail_pointer_down", (), || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    let s = ui.scale.max(0.01);
    let doc = doc_of(ui);
    doc.set_hover_to(x / s, y / s);
    let hit = doc.element_from_point(x / s, y / s);
    // The open select list takes the press: on a row it chooses on release; elsewhere it closes and the press ends
    // there (as in a browser).
    if ui.popup.is_some() {
        let inside = hit.is_some_and(|h| forms::in_popup(ui, h));
        if !inside { forms::close(ui); ui.pressed = None; return; }
        ui.pressed = hit;
        return;
    }
    let doc = doc_of(ui);
    if doc.active_node() { ui.dirty = true; }
    ui.pressed = hit;
    if let Some(h) = hit { for k in ["pointerdown", "mousedown"] { js_event(ui, k, h, x / s, y / s, 0); } }
    // A press focuses the field, button or [tabindex] element under it (keys go there), or takes focus away.
    let target = hit.and_then(|h| { let mut c = Some(h); while let Some(n) = c { if focusable(&ui.doc, n) { return Some(n); } c = ui.doc.get_node(n).and_then(|nd| nd.parent); } None });
    set_focus(ui, target);
    // The mouse shows no focus ring (`:focus-visible`) except in text fields, as in browsers.
    if let Some(t) = target { if text_field_of(&ui.doc, t).is_none() { ui.doc.set_focus_ring(t, false); } }
    // A press on a select opens its list.
    if let Some(sel) = hit.and_then(|h| forms::select_of(&ui.doc, h)) { forms::open(ui, sel); ui.pressed = None; return; }
    // A press in a text field: Blitz places the caret (a double press selects a word, a triple the line) and the
    // drag that follows selects.
    // A press on an overlay scrollbar's thumb: Blitz drags it.
    ui.field_press = ui.doc.hovered_scrollbar().map(|b| b.node_id).or_else(|| hit.and_then(|h| text_field_of(&ui.doc, h)));
    if ui.field_press.is_some() { drive(ui, blitz_traits::events::UiEvent::PointerDown(mouse(x / s, y / s, true))); }
    })
}

/// The pointer was released at (x, y). When it is released over the element it was pressed on (or one inside it), the
/// nearest element with a `data-action` (or else an `id`) is the click: its action and id are written, NUL-terminated,
/// into `action` and `id` (buffers of `cap` bytes). Returns 1 for a click, 0 otherwise.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_pointer_up(ui: *mut Vitrail, x: f32, y: f32, action: *mut c_char, id: *mut c_char, cap: usize) -> i32 {
    guard("vitrail_pointer_up", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let s = ui.scale.max(0.01);
    let pressed = ui.pressed.take();
    if ui.popup.is_some() {
        let doc = doc_of(ui);
        if doc.unactive_node() { ui.dirty = true; }
        let row = doc_of(ui).element_from_point(x / s, y / s).and_then(|h| forms::row_at(ui, h));
        if let (Some(i), Some(_)) = (row, pressed) { forms::highlight(ui, i); forms::pick(ui); }
        return 0;
    }
    if ui.field_press.take().is_some() { drive(ui, blitz_traits::events::UiEvent::PointerUp(mouse(x / s, y / s, false))); }
    let doc = doc_of(ui);
    if doc.unactive_node() { ui.dirty = true; }
    let doc = doc_of(ui);
    let Some(hit) = doc.element_from_point(x / s, y / s) else { return 0 };
    if ui.js.is_some() {
        for k in ["pointerup", "mouseup"] { js_event(ui, k, hit, x / s, y / s, 0); }
        // A click goes to the nearest element holding both where the press started and where it ended.
        if let Some(p) = pressed {
            let mut c = Some(hit);
            while let Some(n) = c { if is_inside(&ui.doc, p, n) { break; } c = ui.doc.get_node(n).and_then(|nd| nd.parent); }
            if let Some(t) = c { js_event(ui, "click", t, x / s, y / s, 0); }
        }
    }
    let doc = doc_of(ui);
    // Walk up from the released element to find the clickable one; the press must have started inside it.
    let mut cur = Some(hit);
    while let Some(nid) = cur {
        let Some(node) = doc.get_node(nid) else { break };
        let act = node.attr(LocalName::from("data-action")).map(|s| s.to_string());
        let eid = node.attr(LocalName::from("id")).map(|s| s.to_string());
        if act.is_some() || eid.is_some() {
            let started_inside = pressed.map(|p| is_inside(doc, p, nid)).unwrap_or(false);
            if !started_inside { return 0; }
            unsafe {
                write_c(action, cap, act.as_deref().unwrap_or(""));
                write_c(id, cap, eid.as_deref().unwrap_or(""));
            }
            return 1;
        }
        cur = node.parent;
    }
    0
    })
}

/// The element under the pointer: the nearest one with a `data-action` (or an `id`) from the hovered element up.
/// Writes its action and id (NUL-terminated, `cap` bytes each); returns 1 when there is one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_hovered(ui: *mut Vitrail, action: *mut c_char, id: *mut c_char, cap: usize) -> i32 {
    guard("vitrail_hovered", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let doc = doc_of(ui);
    let mut cur = doc.get_hover_node_id();
    while let Some(nid) = cur {
        let Some(node) = doc.get_node(nid) else { break };
        let act = node.attr(LocalName::from("data-action"));
        let eid = node.attr(LocalName::from("id"));
        if act.is_some() || eid.is_some() {
            unsafe {
                write_c(action, cap, act.unwrap_or(""));
                write_c(id, cap, eid.unwrap_or(""));
            }
            return 1;
        }
        cur = node.parent;
    }
    0
    })
}

/// The border box of the first element matching `selector`, in device pixels from the page's top-left: x, y, width,
/// height into `out`. Returns 1 when found.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_rect(ui: *mut Vitrail, selector: *const c_char, out: *mut f32) -> i32 {
    guard("vitrail_rect", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let sel = unsafe { text(selector) };
    let s = ui.scale;
    ui.doc.as_mut().resolve(ui.time);
    let doc = doc_of(ui);
    let Some(&id) = select(doc, &sel).first() else { return 0 };
    let Some(r) = doc.get_client_bounding_rect(id) else { return 0 };
    if out.is_null() { return 1; }
    let v = [r.x as f32 * s, r.y as f32 * s, r.width as f32 * s, r.height as f32 * s];
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, 4) };
    1
    })
}

fn is_inside(doc: &BaseDocument, node: NodeId, ancestor: NodeId) -> bool {
    let mut cur = Some(node);
    while let Some(n) = cur {
        if n == ancestor { return true; }
        cur = doc.get_node(n).and_then(|n| n.parent);
    }
    false
}

unsafe fn write_c(buf: *mut c_char, cap: usize, s: &str) {
    if buf.is_null() || cap == 0 { return; }
    let bytes = s.as_bytes();
    let n = bytes.len().min(cap - 1);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, buf, n);
        *buf.add(n) = 0;
    }
}

/// The backdrop mask of the last paint: copies width*height*4 bytes into `out` (coverage in alpha, the element's
/// 1-based number in red) when there is one, and 21 floats per element into `params` (blur σ in device px, then the 4×5
/// colour matrix), at most `max_regions` of them. Returns how many elements ask for a backdrop filter.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_backdrop(ui: *mut Vitrail, out: *mut u8, len: usize, params: *mut f32, max_regions: u32) -> u32 {
    guard("vitrail_backdrop", 0, || {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let n = ui.backdrop.len().min(max_regions as usize);
    if !params.is_null() {
        for (i, p) in ui.backdrop.iter().take(n).enumerate() {
            unsafe { std::ptr::copy_nonoverlapping(p.as_ptr(), params.add(i * 21), 21) };
        }
    }
    if n > 0 && !out.is_null() && len >= ui.mask.len() {
        unsafe { std::ptr::copy_nonoverlapping(ui.mask.as_ptr(), out, ui.mask.len()) };
    }
    ui.backdrop.len() as u32
    })
}

fn identity_backdrop() -> [f32; 21] {
    let mut p = [0.0; 21];
    for i in 0..4 { p[1 + i * 5 + i] = 1.0; }
    p
}

/// The recording with its shared pieces (`RenderCommand::Group`, elements Blitz painted earlier) written out in place,
/// for painters that walk plain command lists.
pub(crate) fn flatten(scene: Scene) -> Scene {
    if !scene.commands.iter().any(|c| matches!(c, RenderCommand::Group(_))) { return scene; }
    fn push(out: &mut Vec<RenderCommand>, cmds: &[RenderCommand]) {
        for c in cmds { match c { RenderCommand::Group(g) => push(out, g), c => out.push(c.clone()) } }
    }
    let mut out = Vec::with_capacity(scene.commands.len() * 4);
    push(&mut out, &scene.commands);
    Scene { tolerance: scene.tolerance, commands: out }
}

/// Splits a recorded page into the page itself (backdrop filters removed: the game picture under the page isn't
/// Blitz's to see) and a mask scene holding only the shapes of elements with a backdrop filter, clipped by their
/// ancestors' clips exactly as in the page.
fn split_backdrop(rec: Scene) -> (Scene, Scene, Vec<[f32; 21]>) {
    let rec = flatten(rec);
    let mut page = Scene::new();
    let mut mask = Scene::new();
    let mut regions: Vec<[f32; 21]> = Vec::new();
    for cmd in rec.commands {
        match cmd {
            RenderCommand::PushLayer(mut layer) => {
                if let Some(bf) = layer.backdrop_filter.take() {
                    let c = layer.transform.as_coeffs();
                    let scale = ((c[0] * c[3] - c[1] * c[2]).abs()).sqrt() as f32;
                    let mut p = identity_backdrop();
                    let mut m = [1.0f32, 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0.];
                    for node in bf.nodes() {
                        match &node.effect {
                            FilterEffect::GaussianBlur(b) => p[0] += b.std_deviation * scale,
                            FilterEffect::ColorMatrix(cm) => m = mat_mul(&cm.0, &m),
                            _ => {}
                        }
                    }
                    p[1..].copy_from_slice(&m);
                    regions.push(p);
                    let id = regions.len().min(255) as u8;
                    mask.push_clip_layer(layer.fill, layer.transform, &layer.clip);
                    mask.fill(Fill::NonZero, layer.transform, Color::from_rgba8(id, 0, 0, 255), None, &layer.clip);
                } else {
                    mask.push_clip_layer(layer.fill, layer.transform, &layer.clip);
                }
                page.commands.push(RenderCommand::PushLayer(layer));
            }
            RenderCommand::PushClipLayer(clip) => {
                mask.push_clip_layer(clip.fill, clip.transform, &clip.clip);
                page.commands.push(RenderCommand::PushClipLayer(clip));
            }
            RenderCommand::PopLayer => {
                mask.pop_layer();
                page.commands.push(RenderCommand::PopLayer);
            }
            other => page.commands.push(other),
        }
    }
    (page, mask, regions)
}


/// a·b for 4×5 colour matrices (b applied first).
fn mat_mul(a: &[f32; 20], b: &[f32; 20]) -> [f32; 20] {
    let mut r = [0.0; 20];
    for i in 0..4 {
        for j in 0..5 {
            let mut v = if j == 4 { a[i * 5 + 4] } else { 0.0 };
            for k in 0..4 { v += a[i * 5 + k] * b[k * 5 + j]; }
            r[i * 5 + j] = v;
        }
    }
    r
}

/// A key went down (`down` 1) or up (0): `key` and `code` as the DOM names them ("a", "Enter", "ArrowLeft"; "KeyA",
/// "Enter"), `mods` 1 Shift, 2 Control, 4 Alt, 8 Meta, `repeat` 1 when held. The page's scripts hear keydown/keyup on
/// the focused element (or the body); unless one prevents it, the default happens (typing into a focused text field,
/// Tab moving the focus). Returns 1 when the page took the key (a focused text field, or a script prevented it), so
/// the game leaves it alone.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_key(ui: *mut Vitrail, down: i32, key: *const c_char, code: *const c_char, mods: u32, repeat: i32) -> i32 {
    guard("vitrail_key", 0, || {
    use blitz_traits::events::{BlitzKeyEvent, KeyState, UiEvent};
    use keyboard_types::{Code, Key, Location, Modifiers};
    use std::str::FromStr;
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (key, code) = unsafe { (text(key), text(code)) };
    if key.is_empty() { return 0; }
    // A field a script just made gets its editor when the page is resolved: bring the page up to date first.
    if ui.dirty { ui.doc.as_mut().resolve(ui.time); }
    let kind = if down != 0 { "keydown" } else { "keyup" };
    // The open select list takes every key.
    if ui.popup.is_some() { if down != 0 { forms::key(ui, &key, mods & 4 != 0); } return 1; }
    // Keys go to the focused element; with none (or the root, where Blitz keeps it), to the body, as in a browser.
    let root = ui.doc.root_element().id;
    let target = ui.doc.get_focussed_node_id().filter(|&f| f != root).or_else(|| select(&ui.doc, "body").first().copied());
    let mut prevented = false;
    if let Some(t) = target.filter(|_| ui.js.is_some() && ui.js_state.listening.contains(kind)) {
        let extra = format!("{{\"key\":{},\"code\":{},\"shiftKey\":{},\"ctrlKey\":{},\"altKey\":{},\"metaKey\":{},\"repeat\":{}}}",
            json_str(&key), json_str(&code), mods & 1 != 0, mods & 2 != 0, mods & 4 != 0, mods & 8 != 0, repeat != 0);
        prevented = !with_js(ui, |js, p| js.call_bool(p, "__dispatch", (kind, (t.as_u64() + 1) as f64, 0.0, 0.0, -1, extra))).unwrap_or(true);
    }
    let editing = ui.doc.get_focussed_node_id().filter(|&f| f != root).and_then(|f| ui.doc.get_node(f)).and_then(|n| n.element_data()).map(|e| e.text_input_data().is_some()).unwrap_or(false);
    // The default (typing, Tab) only for a focused element: a HUD nobody clicked into leaves keys to the game.
    let focused = ui.doc.get_focussed_node_id().filter(|&f| f != root).is_some();
    if !prevented && focused && down != 0 && forms::key(ui, &key, mods & 4 != 0) { return 1; }
    if !prevented && focused {
        let mut m = Modifiers::empty();
        if mods & 1 != 0 { m |= Modifiers::SHIFT; } if mods & 2 != 0 { m |= Modifiers::CONTROL; } if mods & 4 != 0 { m |= Modifiers::ALT; } if mods & 8 != 0 { m |= Modifiers::META; }
        let k = Key::from_str(&key).unwrap_or_else(|_| Key::Character(key.clone().into()));
        let printable = matches!(k, Key::Character(_)) && mods & (2 | 8) == 0;
        let ev = BlitzKeyEvent { key: k, code: Code::from_str(&code).unwrap_or(Code::Unidentified), modifiers: m, location: Location::Standard,
            is_auto_repeating: repeat != 0, is_composing: false, state: if down != 0 { KeyState::Pressed } else { KeyState::Released },
            text: if down != 0 && printable { Some(key.as_str().into()) } else { None } };
        // On macOS Blitz edits on the system's standard key bindings (sent by a window), not on Backspace itself.
        let mac_binding = cfg!(target_os = "macos") && down != 0 && key == "Backspace";
        drive(ui, if mac_binding { UiEvent::AppleStandardKeybinding(if mods & 4 != 0 { "deleteWordBackward:" } else { "deleteBackward:" }.into()) }
            else if down != 0 { UiEvent::KeyDown(ev) } else { UiEvent::KeyUp(ev) });
    }
    (prevented || editing) as i32
    })
}

/// For tests: the page's draw commands as the next paint would record them (shared pieces written out), after
/// resolving. Painting consumes the damage gathered since the last paint, as a real paint does.
#[doc(hidden)]
pub fn record_page(ui: *mut Vitrail) -> Vec<RenderCommand> {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return Vec::new() };
    forms::sync(ui);
    let (w, h, s) = (ui.width, ui.height, ui.scale as f64);
    ui.doc.as_mut().resolve(ui.time);
    ui.choose_composited();
    let mut rec = Scene::new();
    paint_scene(&mut rec, ui.doc.as_mut(), s, w, h, 0, 0);
    ui.dirty = false;
    flatten(rec).commands
}

/// What the last `vitrail_gpu_build` cost (4 doubles): ms for styles and layout, the draw list, the GPU frame, and how
/// many draw commands the page had. For measuring.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_profile(ui: *mut Vitrail, out: *mut f64) {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return };
    if !out.is_null() { unsafe { std::ptr::copy_nonoverlapping(ui.profile.as_ptr(), out, 4) }; }
}

/// Text the page copied (Ctrl/Cmd+C or X in a text field, or a selection): written NUL-terminated into `buf` for the
/// game to put on the system clipboard. Returns 1 when there was some, 0 none, minus the bytes needed when too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_clipboard_take(ui: *mut Vitrail, buf: *mut c_char, cap: usize) -> i32 {
    guard("vitrail_clipboard_take", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let mut c = ui.shell.copied.lock().unwrap();
    let Some(t) = c.as_ref() else { return 0 };
    if t.len() >= cap || buf.is_null() { return -((t.len() + 1) as i32); }
    unsafe { write_c(buf, cap, t) };
    *c = None;
    1
    })
}

/// 1 once when the page asked to paste (Ctrl/Cmd+V in a text field): the game answers with `vitrail_paste`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_paste_wanted(ui: *mut Vitrail) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    ui.shell.paste.swap(false, std::sync::atomic::Ordering::Relaxed) as i32
}

/// Pastes `text` into the focused text field (the system clipboard's text, after `vitrail_paste_wanted`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_paste(ui: *mut Vitrail, value: *const c_char) {
    guard("vitrail_paste", (), || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    let t = unsafe { text(value) };
    if ui.doc.get_focussed_node_id().is_some() { drive(ui, blitz_traits::events::UiEvent::Paste(t)); }
    })
}

/// The IME: 1 while a text field wants it (then `out` gets its caret: x, y, width, height in device px), else 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_ime_state(ui: *mut Vitrail, out: *mut f32) -> i32 {
    let Some(ui) = (unsafe { ui.as_ref() }) else { return 0 };
    let (on, r) = *ui.shell.ime.lock().unwrap();
    if !out.is_null() { let s = ui.scale; unsafe { std::ptr::copy_nonoverlapping([r[0] * s, r[1] * s, r[2] * s, r[3] * s].as_ptr(), out, 4) }; }
    on as i32
}

/// An IME event for the focused text field: `kind` 0 composition text so far (`text`, the caret at `caret` bytes or
/// -1 for none), 1 composition committed (`text`), 2 composition cleared (empty preedit).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_ime(ui: *mut Vitrail, kind: i32, value: *const c_char, caret: i32) {
    guard("vitrail_ime", (), || {
    use blitz_traits::events::{BlitzImeEvent, UiEvent};
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    if ui.doc.get_focussed_node_id().is_none() { return; }
    let t = unsafe { text(value) };
    let ev = match kind {
        1 => BlitzImeEvent::Commit(t),
        2 => BlitzImeEvent::Preedit(String::new(), None),
        _ => BlitzImeEvent::Preedit(t, if caret >= 0 { Some((caret as usize, caret as usize)) } else { None }),
    };
    drive(ui, UiEvent::Ime(ev));
    })
}

/// 1 while a text field of the page has the focus (the game should leave typing keys to it).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_editing(ui: *mut Vitrail) -> i32 {
    guard("vitrail_editing", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    if ui.dirty { ui.doc.as_mut().resolve(ui.time); }
    let root = ui.doc.root_element().id;
    ui.doc.get_focussed_node_id().filter(|&f| f != root).and_then(|f| ui.doc.get_node(f)).and_then(|n| n.element_data()).map(|e| e.text_input_data().is_some()).unwrap_or(false) as i32
    })
}

/// Takes the focus away from the page (blur/focusout reach its scripts).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_blur(ui: *mut Vitrail) {
    guard("vitrail_blur", (), || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return };
    set_focus(ui, None);
    })
}

/// Moves the focus one step in a direction (0 up, 1 right, 2 down, 3 left) between the focusable elements, as a
/// gamepad's d-pad or stick does; with a select's list open, up and down move its highlight. The focus ring shows
/// (`:focus-visible`) and the element is scrolled into view. Returns 1 when something moved.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_navigate(ui: *mut Vitrail, dir: i32) -> i32 {
    guard("vitrail_navigate", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    nav::navigate(ui, dir.clamp(0, 3) as u32) as i32
    })
}

/// Presses the focused element as the mouse would (a gamepad's A): a click with its `data-action` and id written as
/// `vitrail_pointer_up` writes them, a select opening, a checkbox ticking; with a select's list open, chooses the
/// highlighted row. Returns 1 for a click with an action or id, 2 when it did something else, 0 with nothing focused.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_activate(ui: *mut Vitrail, action: *mut c_char, id: *mut c_char, cap: usize) -> i32 {
    guard("vitrail_activate", 0, || {
    let Some(ui_ref) = (unsafe { ui.as_mut() }) else { return 0 };
    if ui_ref.popup.is_some() { forms::pick(ui_ref); return 2; }
    let Some(f) = nav::focused(ui_ref) else { return 0 };
    ui_ref.doc.as_mut().resolve(ui_ref.time);
    let Some(r) = ui_ref.doc.get_client_bounding_rect(f) else { return 0 };
    let s = ui_ref.scale;
    let (x, y) = (((r.x + r.width / 2.0) as f32) * s, ((r.y + r.height / 2.0) as f32) * s);
    let clicked = unsafe { vitrail_pointer_down(ui, x, y); vitrail_pointer_up(ui, x, y, action, id, cap) };
    let ui_ref = unsafe { &mut *ui };
    if let Some(now) = nav::focused(ui_ref) { ui_ref.doc.set_focus_ring(now, true); }
    if clicked == 1 { 1 } else { 2 }
    })
}

/// Backs out (a gamepad's B): closes an open select list, else takes the focus away. Returns 1 when it did either.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_cancel(ui: *mut Vitrail) -> i32 {
    guard("vitrail_cancel", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    if ui.popup.is_some() { forms::close(ui); return 1; }
    if nav::focused(ui).is_some() { set_focus(ui, None); return 1; }
    0
    })
}

/// The value of the first element matching `selector`: a select's chosen option's value, a text field's text, a
/// checkbox's or radio's "on" or "" by whether it is checked, else its `value` attribute. Written NUL-terminated into
/// `buf`; returns 1, 0 when nothing matched, or minus the bytes needed when `buf` is too small.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_value(ui: *mut Vitrail, selector: *const c_char, buf: *mut c_char, cap: usize) -> i32 {
    guard("vitrail_value", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let sel = unsafe { text(selector) };
    forms::sync(ui);
    let Some(&n) = select(&ui.doc, &sel).first() else { return 0 };
    let Some(node) = ui.doc.get_node(n) else { return 0 };
    let Some(el) = node.element_data() else { return 0 };
    let v = match el.name.local.as_ref() {
        "select" => forms::chosen(&ui.doc, n).map(|o| forms::value_of(&ui.doc, o)).unwrap_or_default(),
        _ if el.text_input_data().is_some() => el.text_input_data().map(|t| t.editor.raw_text().to_string()).unwrap_or_default(),
        _ if el.checkbox_input_checked().is_some() => if el.checkbox_input_checked() == Some(true) { "on".into() } else { String::new() },
        _ => el.attr(LocalName::from("value")).unwrap_or("").to_string(),
    };
    if v.len() >= cap || buf.is_null() { return -((v.len() + 1) as i32); }
    unsafe { write_c(buf, cap, &v) };
    1
    })
}

/// Chooses the option with `value` in the select matching `selector` (no input/change events: the game set it).
/// Returns 1 when such an option was found.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_select_value(ui: *mut Vitrail, selector: *const c_char, value: *const c_char) -> i32 {
    guard("vitrail_select_value", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (sel, value) = unsafe { (text(selector), text(value)) };
    forms::sync(ui);
    let Some(&n) = select(&ui.doc, &sel).first() else { return 0 };
    let Some(o) = forms::options(&ui.doc, n).into_iter().find(|&o| forms::value_of(&ui.doc, o) == value) else { return 0 };
    forms::choose(ui, n, o, false);
    1
    })
}

/// The oldest `engine.trigger(name, ...args)` of the page's scripts: writes the name and the arguments (a JSON array)
/// NUL-terminated into the two buffers and removes it. Returns 1 when there was one, 0 when none, or minus the bytes
/// needed (the larger of the two, with the NUL) when a buffer is too small (the event stays).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_js_poll(ui: *mut Vitrail, name: *mut c_char, name_cap: usize, json: *mut c_char, json_cap: usize) -> i32 {
    guard("vitrail_js_poll", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some((n, j)) = ui.js_state.events.front() else { return 0 };
    if n.len() >= name_cap || j.len() >= json_cap || name.is_null() || json.is_null() { return -((n.len().max(j.len()) + 1) as i32); }
    unsafe { write_c(name, name_cap, n); write_c(json, json_cap, j); }
    ui.js_state.events.pop_front();
    1
    })
}

/// The oldest message of the page's console (and uncaught errors): writes it NUL-terminated into `buf`, its level
/// (0 log, 1 warning, 2 error) into `level`, and removes it. Returns as `vitrail_js_poll`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_js_log(ui: *mut Vitrail, buf: *mut c_char, cap: usize, level: *mut i32) -> i32 {
    guard("vitrail_js_log", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let Some((l, m)) = ui.js_state.log.front() else { return 0 };
    if m.len() >= cap || buf.is_null() { return -((m.len() + 1) as i32); }
    unsafe { write_c(buf, cap, m); if !level.is_null() { *level = *l as i32; } }
    ui.js_state.log.pop_front();
    1
    })
}

/// Calls the page's `engine.on(name, ...)` handlers with the arguments in `json` (a JSON array, or one value).
/// Returns 1 when the page has scripts.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_js_call(ui: *mut Vitrail, name: *const c_char, json: *const c_char) -> i32 {
    guard("vitrail_js_call", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let (n, j) = unsafe { (text(name), text(json)) };
    // A model for data binding may come to a page without scripts: it gets an engine then.
    if ui.js.is_none() && n == "__model" {
        let Some(js) = js::Js::new() else { return 0 };
        ui.js = Some(js);
        ui.js_state.wake = f64::INFINITY;
    }
    with_js(ui, |js, p| js.call(p, "__engineCall", (n, if j.is_empty() { "[]".to_string() } else { j }))).is_some() as i32
    })
}

/// Runs JavaScript in the page (starting its runtime if it had no scripts); errors go to the page's log.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_js_eval(ui: *mut Vitrail, source: *const c_char) -> i32 {
    guard("vitrail_js_eval", 0, || {
    let Some(ui) = (unsafe { ui.as_mut() }) else { return 0 };
    let src = unsafe { text(source) };
    if ui.js.is_none() {
        let Some(js) = js::Js::new() else { return 0 };
        ui.js = Some(js);
        ui.js_state.wake = f64::INFINITY;
    }
    with_js(ui, |js, p| js.run(p, "eval", &src)).is_some() as i32
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vitrail_destroy(ui: *mut Vitrail) {
    guard("vitrail_destroy", (), || {
    if !ui.is_null() {
        let mut b = unsafe { Box::from_raw(ui) };
        finish_job(&mut b);
        drop(b);
    }
    })
}
