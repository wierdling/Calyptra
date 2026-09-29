# Calyptra — User Manual

Calyptra is a Windows desktop program for exploring and rendering fractals.
It does two kinds:

- **3D fractals**: distance-estimated shapes like the Mandelbulb and
  Mandelbox, which you fly around like a landscape and light like a
  photograph.
- **Flames**: fractal flames in the style of Apophysis and JWildfire, built
  from weighted transforms, in 2D or with depth.

Both kinds share the same color tools, animation timeline, image export and
movie rendering.

---

## Contents

1. [Getting started](#1-getting-started)
2. [The window](#2-the-window)
3. [3D fractals](#3-3d-fractals)
4. [Flames](#4-flames)
5. [Color and palettes](#5-color-and-palettes)
6. [Random, Mutate, Batch and History](#6-random-mutate-batch-and-history)
7. [Scene files](#7-scene-files)
8. [Exporting images](#8-exporting-images)
9. [Animation and movies](#9-animation-and-movies)
10. [Custom formulas](#10-custom-formulas)
11. [Command-line rendering](#11-command-line-rendering)
12. [Tips and troubleshooting](#12-tips-and-troubleshooting)

---

## 1. Getting started

### Requirements

- Windows 10 or 11 with a GPU that supports Direct3D 12 or Vulkan.
  Integrated graphics (e.g. Intel Iris Xe) is fine.
- To render movies: **ffmpeg**, installed with `winget install ffmpeg`.
  Not needed for still images or PNG frame sequences.

### Building and running

From the source folder:

```bash
cargo run --release -p calyptra
```

Always use `--release` for real work; debug builds are much slower.

### First launch

The program opens on a Mandelbulb. Try:

1. **Left-drag** in the view to orbit around it, and scroll to zoom.
2. Click **🎲 Random** in the *Fractal* section for a new random fractal.
3. Pick a palette from **Load palette…** in the *Color* section.
4. **File → Export image…** to save a high-resolution picture.

The program remembers your current scene and animation when you close it.
**File → Reset everything** returns to the first-launch defaults.

---

## 2. The window

| Area | What it holds |
|---|---|
| **Menu bar** (top) | File menu: open/save scenes, reset, export image, render movie. Status messages from file operations appear to the right. |
| **Side panel** (left) | The *3D fractal / Flame* switch, history arrows (◀ ▶), **▦ Batch**, and collapsible sections of settings. |
| **Viewport** (center) | The live render. Navigate with the mouse and keyboard. |
| **Timeline** (bottom) | Playback, keyframes and the **🎬 Render movie…** button. |

The side panel shows different sections depending on the mode:

- **3D fractal:** Camera, Fractal, Custom formulas, Color, Lighting, Render,
  Quality, Display, Performance.
- **Flame:** Flame, Color, Display, Performance.

### How the view renders

While you move the camera, the view renders at reduced resolution so it
stays responsive. About 150 ms after you stop, it switches to full
resolution and keeps refining (anti-aliasing, path tracing, or flame
samples) until it reaches its target. You don't need to wait for it to
finish before doing something else.

---

## 3. 3D fractals

Select **3D fractal** at the top of the side panel.

### 3.1 Camera and navigation

The **Camera** section has two navigation modes.

**Orbit** (default): the camera circles a pivot point.

| Input | Action |
|---|---|
| Left-drag | Orbit around the pivot |
| Right-drag (or middle-drag) | Pan |
| Mouse wheel | Zoom in / out |

**Fly**: free flight, for going deep into detail.

| Input | Action |
|---|---|
| Drag | Look around |
| W / S | Forward / back |
| A / D | Left / right |
| R / F | Up / down |
| Q / E | Roll |
| Shift | Move 4× faster |
| Mouse wheel | Change fly speed |

Fly speed scales with your distance to the nearest surface, so you slow down
automatically as you get close. That's what lets you zoom into detail at any
depth. The keys only work while the mouse is over the view.

Switching back to Orbit pivots around whatever is in the center of the view.

Other camera controls:

- **Field of view**: 10°–120°.
- **Position** and **Surface distance**: read-only; the distance to the
  surface under the crosshair.
- **Aperture**: depth of field, used by the path tracer. 0 keeps everything
  sharp.
- **focus** and **Focus at center**: the focus distance; the button sets it
  to whatever is under the center of the view.
- **Reset camera**.

### 3.2 Fractal

**Load preset…** loads a ready-made fractal with a suitable camera:
Mandelbulb, Juliabulb, Mandelbox, Menger sponge, KIFS octahedron, Amazing
Surf, Hybrid: Mandelbulb + Mandelbox, Quaternion Julia, Pseudo-Kleinian.

#### Formulas and hybrids

A fractal is a list of up to 8 **formula slots**, applied in order on each
iteration. One slot is a plain fractal; two or more make a *hybrid*.

Each slot shows:

- its number and a **formula** dropdown (built-in and custom formulas);
- **×N**: how many times in a row this formula runs before moving on;
- **⏶ ⏷** to reorder, **🗑** to remove;
- sliders for the formula's parameters. You can type values outside a
  slider's range.

**+ Add formula** appends a new slot (a copy of the last formula).

Built-in formulas:

| Formula | Notes |
|---|---|
| Mandelbulb | Power formula; the classic 3D Mandelbrot. |
| Mandelbox | Box and sphere folds. |
| Menger sponge | Cube-based IFS. |
| KIFS octahedron | Kaleidoscopic IFS with rotation. |
| Amazing Surf | Folding formula with surf-like forms. |
| Quaternion Julia | 4D formula; a parameter picks the 3D slice. |
| Pseudo-Kleinian | Kleinian-group-like structures (custom distance estimate). |

#### Other fractal settings

- **Iterations**: more gives finer detail and slower rendering.
- **Bailout**: escape radius.
- **Distance estimate**: how distance to the surface is computed. **Auto**
  picks the right one for the formulas; override it (Logarithmic, Linear,
  Box, Custom) if a hybrid renders with holes or blobs.
- **Julia mode**: uses a fixed constant **c** (three values) instead of the
  sample position. This gives Juliabulbs and similar shapes.

Composition or shader errors appear in red at the top of this section.

### 3.3 Lighting

- **Key light**: Azimuth, Elevation, color, Intensity; **Soft shadows** and
  **Shadow sharpness**.
- **Surface**: Ambient, Ambient occlusion, Specular, Shininess.
- **Atmosphere**: Background (top / bottom gradient colors), Fog, Glow color
  and strength, Glow radius.
- **Reset lighting**.

### 3.4 Render: Preview vs. Path trace

- **Preview**: fast direct lighting. When idle, it gathers 16 samples for
  anti-aliasing. Best for exploring.
- **Path trace**: physically based global illumination with soft sun
  shadows, sky light, glossy reflections and depth of field. It converges
  over time; the progress bar shows samples done out of the target.
  - **Bounces**: light bounces (1–8).
  - **Target samples**: how far the view keeps refining (16–4096).
  - **Sun size**: larger means softer shadows.
  - **Denoise** / **Denoise strength**: cleans up noise so you need fewer
    samples. It keeps fractal color detail sharp.

While you navigate, the view always uses the fast preview; path tracing
resumes once the camera settles. Changes to exposure, tone mapping and
denoising apply without restarting the accumulated image.

### 3.5 Quality

- **Max steps**: raymarching steps per pixel. Raise it if distant parts
  disappear.
- **Detail (px)**: surface precision in pixels. Smaller means finer detail
  and slower rendering.
- **Step factor**: below 1 makes steps more cautious. Lower it if you see
  banding, holes or "overstepping" artifacts, which is common with hybrids.
- **Max distance**: how far rays travel.
- **Resolution scale**: render resolution relative to the window. Below 1
  is faster; above 1 supersamples.
- **Resolution while moving**: the scale used during navigation.

### 3.6 Display

- **Exposure (EV)**: brightness, −5 to +5.
- **Tone map**: ACES, AgX (gentler highlights) or Clamp.
- **Dither**: prevents banding in smooth gradients.

### 3.7 Performance

Shows your GPU, the render size, and GPU time per frame. Use it to judge
the effect of quality settings.

---

## 4. Flames

Select **Flame** at the top of the side panel.

### 4.1 Creating a flame

In the **Flame** section:

- **Presets…**: Swirl galaxy, Golden swirl, Electric web, Nebula, 3D plume,
  Fire feather, Julian bloom, Sierpinski triangle.
- **🎲 Random**: a new random 2D flame.
- **🎲 Random 3D**: a random flame with depth (tilted transforms, 3D
  variations, perspective).
- **Mutate**: nudges the current flame's transforms slightly.
- **Import .flame…** / **Export .flame…**: see [4.5](#45-flame-files).

### 4.2 Navigating

| Input | Action |
|---|---|
| Left-drag | Pan |
| Right-drag | Orbit (tilt and turn a 3D flame) |
| Mouse wheel | Zoom |

The view keeps adding samples after you stop moving, so it gets smoother
over a few seconds.

### 4.3 Transforms

A flame is a set of up to 12 **transforms**. Each is listed by its
variations (e.g. `1. spherical + swirl`). Click one to edit it.

- **+ Add**, **Duplicate**, **🗑** (delete).
- **Final**: adds a final transform, applied to every plotted point. Useful
  for warping the whole image.

For the selected transform:

- **Weight**: how often it is chosen. Heavier transforms dominate.
- **Color**: its position on the palette (0–1).
- **Color speed**: how fast points take on this transform's color.
- **Affine**: a 3×4 grid. Rows are x′, y′, z′; columns multiply x, y, z,
  plus an offset. The z row and column give a flame depth; leave them at
  identity for a flat flame. Quick buttons: **⟲ 15°** / **⟳ 15°** rotate,
  **×1.1** / **×0.9** scale, **Reset**.
- **Post transform**: an optional second affine applied after the
  variations.
- **Variations**: up to 4 per transform. Each has a type, a weight (**w**),
  and parameters for types that take them. **+ Variation** adds one.

Available variations: linear, sinusoidal, spherical, swirl, horseshoe,
polar, handkerchief, heart, disc, spiral, hyperbolic, diamond, ex, julia,
bent, fisheye, exponential, power, cosine, eyefish, bubble, cylinder,
tangent, cross, blur, julian, curl, pdj; 3D: linear3D, spherical3D,
sinusoidal3D, blur3D, julia3D, hemisphere; depth helpers: separation,
zcone, ztranslate, zscale, pre_blur.

### 4.4 Flame render settings

- **Quality (points/pixel)**: higher is smoother and slower. Low values are
  fine for exploring; raise it for final exports.
- **Supersample**: 1–3× internal resolution. Sharper, uses more memory.
- **Brightness**, **Gamma**, **Vibrancy**: tone controls. Vibrancy keeps
  colors saturated in bright areas.
- **Smoothing**: density estimation. It blurs sparse, grainy areas while
  dense detail stays sharp. 0 turns it off. When on, **Smoothing falloff**
  and **Minimum smoothing** tune it.
- **Background** color.
- **zoom**, rotation (°) and **Reset view**.
- **3D view**: Yaw, Pitch, Perspective, Depth of field, Focus depth, Depth
  fade. These only matter for flames with depth.

Tone changes (brightness, gamma, vibrancy, background, smoothing, quality)
are applied to the existing image without starting over.

### 4.5 .flame files

**Import .flame…** reads `.flame`, `.flam3` and `.xml` files from Apophysis
(including the 3D hack), JWildfire and flam3. If a file holds several
flames, a window lists them; pick one. The palette is imported with the
flame.

If something in the file isn't supported (an unknown variation, more than 4
variations per transform, more than 12 transforms), the flame is still
imported. The approximations are listed under the buttons, and marked ⚠ in
the picker.

The file's `quality` setting is ignored, since it's usually a preview
value. Set quality in the program or in the export.

**Export .flame…** writes flam3 XML that other programs can open. Files
exported from this program reopen exactly, including the 3D settings.

---

## 5. Color and palettes

The **Color** section appears in both modes.

### Palettes

**Load palette…** lists 15 built-in palettes with previews: Bronze,
Electric, Fire, Gold, Grayscale, Ice, Inferno, Jade, Magma, Ocean, Pastel,
Spectrum, Sunset, Turbo, Viridis.

### Gradient editor

The bar under the menu is the current gradient. Triangles below it are
color stops.

- **Double-click the bar** to add a stop.
- **Click or drag a triangle** to select or move a stop.
- For the selected stop: color picker, position (**at**), hex code, and
  **🗑** to delete (a gradient keeps at least two stops).
- **Reverse**, **Even spacing**.
- **Import…**: Fractint `.map`, GIMP `.ggr` or Ultra Fractal `.ugr`
  palettes.

Colors blend perceptually (OKLab), so gradients stay smooth without muddy
midpoints.

### Coloring 3D fractals

These controls appear only for 3D fractals. Flames get their colors from
each transform's **Color** value.

- **Color source**: what value picks the gradient color:
  - *Orbit trap (point)* and *Orbit trap (planes)*: how close the iteration
    came to a point or planes. Gives the classic banded, cellular looks.
  - *Iterations*: smoothed iteration count.
  - *Height*: vertical position.
  - *Surface normal*: facing direction.
- **Fit to view**: stretches the gradient over the range of values visible
  now. It runs automatically when you change the source, load a preset, or
  generate a random fractal. Press it again after moving somewhere very
  different.
- **Offset**: shifts colors along the gradient.
- **Frequency**: how many times the gradient repeats.
- **Wrap**: Repeat, Mirror or Clamp. Palettes that don't loop switch to
  Mirror automatically to avoid a visible seam.

---

## 6. Random, Mutate, Batch and History

### Random

- **3D:** **🎲 Random** (in *Fractal*) builds a single or hybrid fractal,
  tests candidates until one is neither empty nor a shapeless blob, frames
  it, and picks a palette and color source. Your lighting and render
  settings are kept. The formulas used are shown under the button; if no
  candidate passed, the best one is shown with a note. **Cancel** stops the
  search.
- **Flame:** **🎲 Random** and **🎲 Random 3D** (in *Flame*).

### Mutate

**Mutate** makes small changes to the current fractal or flame. Press it
repeatedly to explore variations of something you like.

### Batch

**▦ Batch** (top of the side panel) opens a window that generates 20 random
fractals or flames (depending on the current mode) as thumbnails, using
your current settings.

- **Double-click** a thumbnail, or click **Edit**, to load it into the main
  view. It's outlined in the grid.
- **Save…** saves that one as a scene file.
- **🎲 Generate again** makes a new batch; **Stop** stops the current one.

### History (◀ ▶)

The arrows next to the mode switch step back and forward through results of
Random, Mutate, presets, batch picks and flame imports (the last 50). Edits
you made to a result are kept: go back, then forward, and your tweaked
version returns. Generating something new after going back discards the
forward entries.

---

## 7. Scene files

A **scene** is everything about the current picture: mode, fractal or
flame, camera, colors, lighting and render settings. It also includes the
animation, if you have keyframes.

- **File → Save scene as…** writes a readable `.json` file.
- **File → Open scene…** opens a `.json` scene, **or a PNG exported by this
  program**: every exported PNG has its scene embedded, so any image you've
  rendered can be reopened and continued.
- **File → Reset everything** restores the defaults.

---

## 8. Exporting images

**File → Export image…** opens the export window.

- **Size**: width × height in pixels (up to 16384), with **Presets** (HD,
  Full HD, QHD, 4K, 8K, Square, Portrait) and **Match viewport aspect**.
- **Anti-aliasing**: samples per pixel, from Off to 100. For path-traced
  scenes this is also the number of path samples; 16–64 with denoising is a
  good range. For flames, quality comes mainly from the flame's
  **Quality** setting.
- **Format**:
  - *PNG (8-bit)*: dithered, smallest files.
  - *PNG (16-bit)*: default; best for further editing.
  - *OpenEXR (linear HDR)*: raw linear light for compositing and grading.
- **Estimated time** is based on how fast the viewport currently renders.

Click **Render and save…**, choose a file, and the render runs in the
background with a progress bar and time remaining. You can keep working
meanwhile. **Cancel** stops it.

Exports use the same settings as the viewport (render mode, denoise,
exposure, tone map), rendered at full size.

---

## 9. Animation and movies

### Keyframes

Every setting can be animated: camera, formula parameters, colors,
lighting, flame transforms. A keyframe is a snapshot of the whole scene.

The **timeline** at the bottom:

- **⏮ ▶/⏸ ⏭**: go to start, play/pause, go to end.
- Current time and frame number.
- **length** (seconds) and **fps**.
- **Loop** and **Show path** (draws the camera path over the view: a line,
  with dots at keyframe positions).
- **+ Add key**: stores the current scene as a keyframe at the playhead.
- With a key selected:
  - **Update key**: replaces the key with the current scene. It shows **●**
    when you've changed the scene since the key was made.
  - **Easing**: timing from this key to the next (Smooth, Ease in, Ease
    out, Ease in/out).
  - **🗑**: delete the key.
- The track below: click or drag on empty space to scrub; click a ◆ to
  select a key and jump to it; drag a ◆ to scrub, Shift+drag a ◆ to move it in time.

A typical workflow:

1. Set up the first view and click **+ Add key**.
2. Move the playhead later, change the view or parameters, and **+ Add
   key** again.
3. Press ▶ to preview. Playback uses the fast preview renderer.
4. To fix a key, click it, adjust, and press **Update key**.

The camera moves along smooth curves through the keyframes. Numeric values
interpolate smoothly; on/off settings and formula choices switch when the
next key is reached.

### Rendering a movie

Click **🎬 Render movie…** (timeline) or **File → Render movie…**.

- The top line shows keyframes, length, fps and total frame count.
- **Size** with presets (720p to 4K, square, vertical).
- **Samples**: per pixel per frame (1–256).
- **Format**:
  - *H.264 MP4*: plays everywhere.
  - *H.265 MP4*: smaller files, 10-bit color.
  - *ProRes 422 HQ MOV*: for video editing.
  - *PNG frames only*: no ffmpeg needed.
- **ffmpeg**: the command or full path. The detected version is shown
  below; if it isn't found, install it with `winget install ffmpeg` or
  enter the full path to `ffmpeg.exe`.

Click **Render…** and choose the output file. Frames are rendered as 16-bit
PNGs into a folder next to it (`<name>_frames`), then encoded.

**Renders resume.** If you cancel or close the program, render again to the
same file: finished frames are reused and only the missing ones render.

---

## 10. Custom formulas

You can write your own 3D formulas in WGSL, the shader language used by
the program. They live in `%APPDATA%\Calyptra\formulas`.

In the **Custom formulas** section:

- **Open folder** opens that folder in Explorer.
- Type a name and click **New formula**. This creates a documented starter
  file and opens it in your editor. Then pick it in a formula slot.
- Your formulas are listed with **Edit** buttons.

Saving a file updates the view immediately. If a formula has an error, the
section title shows **⚠** with a count, and the error is listed with its
file, line and column. Only the broken formula is disabled; fix it and save.

The file format is described in [custom-formulas.md](custom-formulas.md).

---

## 11. Command-line rendering

Stills and movies can also be rendered without the window, which is useful
for batch jobs.

**Still image:**

```bash
cargo run --release -p export --example export_still -- out.png 3840 2160 16 --scene my_scene.json
```

- Arguments: output file (`.png` or `.exr`), then width, height and samples
  (default 1920 × 1080, 16).
- `--scene` accepts a scene `.json`, an exported `.png`, or a `.flame`
  file.
- `--png8` for 8-bit PNG.
- Flames: `--quality N` (points per pixel) and `--smoothing R`.

**Movie:**

```bash
cargo run --release -p export --example export_movie -- out.mp4 1920 1080 16 --project my_scene.json
```

The scene file must contain keyframes (save it with **Save scene as…**
after adding them). Without `--project` it renders a short demo.

---

## 12. Tips and troubleshooting

**The view is slow or choppy.** Lower **Resolution scale** or **Resolution
while moving** (Quality), reduce **Iterations** or **Max steps**, or switch
from Path trace to Preview while exploring.

**Holes, speckles or missing pieces in a 3D fractal.** Lower **Step
factor**, raise **Max steps**, or try a different **Distance estimate**
mode. Hybrids especially benefit from a step factor of 0.5–0.7.

**Surface looks blurry up close.** Lower **Detail (px)** and raise
**Iterations**.

**Colors are all one shade.** Press **Fit to view**, or try a different
**Color source**.

**The path-traced image is noisy.** Raise **Target samples**, enable
**Denoise**, or make the sun larger (softer shadows converge faster).

**A flame looks grainy.** Wait for it to refine, raise **Quality**, or
increase **Smoothing**.

**A flame is too dark or washed out.** Adjust **Brightness** and **Gamma**;
lower **Vibrancy** if bright areas look oversaturated.

**An imported flame looks different from Apophysis/JWildfire.** Check the
warnings under the import buttons; unsupported variations are approximated.

**"ffmpeg not found."** Run `winget install ffmpeg`, restart the program,
or enter the full path to `ffmpeg.exe` in the movie window. You can always
choose *PNG frames only* and encode separately.

**Lost a good random result.** Use ◀ to step back through History.

**Something is in a weird state.** **File → Reset everything.**
