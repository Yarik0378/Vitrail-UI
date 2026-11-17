# Vitrail UI for Unity

Game interfaces written in HTML, CSS and JavaScript, drawn by the game's own GPU, without a browser. An alternative to
game UI middleware such as Coherent Gameface:

- **CSS from Firefox.** Styles, selectors, the cascade, variables, `calc()`, flexbox and grid come from Stylo (Firefox's
  CSS engine) and Taffy, through Blitz. Pages match Chrome within about 1/255 on average (see [CSS support](https://github.com/Yarik0378/Vitrail-UI/blob/main/docs/css-support.md)).
- **The GPU paints.** Pages become triangles drawn by a URP pass: gradients, shadows, rounded clips, `backdrop-filter`
  glass over the game, CSS `filter`, `mix-blend-mode`, SVG, emoji, game textures in `<img src="texture:name">`.
- **Fast.** Only what changed is painted again, element by element; styles and layout on the main thread take a
  fraction of a millisecond and the frame is built on a worker. A hover on a 4K window of 100 cards costs about
  0.1 ms of the main thread; scrolling moves content on the GPU. Numbers: [performance](https://github.com/Yarik0378/Vitrail-UI/blob/main/docs/performance.md).
- **JavaScript.** QuickJS with a DOM: events, timers, `requestAnimationFrame`, modules, `fetch` of local files,
  `MutationObserver`, Web Animations, drag and drop. Preact and petite-vue run unchanged. `engine.on` /
  `engine.trigger` talk to C#, and `data-bind-*` attributes show C# models without any script.
- **Input.** Mouse, wheel (smooth), overlay scrollbars, keyboard, text fields with selection, clipboard and IME,
  `<select>` with a list you style in CSS, and gamepad navigation between focusable elements.

## Install

Package Manager > + > Add package from git URL: `https://github.com/Yarik0378/Vitrail-UI.git?path=/package` (or from disk, `package.json` of this folder). Needs Unity 6 with URP
and the Input System. Plugins for macOS (universal) and Windows (x86_64) are included; Linux comes from CI builds.

## Use

1. Put a page under `Assets/StreamingAssets`, e.g. `ui/menu.html` with its CSS and scripts next to it.
2. GameObject > UI > Vitrail UI Screen, pick the page. Play.
3. React to clicks on elements with a `data-action` in the component's On Click, or from code:

```csharp
var ui = GetComponent<VitrailScreen>().Panel;
ui.Clicked += (action, id) => Debug.Log(action);
ui.SetText("#gold", "120");                 // by CSS selector
ui.SetClass(".turn", "urgent", true);
ui.SetModel("player", JsonUtility.ToJson(player)); // data-bind-value="{{player.gold}}"
ui.Call("showToast", "[\"Saved\"]");        // engine.on('showToast', …) in the page
ui.Triggered += (name, json) => { };        // engine.trigger(name, …) from the page
var res = ui.Value("#resolution");          // a select's or a field's value
```

For a page on a surface in the world, GameObject > UI > Vitrail UI World Panel (a quad; clicks reach it by a raycast).
Gamepad: tick "gamepad" on one panel (the d-pad or stick moves the focus, A presses, B backs out).

Window > Vitrail UI > Inspector shows a live page's elements, their boxes and computed styles (editable), highlights the
chosen one over the game, and picks elements with the mouse. In the editor, saving a page's file reloads it at once.

## Samples

Package Manager > Vitrail UI > Samples: a main menu (gamepad, select, text field), a HUD bound to a C# model, a scrolling
inventory with drag and drop, and a panel in the world. Copy each sample's `StreamingAssets` folder into `Assets`.

## Limits

URP only for now (Built-in and HDRP later). `<select multiple>`, 3D transforms (drawn flat), network URLs (`file:`,
`data:` and `texture:` load) and `::-webkit-scrollbar` styles aren't supported yet. The full list: [CSS support](https://github.com/Yarik0378/Vitrail-UI/blob/main/docs/css-support.md).

## Licence

MIT with the Commons Clause (`LICENSE.md`): free to use in any free or paid game or application, to change and to
share; selling Vitrail UI itself is not allowed. Third-party parts and their licenses: `Third Party Notices.md`.
