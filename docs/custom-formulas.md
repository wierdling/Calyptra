# Custom formulas

Custom formulas are WGSL files in `%APPDATA%\Calyptra\formulas`. The app
watches the folder: save a file and the view updates. The quickest start is
**Custom formulas → New formula** in the side panel, which writes a documented
starter file and opens it.

## Anatomy

```wgsl
// @name Folded box
// @de linear
// @bailout 100
// @param scale 2 -3 3
// @param fold 1 0 2
fn folded_box(z: ptr<function, vec3<f32>>, dr: ptr<function, f32>, c: vec3<f32>, P: folded_box_Params) {
    let v = clamp(*z, vec3<f32>(-P.fold), vec3<f32>(P.fold)) * 2.0 - *z;
    *z = v * P.scale + c;
    *dr = (*dr) * abs(P.scale) + 1.0;
}
```

- **The file name is the formula id** (`folded_box.wgsl` → `folded_box`):
  letters, digits and `_`, not starting with a digit. The function must have
  the same name.
- The function is **one iteration**. It updates:
  - `z`: the point being iterated;
  - `dr`: the running derivative, used to estimate the distance to the surface;
- and reads:
  - `c`: the constant to add, which is the sample position, or the Julia
    constant in Julia mode;
  - `P`: the parameters, one `f32` field per `@param`.
- **Helper functions** must be prefixed with the id (`folded_box_rotate`) so
  formulas can be combined in hybrids without name clashes.

## Header keys

Each key goes on its own line, starting with `// @`.

| Key | Meaning |
|---|---|
| `@name Text` | Display name. |
| `@de logarithmic \| linear \| box \| custom` | How the distance is estimated from the final `z` and `dr`. Use `logarithmic` for power formulas (Mandelbulb), `linear` for folding formulas (Mandelbox, IFS), `box` for cube-based IFS (Menger). |
| `@bailout N` | Suggested escape radius. |
| `@param name default min max` | A slider. Up to 8 per formula. |
| `@state vec4` | Iterate a 4D point: `z: ptr<function, vec4<f32>>`. |
| `@init_w param` | For 4D formulas: the starting 4th coordinate, which picks the 3D slice. |

### Custom distance estimates

With `@de custom`, also define:

```wgsl
fn my_formula_de(z: vec4<f32>, dr: f32, P: my_formula_Params) -> f32 { ... }
```

It receives the final point (as `vec4`; `.xyz` for 3D formulas), the
derivative and the parameters. See `pseudo_kleinian.wgsl` for an example.

## Errors

Problems are listed under **Custom formulas** with the file, line and
column, for example `folded_box.wgsl:7:19: no definition in scope for
identifier`. A broken file disables only that formula. Fix it and save.

## Examples

The built-in formulas are good templates. They are in `formulas/wgsl/` in the
source tree:

- `mandelbulb.wgsl`: power formula, logarithmic distance estimate
- `mandelbox.wgsl`: box and sphere folds
- `kifs_octahedron.wgsl`: helper function, rotation
- `quaternion_julia.wgsl`: 4D state, slice selection
- `pseudo_kleinian.wgsl`: custom distance estimate
