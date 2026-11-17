# Getting started in Unity

## Install

Add the package `package/` of this repository: Package Manager > + > Add package from disk (its `package.json`), or in
`Packages/manifest.json` `"com.vitrail.ui": "file:<path to>/package"`, or the
`com.vitrail.ui-<version>.tgz` that CI builds. Namespace `Vitrail`.

## Requirements

- Unity 6 with URP, the Input System, linear colour space.
- macOS (Apple silicon or Intel), Windows x64, Linux x64 (the CI-built package). The native library is in the
  package's `Plugins/`.

## The quick way

GameObject > UI > Vitrail UI Screen, pick a page under StreamingAssets, press Play. Its On Click event gets
`data-action` clicks; `GetComponent<VitrailScreen>().Panel` is the WebPanel below for everything else. GameObject > UI >
Vitrail UI World Panel puts a page on a quad in the scene. Window > Vitrail UI > Inspector shows the live page's elements.

## A page from code

Put the page and everything it links (CSS, fonts, images) under `Assets/StreamingAssets`, e.g.
`StreamingAssets/ui/hud.html`. Then:

```csharp
using Vitrail;

public sealed class Hud : MonoBehaviour
{
    private WebPanel _web;

    private void Start()
    {
        // The page covers the top 140 CSS px of the screen; pass 0 for the whole screen (a menu, a window).
        _web = new WebPanel("ui/hud.html", 140);
        _web.Clicked += (action, id) => Debug.Log($"clicked {action} ({id})");
        _web.Loaded += () => { /* the page was (re)loaded: set what depends on its size */ };
    }

    private void Update()
    {
        _web.Update(scale: 1f);          // device px per CSS px; follow your UI scale
        _web.SetText("#gold", "120");    // only a changed value touches the page
        _web.SetClass("#cell-food", "full", food >= foodCap);
    }

    private void OnDestroy() => _web.Dispose();
}
```

`Update` keeps the page the screen's width, feeds it the mouse (hover, clicks, wheel, cursor), plays CSS animations,
and repaints when something changed. The page is drawn by the GPU painter; if its shaders can't run, the CPU painter
takes over and the console says why.

## Talking to the page

The game can fill a page by CSS selector, or the page's own JavaScript can do the work and talk to the game (see
below). Every setter remembers the last value per selector, so calling it every frame with the same value costs
nothing.

| C# | Does |
|---|---|
| `SetText(selector, text)` | the text of the first element matching |
| `SetAttr(selector, name, value)` | an attribute (`src` of an image, `data-*`) |
| `SetClass(selector, cls, on)` | adds or removes a class: the usual way to switch looks, played by CSS transitions |
| `SetStyle(selector, property, value)` | one CSS property inline (`--p` for a progress ring, `transform`) |
| `SetTexture(name, texture)` | a Unity texture where the page has `<img src="texture:name?w=256&h=256">` |
| `Clicked(action, id)` | an element with `data-action="…"` was clicked (its `id` too) |
| `HoverChanged(action, id)` | the element with `data-action` under the pointer changed |
| `Covers(screenPoint)` | whether the page takes the pointer there (so the game shouldn't) |
| `RectOf(selector)` | where an element is on screen (to anchor a tooltip or a tutorial arrow) |
| `Stats` | what the last change cost: main thread, worker, what is drawn every frame |

```html
<div class="btn" data-action="settings"><img src="icons/gear.svg"><div class="tip">Settings</div></div>
<div class="minimap"><img src="texture:minimap?w=256&h=256"></div>
```

```css
.btn { transition: transform .15s, background .15s; }
.btn:hover { transform: translateY(-2px); background: #2a4a43; }
```

## JavaScript

`<script>` elements run in document order (inline, or `src` next to the page) on QuickJS, then the modules
(`type="module"`, with an import map for bare names like `preact`), then `DOMContentLoaded` and `load` fire. A page without scripts gets no engine. The DOM covers what UI libraries use: querying, creating and moving
nodes, attributes, `classList`, `style`, `dataset`, `textContent`, `innerHTML` (set), `getBoundingClientRect`, events
with capture and bubbling (pointer, mouse, click, enter/leave, keydown/keyup, focus/blur, input, animation and
transition events; listened types only), focus (`el.focus()`, `document.activeElement`), text fields you can type into,
`setTimeout`, `setInterval`, `requestAnimationFrame` (on the game's clock, so they run when the game is paused too),
promises, `console`, `fetch()` of files next to the page, `MutationObserver`, `innerHTML`/`outerHTML`, `el.animate()`.

The keyboard: `WebPanel` sends each frame's keys to the page (characters as the system types them, so any layout
works). While a text field of the page has the focus, `EditingText` is true (the game should ignore its hotkeys then);
Escape leaves the field. The mouse places the caret and selects, Ctrl/Cmd+C, X and V use the system clipboard, and the
system IME (Chinese, Japanese, Korean) composes in the field.

The game and the page talk as in Coherent:

```js
engine.on('gold', (amount) => { document.querySelector('#gold').textContent = amount; });
document.querySelector('#end').addEventListener('click', () => engine.trigger('endTurn'));
```

```csharp
panel.Call("gold", "[120]");                       // calls engine.on('gold') handlers; arguments as a JSON array
panel.Triggered += (name, json) => { if (name == "endTurn") EndTurn(); };
panel.Eval("console.log(document.title)");        // any code; console and errors go to the Unity console
```

## Data binding

Without any script, a page can show the game's models through `data-bind-*` attributes (as Coherent's):

```html
<div data-bind-value="Turn {{game.turn}}"></div>
<div class="cell" data-bind-value="{{player.gold}}" data-bind-class-toggle="low:{{player.gold < 10}}"></div>
<div class="bar"><div data-bind-style-width="{{player.hp * 100}}%"></div></div>
<li data-bind-for="i, it:{{player.items}}" data-bind-click="engine.trigger('pick', i)">{{…}}</li>
```

`data-bind-value`, `-html`, `-if`, `-class`, `-class-toggle`, `-style-<property>`, `-attr-<name>`, `-for` and events
(`-click`, `-mouseenter`, …). From C#: `panel.Call("__model", "[\"player\", {…}]")` sends a whole model and syncs; only
values that changed are written (a sync of a small HUD ≈0.01 ms). A binding to a model that hasn't come yet waits
quietly.

## Game textures

`SetTexture("minimap", renderTexture)` shows any texture where the page names it, sampled directly on the GPU (no copy
through the CPU), with everything CSS gives an image: `border-radius`, `object-fit`, filters, masks. While one is on
screen, the page is redrawn every frame so a live render texture stays live.

## Editing live

In the editor, saving any `.html`, `.css`, `.svg` or `.png` under the page's folder reloads the page in the running
game; a rebuilt native library is picked up without restarting Unity.

## Performance tips

- Animate `transform` and `opacity`: they run on the compositor (no repaint at all). Animating `width`, `color` or
  `top` repaints the page each frame (still under a millisecond for a HUD, but not free).
- Avoid `opacity` or a rounded `overflow: hidden` around an animated element: it then can't be its own layer.
- Glass (`backdrop-filter`) is drawn every frame, but only the glass and its group.
- `Stats` tells what each change cost.
