# Fractal Renderer — Project Plan

A Windows desktop application for exploring and rendering **3D fractals** to
high-quality stills and movies, with first-class color/palette tools. Built in
**Rust** on **wgpu** (D3D12/Vulkan) with an **egui** UI.

Target machine: Intel i7-1260P, 16 GB RAM, Intel Iris Xe (integrated). The
design must run well on integrated graphics and never depend on CUDA or fast
FP64.

## Scope

- **Primary:** 3D distance-estimated fractals (Mandelbulb, Mandelbox, IFS, hybrids).
- **Built-in formulas first**, user-written custom formulas later (same mechanism).
- **Later:** flame fractals (M8); 2D escape-time fractals possibly after that.

## Baseline: what existing programs do

| Program | Takeaways |
|---|---|
| Mandelbulber v2 | DE raymarching on GPU (OpenCL); AO, soft shadows, DOF, fog, glow, reflection/refraction; hundreds of formulas + hybrid slots; keyframe/flight animation with visible camera paths |
| Mandelbulb3D | Up to 6 hybridized formulas; animation; hi-res rendering; steep learning curve |
| Fragmentarium / FragM | Fractals as live-edited GLSL; progressive path tracer |
| Ultra Fractal 6 | Layers + blend modes, gradient editor w/ transparency, parameter timeline animation |
| Chaotica / JWildfire | Flame fractals: chaos game, log-density tone mapping, motion blur, animation |
| Fractadyne | Modern Rust + wgpu + egui reference; PNG/EXR export; keyframed tours → ffmpeg |

"Top of the line" color comes from: orbit-trap / iteration-driven coloring fed
through gradients, perceptual (OKLab) gradient interpolation, HDR accumulation
with good tone mapping (AgX/ACES), supersampling and dithering, 16-bit/EXR output.

## Architecture

```
fractals/
├─ app/        egui + wgpu window: viewport, panels, timeline UI, render queue
├─ scene/      scene model (serde), parameters, camera, keyframes & interpolation
├─ formulas/   formula registry + WGSL snippet library + shader composer
├─ render/     Renderer trait; raymarch preview, path tracer, (later) flame
├─ color/      gradients (OKLab), palette library, .ugr/.map/.ggr import
└─ export/     tiled hi-res stills, PNG16/EXR, frame sequences, ffmpeg piping
```

Crates are added as milestones need them.

### Key design decisions

1. **Formulas are composable iteration steps.** Each formula is a WGSL function
   that updates `(z, dr, trap state)` for one iteration. Hybrids are an ordered
   list of steps. The shader composer generates the final shader at runtime.
   Custom user formulas use the exact same path.
2. **Two render modes.**
   - *Interactive preview:* DE sphere tracing with cheap lighting (Phong/PBR,
     DE-based AO, soft shadows, fog, glow). Adaptive resolution while moving,
     progressive refinement when idle.
   - *Final render:* progressive path tracer (DOF, area/environment lights,
     multiple bounces) → tone mapping → optional bloom → optional Intel Open
     Image Denoise (runs on CPU).
3. **Always render offscreen into HDR textures**, then display/export. This
   enables accumulation, tiling and export from one code path.
4. **Short GPU submissions.** Windows resets the GPU if a single submission
   runs ~2 s (TDR). Final renders are split into tiles and sample passes.
5. **Color:** orbit traps, iteration counts, and normals drive gradients
   interpolated in OKLab. Everything stays HDR until the final output transform.
6. **Scenes are human-readable files** (RON/JSON via serde). Every parameter
   is keyframeable. Metadata embedded in exported images for reproducibility.
7. **`Renderer` trait** so the raymarcher, path tracer and flame renderer are
   interchangeable back-ends sharing camera, color, timeline and export.

### Starter formulas

M2 ships Mandelbulb (power n), Mandelbox, Menger sponge, KIFS octahedron,
Amazing Surf, and a Julia mode for every formula (Juliabulb). Quaternion Julia
(4D state) and Pseudo-Kleinian (custom final DE) move to M7, which extends the
formula format.

Formula files live in `formulas/wgsl/`; the header format is documented in
`formulas/src/lib.rs`. `cargo run -p render --example render_presets` renders
every preset headlessly to `renders/presets/`.

## Milestones

| # | Milestone | Deliverable |
|---|---|---|
| **M0** ✅ | Workspace, egui + wgpu window, offscreen full-screen shader, GPU timing | Window with a live shader |
| **M1** ✅ | Mandelbulb raymarcher, orbit + fly camera, basic shading, parameter panel | Fly around a Mandelbulb |
| **M2** ✅ | Formula library, hybrids, shader composer, hot reload | Mix Mandelbox + Mandelbulb |
| **M3** ✅ | Lighting & color: AO, soft shadows, fog, glow, orbit-trap coloring, OKLab gradient editor, palette presets | Genuinely pretty pictures |
| **M4** ✅ | Hi-res stills: tiled rendering, supersampling, PNG16/EXR, scene save/load | Print-quality images |
| **M5** | Path tracer: progressive, DOF, environment light, tone mapping, OIDN denoise | Photoreal renders |
| **M6** | Animation: timeline, keyframes, Catmull-Rom camera splines, easing, preview, resumable render queue, ffmpeg | Movies |
| **M7** | Custom formulas: user WGSL snippets, hot reload, in-UI error reporting | Your own formulas |
| **M8** | Flame fractals: chaos game compute shader, variations, log-density + density estimation | Flames |

## Coloring (M3) notes

- Gradients (`color` crate) are authored in sRGB and interpolated in OKLab;
  baked to a 1024-texel sRGB lookup texture.
- Color sources: point orbit trap, plane orbit trap, smoothed iterations,
  height, surface normal; offset / frequency / wrap (repeat, mirror, clamp).
- **Fit to view:** the GPU probe samples the coloring value on a 16×16 ray
  grid; the 2nd–98th percentile range is mapped onto the gradient.
  Automatic on color-source change, preset load and startup.
- 15 built-in palettes; import of Fractint `.map`, GIMP `.ggr`, Ultra Fractal
  `.ugr` (simplified to the stops that matter).
- `cargo run -p render --example render_presets -- --palettes` renders a
  contact sheet of every palette.

## Stills and scene files (M4) notes

- `render::render_still`: 256×256 tiles, one sample per GPU submission (TDR
  safe), R2 sub-pixel jitter, float32 accumulation. 4K at 16 spp renders in
  ~3 s on the Iris Xe.
- `export` crate: PNG 8-bit (dithered) / 16-bit, OpenEXR (linear, exposure
  applied). PNGs embed the scene as JSON (iTXt `fractals-scene`), so
  File → Open scene accepts exported PNGs.
- Scene files are JSON wrapped with a format name and version; floats
  round-trip exactly.
- CLI: `cargo run --release -p export --example export_still -- out.png 3840 2160 16 [--scene s.json]`.

## Flame fractals (M8) notes

Different paradigm (point splatting into a histogram rather than per-pixel
rays), so new code is: variation library, chaos-game compute shader (atomic
accumulation), log-density tone mapping, density-estimation filter. Reused:
gradients, timeline/keyframes, HDR accumulation, tone mapping, export, UI,
camera (3D flames). Estimated 1–2 milestones if the `Renderer` trait is in
place.

## Environment setup

- Rust stable (pinned via `rust-toolchain.toml`).
- ffmpeg needed from M6: `winget install ffmpeg`.
