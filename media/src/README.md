# Sources of the README pictures

`showcase.html` and `hero.html` are painted by Vitrail UI itself, over a real Unity scene: the pages are transparent
and the game picture (`scene-showcase.jpg`, `scene-hero.jpg`) is drawn under them by the camera, so `backdrop-filter`
blurs the scene the way it does in a game.

`shot/` holds the tool: put `Shot.cs` and `Editor/BuildShot.cs` in an empty Unity 6 project with URP, the Input
System and this package, put this folder under `Assets/StreamingAssets/show/` with a `jobs.txt`
(`page background scale out.png` per line, e.g. `show/showcase.html show/scene-showcase.jpg 1.5 showcase.png`), run
`BuildShot.Build` and start the built player with `-out <folder>`.

`chrome-vs-vitrail.jpg`: the same page in headless Chrome at device scale 1.5 over the same picture (as the body's
background), its left half joined to Vitrail UI's right half. They differ by about 1/255 on average.

- `scene-*.jpg`: a game scene behind the panels.
- `icons/`: painted game icons.
- `AlegreyaSans-*.ttf`: Alegreya Sans by Huerta Tipográfica, SIL Open Font License 1.1.
