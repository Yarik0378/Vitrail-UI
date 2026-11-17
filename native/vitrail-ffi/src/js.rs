//! JavaScript in the page: QuickJS (through rquickjs) runs the page's `<script>`s. The DOM is written in JavaScript
//! (`dom.js`) over a few native operations on node ids (`__n`), so the Rust side stays small: query, read and change
//! nodes, attributes, styles and text, report to the game. Events, timers and the game's bridge (`engine.on`,
//! `engine.trigger`) live in `dom.js` too.
//!
//! Every call into JavaScript goes through `Js::enter`, which points the native operations at the page (`UI`) for the
//! duration and runs the promise jobs left afterwards.

use crate::Vitrail;
use blitz_dom::{LocalName, NodeId, QualName, ns};
use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Context, Ctx, Function, Module, Object, Runtime, Value};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

thread_local! {
    /// The page the native operations work on while JavaScript runs (null otherwise).
    static UI: Cell<*mut Vitrail> = const { Cell::new(std::ptr::null_mut()) };
}

/// What the page's JavaScript left for the game and the library between calls.
#[derive(Default)]
pub struct JsState {
    /// `engine.trigger(name, ...args)`: (name, JSON array of the arguments), oldest first, for `vitrail_js_poll`.
    pub events: std::collections::VecDeque<(String, String)>,
    /// `console.*` and uncaught errors: (level 0 log, 1 warning, 2 error, message), for `vitrail_js_log`.
    pub log: std::collections::VecDeque<(u8, String)>,
    /// The page clock's time (ms) the next timer or animation frame is due; infinite when none.
    pub wake: f64,
    /// Event types some listener waits for (pointer moves and hover changes are only sent for these).
    pub listening: std::collections::HashSet<String>,
    /// Each CSS animation's and transition's state when last looked at, for animation events:
    /// (node, is a transition, name or property) → (state, iteration).
    pub anims: std::collections::HashMap<(u64, bool, String), (u8, f64)>,
}

pub struct Js {
    rt: Runtime,
    ctx: Context,
    /// The page's import map (`<script type="importmap">`): a bare or prefixed specifier → a path.
    imports: Rc<RefCell<Vec<(String, String)>>>,
}

/// A path with `.` and `..` worked out (module names are absolute paths, so each module is loaded once).
fn normalize(path: &std::path::Path) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir => out.clear(),
            c => out.push(c.as_os_str().to_string_lossy().into_owned()),
        }
    }
    format!("/{}", out.join("/"))
}

/// Module specifiers as a browser resolves them for a page from files: relative to the importing module, absolute
/// paths and file: URLs, and bare names through the page's import map.
struct PageResolver(Rc<RefCell<Vec<(String, String)>>>);

impl Resolver for PageResolver {
    fn resolve<'js>(&mut self, ctx: &Ctx<'js>, base: &str, name: &str, _: Option<ImportAttributes<'js>>) -> rquickjs::Result<String> {
        let dir = std::path::Path::new(base.split('#').next().unwrap_or(base)).parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let target = |spec: &str| -> Option<String> {
            if let Some(p) = spec.strip_prefix("file://") { Some(normalize(std::path::Path::new(p))) }
            else if spec.starts_with('/') { Some(normalize(std::path::Path::new(spec))) }
            else if spec.starts_with("./") || spec.starts_with("../") { Some(normalize(&dir.join(spec))) }
            else { None }
        };
        let map = self.0.borrow();
        // The import map: an exact entry, or the longest prefix ending with '/'.
        let mapped = map.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone()).or_else(|| {
            map.iter().filter(|(k, _)| k.ends_with('/') && name.starts_with(k.as_str())).max_by_key(|(k, _)| k.len()).map(|(k, v)| format!("{v}{}", &name[k.len()..]))
        });
        match mapped.as_deref().and_then(target).or_else(|| target(name)) {
            Some(path) => Ok(path),
            None => Err(rquickjs::Exception::throw_type(ctx, &format!("cannot resolve module '{name}' (not relative, and not in the page's import map)"))),
        }
    }
}

/// Modules are files next to the page; `with { type: 'json' }` makes a JSON file the default export.
struct PageLoader;

impl Loader for PageLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str, attrs: Option<ImportAttributes<'js>>) -> rquickjs::Result<Module<'js, rquickjs::module::Declared>> {
        let path = name.split(['?', '#']).next().unwrap_or(name);
        let Ok(source) = std::fs::read_to_string(path) else {
            return Err(rquickjs::Exception::throw_reference(ctx, &format!("cannot read module {name}")));
        };
        let json = attrs.and_then(|a| a.get_type().ok().flatten()).as_deref() == Some("json") || path.ends_with(".json");
        let source = if json { format!("export default {source};") } else { source };
        Module::declare(ctx.clone(), name, source)
    }
}

/// The page, from a native operation (only while JavaScript runs).
fn ui<R>(f: impl FnOnce(&mut Vitrail) -> R) -> Option<R> {
    let p = UI.with(|u| u.get());
    if p.is_null() { None } else { Some(f(unsafe { &mut *p })) }
}

/// Node ids in JavaScript are Blitz's plus one, so 0 means "no node" (the document node is Blitz's 0).
fn id(n: f64) -> NodeId { NodeId::from_u64((n as u64).saturating_sub(1)) }
fn num(n: NodeId) -> f64 { (n.as_u64() + 1) as f64 }
fn qual(name: &str) -> QualName { QualName::new(None, ns!(), LocalName::from(name)) }

/// A message for the log from a JavaScript exception (its message and stack).
fn describe(ctx: &Ctx, v: Value) -> String {
    let _ = ctx;
    if let Some(e) = v.as_exception() {
        format!("{}{}", e.message().unwrap_or_default(), e.stack().map(|s| format!("\n{s}")).unwrap_or_default())
    } else if let Some(s) = v.as_string() {
        s.to_string().unwrap_or_default()
    } else {
        format!("{v:?}")
    }
}

impl Js {
    pub fn new() -> Option<Js> {
        let rt = Runtime::new().ok()?;
        rt.set_memory_limit(256 << 20);
        rt.set_max_stack_size(4 << 20);
        let ctx = Context::full(&rt).ok()?;
        let imports = Rc::new(RefCell::new(Vec::new()));
        rt.set_loader(PageResolver(imports.clone()), PageLoader);
        let js = Js { rt, ctx, imports };
        js.ctx.with(|ctx| -> rquickjs::Result<()> {
            let n = Object::new(ctx.clone())?;
            natives(&ctx, &n)?;
            ctx.globals().set("__n", n)?;
            ctx.eval::<(), _>(include_str!("dom.js"))?;
            ctx.eval::<(), _>(include_str!("bind.js"))?;
            Ok(())
        }).ok()?;
        Some(js)
    }

    /// Runs `f` with the page reachable from the native operations, then the promise jobs it queued.
    pub fn enter<R>(&self, ui: *mut Vitrail, f: impl FnOnce(&Ctx) -> R) -> R {
        let prev = UI.with(|u| u.replace(ui));
        let r = self.ctx.with(|ctx| f(&ctx));
        // Promise reactions (and whatever they queue) run now, as a browser's microtasks after a task.
        for _ in 0..10_000 {
            match self.rt.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    let msg = e.0.with(|ctx| { let v = ctx.catch(); describe(&ctx, v) });
                    unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, format!("Uncaught (in promise) {msg}"))); } }
                }
            }
        }
        UI.with(|u| u.set(prev));
        r
    }

    /// Evaluates a classic script; an exception goes to the log with `name`.
    pub fn run(&self, ui: *mut Vitrail, name: &str, source: &str) {
        let name = name.to_string();
        let source = source.to_string();
        self.enter(ui, |ctx| {
            if ctx.eval::<(), _>(source).is_err() {
                let v = ctx.catch();
                let msg = describe(ctx, v);
                unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, format!("{name}: {msg}"))); } }
            }
        });
    }

    /// The page's import map: `{"imports": {"preact": "./vendor/preact.mjs", "lib/": "./lib/"}}`, its paths made
    /// absolute against the page's directory.
    pub fn set_import_map(&self, json: &str, page_dir: &std::path::Path) {
        let mut map = self.imports.borrow_mut();
        let Some(body) = json.find("\"imports\"").map(|i| &json[i..]) else { return };
        // A small reader for the flat {"spec": "path"} object (no nesting in import maps' `imports`).
        let Some(open) = body.find('{') else { return };
        let Some(close) = body[open..].find('}') else { return };
        let inner = &body[open + 1..open + close];
        let strings: Vec<&str> = inner.split('"').skip(1).step_by(2).collect();
        for pair in strings.chunks(2) {
            if let [k, v] = pair {
                let path = if v.starts_with("./") || v.starts_with("../") { normalize(&page_dir.join(v)) } else { v.to_string() };
                map.push((k.to_string(), path));
            }
        }
    }

    /// Evaluates a module (named by a path, so its relative imports resolve from there); a failure at any point,
    /// loading an import or a rejected top-level await, goes to the log.
    pub fn run_module(&self, ui: *mut Vitrail, name: &str, source: &str) {
        let (name, source) = (name.to_string(), source.to_string());
        let promise = self.enter(ui, |ctx| match Module::evaluate(ctx.clone(), name.clone(), source) {
            Ok(p) => Some(rquickjs::Persistent::save(ctx, p)),
            Err(_) => {
                let v = ctx.catch();
                let msg = describe(ctx, v);
                unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, format!("{name}: {msg}"))); } }
                None
            }
        });
        // The module ran (its jobs too, in `enter`): a rejection is reported now.
        if let Some(p) = promise {
            self.enter(ui, |ctx| {
                let Ok(p) = p.restore(ctx) else { return };
                if let Some(Err(_)) = p.result::<Value>() {
                    let v = ctx.catch();
                    let msg = describe(ctx, v);
                    unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, format!("{name}: {msg}"))); } }
                }
            });
        }
    }

    /// Like `call`, for a function returning a boolean (`__dispatch`: false when a listener called preventDefault).
    pub fn call_bool(&self, ui: *mut Vitrail, func: &str, args: impl for<'js> rquickjs::function::IntoArgs<'js>) -> bool {
        let func = func.to_string();
        self.enter(ui, |ctx| {
            let f: rquickjs::Result<Function> = ctx.globals().get(func.as_str());
            let Ok(f) = f else { return true };
            match f.call::<_, bool>(args) {
                Ok(b) => b,
                Err(_) => {
                    let v = ctx.catch();
                    let msg = describe(ctx, v);
                    unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, msg)); } }
                    true
                }
            }
        })
    }

    /// Calls a global function of `dom.js` by name with plain arguments; an exception goes to the log.
    pub fn call(&self, ui: *mut Vitrail, func: &str, args: impl for<'js> rquickjs::function::IntoArgs<'js>) {
        let func = func.to_string();
        self.enter(ui, |ctx| {
            let f: rquickjs::Result<Function> = ctx.globals().get(func.as_str());
            let Ok(f) = f else { return };
            if f.call::<_, ()>(args).is_err() {
                let v = ctx.catch();
                let msg = describe(ctx, v);
                unsafe { if let Some(u) = ui.as_mut() { u.js_state.log.push_back((2, msg)); } }
            }
        });
    }
}

/// The native operations `dom.js` is built on. Node ids are numbers; 0 is never a node.
fn natives<'js>(ctx: &Ctx<'js>, n: &Object<'js>) -> rquickjs::Result<()> {
    macro_rules! f { ($name:expr, $body:expr) => { n.set($name, Function::new(ctx.clone(), $body)?)?; }; }

    // The document node, and its root element.
    f!("doc", || -> f64 { ui(|u| num(u.doc.root_node().id)).unwrap_or(0.0) });
    // Elements matching a selector inside `scope` (the document node: all), in tree order; one with `first`.
    f!("q", |scope: f64, sel: String, first: bool| -> Vec<f64> {
        ui(|u| {
            let doc = &u.doc;
            let root = doc.root_node().id;
            if id(scope) == root {
                if let Some(i) = sel.strip_prefix('#').filter(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')) {
                    return doc.get_element_by_id(i).map(num).into_iter().collect();
                }
            }
            let found = if id(scope) == root { doc.query_selector_all(&sel) } else { doc.query_selector_all_in(id(scope), &sel) };
            let mut v: Vec<f64> = found.map(|ids| ids.into_iter().map(num).collect()).unwrap_or_default();
            if first { v.truncate(1); }
            v
        }).unwrap_or_default()
    });
    f!("matches", |node: f64, sel: String| -> bool { ui(|u| u.doc.matches_selector(id(node), &sel).unwrap_or(false)).unwrap_or(false) });
    // 1 element, 3 text, 8 comment, 9 document, 0 gone.
    f!("type", |node: f64| -> i32 {
        ui(|u| u.doc.get_node(id(node)).map(|nd| if nd.is_element() { 1 } else if nd.is_text_node() { 3 } else if nd.id == u.doc.root_node().id { 9 } else { 8 }).unwrap_or(0)).unwrap_or(0)
    });
    f!("tag", |node: f64| -> String {
        ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.element_data()).map(|e| e.name.local.to_string()).unwrap_or_default()).unwrap_or_default()
    });
    f!("parent", |node: f64| -> f64 { ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.parent).map(num).unwrap_or(0.0)).unwrap_or(0.0) });
    f!("children", |node: f64| -> Vec<f64> { ui(|u| u.doc.get_node(id(node)).map(|nd| nd.children.iter().copied().map(num).collect()).unwrap_or_default()).unwrap_or_default() });
    // textContent (all the text inside), and a text node's own data.
    f!("text", |node: f64| -> String { ui(|u| u.doc.get_node(id(node)).map(|nd| nd.text_content()).unwrap_or_default()).unwrap_or_default() });
    f!("data", |node: f64| -> String { ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.text_data()).map(|t| t.content.to_string()).unwrap_or_default()).unwrap_or_default() });
    f!("setText", |node: f64, value: String| {
        ui(|u| {
            let is_text = u.doc.get_node(id(node)).map(|nd| nd.is_text_node()).unwrap_or(false);
            let mut m = u.doc.mutate();
            if is_text { m.set_node_text(id(node), &value); } else {
                m.remove_and_drop_all_children(id(node));
                if !value.is_empty() { let t = m.create_text_node(&value); m.append_children(id(node), &[t]); }
            }
            drop(m);
            u.dirty = true;
        });
    });
    f!("attr", |node: f64, name: String| -> Option<String> { ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.attr(LocalName::from(name.as_str())).map(|s| s.to_string()))).flatten() });
    f!("attrs", |node: f64| -> Vec<String> {
        ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.element_data()).map(|e| e.attrs().iter().flat_map(|a| [a.name.local.to_string(), a.value.to_string()]).collect()).unwrap_or_default()).unwrap_or_default()
    });
    f!("setAttr", |node: f64, name: String, value: String| {
        ui(|u| {
            if u.doc.get_node(id(node)).and_then(|nd| nd.attr(LocalName::from(name.as_str()))) == Some(value.as_str()) { return; }
            u.doc.mutate().set_attribute(id(node), qual(&name), &value);
            u.dirty = true;
            if name == "selected" { u.check_selects = true; }
        });
    });
    f!("rmAttr", |node: f64, name: String| {
        ui(|u| {
            if u.doc.get_node(id(node)).and_then(|nd| nd.attr(LocalName::from(name.as_str()))).is_none() { return; }
            u.doc.mutate().clear_attribute(id(node), qual(&name));
            u.dirty = true;
            if name == "selected" { u.check_selects = true; }
        });
    });
    f!("setStyle", |node: f64, name: String, value: String| {
        ui(|u| {
            let mut m = u.doc.mutate();
            if value.is_empty() { m.remove_style_property(id(node), &name); } else { m.set_style_property(id(node), &name, &value); }
            drop(m);
            u.dirty = true;
        });
    });
    // An element in the HTML namespace (so the default styles apply), or the SVG one with `svg`.
    f!("create", |tag: String, svg: bool| -> f64 {
        let name = if svg { QualName::new(None, ns!(svg), LocalName::from(tag.as_str())) } else { QualName::new(None, ns!(html), LocalName::from(tag.to_ascii_lowercase().as_str())) };
        ui(|u| num(u.doc.mutate().create_element(name, Vec::new()))).unwrap_or(0.0)
    });
    f!("createText", |value: String| -> f64 { ui(|u| num(u.doc.mutate().create_text_node(&value))).unwrap_or(0.0) });
    f!("createComment", |value: String| -> f64 { ui(|u| num(u.doc.mutate().create_comment_node(&value))).unwrap_or(0.0) });
    // Moves `child` (from wherever it is) to before `before` in `parent` (0: at the end).
    f!("insert", |parent: f64, child: f64, before: f64| {
        ui(|u| {
            let mut m = u.doc.mutate();
            if m.node_has_parent(id(child)) { m.remove_node(id(child)); }
            if before == 0.0 { m.append_children(id(parent), &[id(child)]); } else { m.insert_nodes_before(id(before), &[id(child)]); }
            drop(m);
            u.dirty = true;
            u.check_selects = true;
        });
    });
    f!("remove", |node: f64| {
        ui(|u| {
            let mut m = u.doc.mutate();
            if m.node_has_parent(id(node)) { m.remove_node(id(node)); }
            drop(m);
            u.dirty = true;
            u.check_selects = true;
        });
    });
    f!("clone", |node: f64, deep: bool| -> f64 {
        ui(|u| {
            if deep { num(u.doc.mutate().deep_clone_node(id(node))) } else {
                let (name, attrs) = match u.doc.get_node(id(node)).and_then(|nd| nd.element_data()) { Some(e) => (Some(e.name.clone()), e.attrs().to_vec()), None => (None, Vec::new()) };
                let text = u.doc.get_node(id(node)).and_then(|nd| nd.text_data()).map(|t| t.content.to_string());
                let mut m = u.doc.mutate();
                match (name, text) { (Some(n), _) => num(m.create_element(n, attrs)), (None, Some(t)) => num(m.create_text_node(&t)), _ => num(m.create_comment_node("")) }
            }
        }).unwrap_or(0.0)
    });
    f!("setHtml", |node: f64, html: String| { ui(|u| { u.doc.mutate().set_inner_html(id(node), &html); u.dirty = true; u.check_selects = true; }); });
    // getBoundingClientRect in CSS px: [x, y, width, height].
    f!("rect", |node: f64| -> Vec<f64> {
        ui(|u| {
            u.doc.as_mut().resolve(u.time);
            u.doc.get_client_bounding_rect(id(node)).map(|r| vec![r.x, r.y, r.width, r.height]).unwrap_or_else(|| vec![0.0; 4])
        }).unwrap_or_default()
    });
    f!("viewport", || -> Vec<f64> { ui(|u| vec![u.width as f64 / u.scale as f64, u.height as f64 / u.scale as f64, u.scale as f64]).unwrap_or_default() });
    // Focus (document.activeElement, el.focus(), el.blur()) and form values (a text field's text lives in its editor).
    f!("focused", || -> f64 { ui(|u| { let root = u.doc.root_element().id; u.doc.get_focussed_node_id().filter(|&f| f != root).map(num).unwrap_or(0.0) }).unwrap_or(0.0) });
    f!("focus", |node: f64| { ui(|u| { if u.doc.set_focus_to(id(node)) { u.dirty = true; } }); });
    f!("blur", || { ui(|u| { if u.doc.get_focussed_node_id().is_some() { u.doc.clear_focus(); u.dirty = true; } }); });
    f!("checked", |node: f64| -> bool { ui(|u| { crate::forms::sync(u); u.doc.is_checked(id(node)) }).unwrap_or(false) });
    f!("pick", |select: f64, option: f64| { ui(|u| { crate::forms::sync(u); crate::forms::choose(u, id(select), id(option), false); }); });
    f!("value", |node: f64| -> Option<String> {
        if let Some(v) = ui(|u| {
            let t = u.doc.get_node(id(node)).and_then(|nd| nd.element_data()).map(|e| e.name.local.to_string()).unwrap_or_default();
            match t.as_str() {
                "select" => { crate::forms::sync(u); Some(crate::forms::chosen(&u.doc, id(node)).map(|o| crate::forms::value_of(&u.doc, o)).unwrap_or_default()) }
                "option" => Some(crate::forms::value_of(&u.doc, id(node))),
                _ => None,
            }
        }).flatten() { return Some(v); }
        ui(|u| u.doc.get_node(id(node)).and_then(|nd| nd.element_data()).and_then(|e| e.text_input_data().map(|t| t.editor.raw_text().to_string()).or_else(|| e.attr(LocalName::from("value")).map(|v| v.to_string())))).flatten()
    });
    f!("computed", |node: f64, prop: String| -> String { ui(|u| { u.doc.as_mut().resolve(u.time); u.doc.resolved_style_value(id(node), &prop) }).unwrap_or_default() });
    f!("log", |level: i32, msg: String| { ui(|u| u.js_state.log.push_back((level.clamp(0, 2) as u8, msg))); });
    f!("emit", |name: String, json: String| { ui(|u| u.js_state.events.push_back((name, json))); });
    f!("wake", |at: f64| { ui(|u| u.js_state.wake = at); });
    f!("listen", |kind: String| { ui(|u| { u.js_state.listening.insert(kind); }); });
    f!("now", || -> f64 { ui(|u| u.time * 1000.0).unwrap_or(0.0) });
    // A file next to the page (a script's src), read as text; None when it can't be read.
    f!("read", |url: String| -> Option<String> { ui(|u| crate::read_relative(u, &url)).flatten() });
    Ok(())
}
