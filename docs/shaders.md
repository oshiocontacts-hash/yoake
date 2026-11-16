# Horizon shader packs

Horizon has its own shader packs: WGSL written against stages of Horizon's renderer, checked before they run. This page
is the guide for pack authors. The design and the pack format are in [api/shaders.md](api/shaders.md); every stage's
signature, input, resource and limit is in the generated [api/shaders-reference.md](api/shaders-reference.md).

Shader packs in the Iris/OptiFine format are a separate feature with its own loader; this page is only about
Horizon's own format.

| Part | Where | State |
|---|---|---|
| Format, stages, check | `crates/horizon-shader-api` | API 0.1, experimental |
| Authoring tool | `horizon-shader` (same crate) | `new`, `check`, `pack` |
| Templates | `crates/horizon-shader-api/templates/` | post, sky, material |
| Example pack | `crates/horizon-shader-api/packs/golden_hour` | a warm sky and two post passes |
| In the game: every stage | Options > Horizon Settings > Shader Packs | sky, material, post passes, hot reload |
| The registry and the launcher | `shader` kind ([api/mod-index.md](api/mod-index.md)); the Mods page's Shader packs tab | checked, then installed into `shaderpacks/` |

## Quick start

```sh
cargo install --path crates/horizon-shader-api      # or: cargo run -p horizon-shader-api --bin horizon-shader -- <command>
horizon-shader new my_pack --template post          # also: --template sky, --template material; --name "My Pack"
horizon-shader check my_pack                        # what the game, the launcher and the registry will check
horizon-shader pack my_pack                         # check, then write my_pack-0.1.0.hshader beside the folder
```

`check` prints each stage with its cost, and the pack's SHA-256:

```
golden_hour 1.0.0 (shader API 0.1): ok
  sky       Warm horizon                     99 expressions, 0 texture reads
  post      Glow                            192 expressions, 3 texture reads
  post      Colour grade                    201 expressions, 2 texture reads
  sha256 b5e0d98c...
```

or every problem, at its file, line and column:

```
my_pack 0.1.0 (shader API 0.1): FAILED
  post.wgsl:9:1: expected `;`, found "}"
```

`--json` prints the same for tools (`ok`, `id`, `version`, `api`, `sha256`, `stages`, `problems`).

## Trying a pack in the game

Put the pack folder (or its `.hshader`) in `shaderpacks/` beside `options.txt` (Open Pack Folder opens it) and pick
it on Options > Horizon Settings > Shader Packs, which lists Horizon's packs, marked (Horizon), beside packs in the
`shaders/` format; one runs at a time, and clicking it again turns it off. `HORIZON_SHADER_PACK=<its folder or file
name>` runs a pack whatever the page says, for development. The game checks the pack first, and refuses one that
fails, with its problems in the chat. While it runs, the game looks at the pack twice a second: save a file
and the pack is checked and rebuilt in place ("Shader pack ... reloaded"); if the new version fails, the chat says
where and the last good version keeps drawing. All three stages run.

A picked pack in the `shaders/` format gets a Shader Options... button at the top of the page: its options on the
screens and with the names the pack gives them, its profiles, and Reset. A click picks an option's next value, a
right click the one before, shift-click the pack's default, and the options the pack lists in `sliders` are sliders;
values that differ from the pack's are yellow. Hovering one shows what the pack says of it (`option.NAME.comment`),
and the whole label when its name was shortened to fit. Leaving the page saves them to
`shaderpacks/<pack>.txt` (`NAME=value` lines) and loads the pack again with them.

## A pack

A folder (or the same files zipped as a `.hshader`) with `horizon-shaders.toml` and WGSL files:

```toml
schema = 1
id = "my_pack"
name = "My Pack"
version = "0.1.0"
api = "0.1"

[[post]]
file = "post.wgsl"

[[options]]
id = "saturation"
name = "Saturation"
type = "float"
default = 1.15
min = 0.0
max = 2.0
```

Every key is in [api/shaders.md](api/shaders.md#the-pack-format-schema-1).

## Stages

A stage is one function your file defines. Horizon puts its prelude (the structs, `frame`, the stage's resources and
helpers) before your file and calls the function from its own pass, so the world is still culled, batched and meshed
by Horizon: a pack changes how things look, not how they are drawn.

| Stage | Your function | What it does |
| --- | --- | --- |
| `[sky]` | `fn sky(input: SkyInput) -> vec4<f32>` | the sky's colour along each view ray, opaque (alpha is ignored), drawn where vanilla draws its sky dome (the Overworld's sky; not the Nether's or the End's); the sun, moon, stars and clouds are drawn over it |
| `[material]` | `fn material(input: MaterialInput) -> Surface` | the lit colour of each point of a block's surface, before the alpha test and fog: every terrain section and distant tile, solid, cutout and translucent (not entities, block entities or particles) |
| `[weather]` | `fn weather(input: WeatherInput) -> vec4<f32>` | the colour of each point of a rain or snow streak, blended over the world with its alpha; Horizon's light and fog aren't applied, so the pack applies its own |
| `[[post]]` | `fn post(input: PostInput) -> vec4<f32>` | a full-screen pass over the world's picture (up to 8, in order), under the hand and HUD. Return `input.color.a` as your alpha: HDR output reads it to find light sources |

A pack has at least one stage; a stage it leaves out keeps vanilla's look. Each input says what vanilla would draw
(`input.color` in the sky and post stages; `albedo * tint * vec4(lightmap, 1.0)` for a material), so a pack can start
from vanilla and change only what it means to.

Every stage can read `frame`: the camera's matrices and position, the sun and moon directions, the time of day,
vanilla's sky and fog colours, rain and thunder, fog distances, the light at the camera, effects and the dimension.
Post passes also read `scene_color`, `scene_depth`, `previous` (the last pass's output), two samplers and the helpers
`depth_at(uv)` and `view_position(uv, depth)`.

Rules the check enforces: your files declare no bindings (`@group`/`@binding`), no entry points and no names starting
with `horizon_`. Code shared by several stages goes in files listed in `shared`, put before every stage.

## Sprite tags

The material stage can tell block sprites apart through tags the manifest declares. Each `[[tags]]` is a bit of
`input.tags`, in the order they appear (32 at most); its `sprites` are sprite names, the namespace `minecraft` by
default, each with at most one `*`:

```toml
[[tags]]
id = "glass"                                      # bit 0: (input.tags & 1u) != 0u
sprites = ["block/glass", "block/*_stained_glass"]
```

Terrain carries its sprite, not its block, so a sprite drawn by several blocks gets the tag for all of them.

## Material ids

Tags answer "is it one of these"; a pack that treats hundreds of kinds of surface differently numbers them
instead. Each sprite takes the `id` of the first `[[materials]]` entry that names it, 0 without one, as
`input.material`. Patterns may hold any number of `*`:

```toml
[[materials]]
id = 10
sprites = ["block/*_leaves"]

[shadow]
solid_materials = [10]   # these cast whole shadows: their cutouts are ignored in the shadow map
```

Shadow-map size can be a pack setting. `resolution_option` names a `choice` option whose values are decimal shadow-map
side lengths in texels, each between 256 and 8192. The chosen value is additionally capped by the device's maximum 2D
texture dimension. Without `resolution_option`, `resolution` retains its existing behavior.

```toml
[shadow]
resolution = 1024
resolution_option = "shadow_size"
cache = true

[[options]]
id = "shadow_size"
name = "Shadow size"
type = "choice"
values = ["512", "1024", "2048"]
default = "1024"
```

Shadow caching is opt-in (`cache = false` by default). A cached map survives small sun movements (less than 0.1 degrees)
and camera movement within the same 8-block snapped cell; its matrix is rebased to the current camera. Nearby mesh
changes invalidate it. Animated vertex stages, shadow-only geometry and moving entity geometry bypass the cache.

Unlit entity overlays (layers 2, 4, 6 and 9–12, including glint and eyes) do not cast shadows, so their animation does
not invalidate a cached map.

## Optional world inputs

World inputs are opt-in at the top level of `horizon-shaders.toml`:

```toml
light_list = true
taa_jitter = true
entity_material = { file = "entities.wgsl" }

[[post]]
file = "resolve.wgsl"

[voxels]
size = [128, 128, 128]
```

Omitting `[voxels]`, `light_list`, `taa_jitter` and `entity_material` keeps their corresponding rendering behavior
disabled. An empty `[voxels]` table requests the default 128³ window. Dimensions must be positive multiples of 16,
each at most 256 blocks, and total at most 256 × 128 × 256 voxels. If the requested size exceeds the GPU's 3D texture
limit, only axes exceeding that limit are reduced to at most 64 blocks (and the device limit, rounded down to a
multiple of 16). Already supported axes keep their requested size.
`frame.voxel_size` reports the actual dimensions; `frame.has_voxels` is 1 only with an enabled voxel input, otherwise 0.

`@group(3) @binding(14) var voxels: texture_3d<u32>` is a camera-centered toroidal `R32Uint` texture, one block per cell.
Only entering or explicitly dirtied 16³ sections are uploaded, nearest to the camera first, with at most 16 sections
per frame. Remaining sections stay queued and invalid until uploaded. Uniform-air sections are written as zeroes.
`frame.voxel_origin` and `frame.voxel_center` are camera-relative block positions; `frame.voxel_cell_size` is 1.
Material, entity and post-process stages can read the texture through the precision-safe helper:

```wgsl
let word = voxel_at(input.position);
let shape = word & 15u;
```

`voxel_at(position: vec3<f32>) -> u32` takes a camera-relative block position and returns zero outside the window or
in a pending section. It never reconstructs large world coordinates from `frame.camera_position`. The integer
`frame.voxel_origin_slot` gives the physical block slot of the grid minimum. The helper combines that slot with the
small camera-relative offset, retaining block precision far from the origin.

Direct `textureLoad` users must also check `frame.voxel_validity` to avoid reading evicted toroidal slots. Physical
section slots are flattened as `x + width * (y + height * z)`, where `width` and `height` are the texture dimensions
divided by 16. For section index `index`, word `index / 32` is stored at `frame.voxel_validity[word / 4][word % 4]`;
bit `index % 32` must be set. A clear bit means unavailable, not confirmed air. The integer texture has no filtering
sampler. Each voxel is a four-byte little-endian word:

| Bits | Meaning |
| --- | --- |
| 0–3 | Shape: 0 empty, 1 full, 2 bottom slab, 3 top slab, 4–7 bottom stairs facing north, east, south, west, 8 thin/fence, 9 pane, 10–13 top stairs in the same order, 14–15 reserved |
| 4 | Translucent |
| 5–8 | Emission level, clamped to 0–15 |
| 9–14 | Red tint, quantized to 6 bits |
| 15–20 | Green tint, quantized to 6 bits |
| 21–26 | Blue tint, quantized to 6 bits |
| 27–31 | Reserved, zero |

Empty voxels are zero. Each 8-bit tint component is converted with `(component * 63 + 127) / 255`, using integer
division. Tint is a block-color approximation (including stained glass), not an atlas sample or baked lighting.

`@group(3) @binding(15) var<storage, read> light_list: array<WorldLight>` exposes the nearest 2048 block lights whose
radius intersects a 128-block camera distance, plus one combined held-item light. Ties are deterministic. Selection
is rebuilt only when the CPU light revision or camera block cell changes; positions are rebased otherwise.
`frame.light_count` bounds valid entries. A `WorldLight`
occupies 48 bytes: `position: vec3<f32>`, `level: f32`, `color: vec3<f32>`, `radius: f32`, `held: u32` and three padding
words. Positions are camera-relative, levels are 0–15, radii are in blocks and `held` is 1 for the held light (0 for
block lights). Emissive blocks default to white; `[[lights]]` entries override their colors. This input is available
to material, entity and post-process stages.

The [World Inputs example](../crates/horizon-shader-api/packs/world_inputs/) demonstrates voxel lookup and the light list.

## Projection jitter

`taa_jitter = true` offsets the rendered projection with a centered one-based Halton (2, 3) sequence. `frame.jitter.xy`
is the current offset in pixels and `.zw` is the previous offset. `frame.unjittered_view_projection` and
`frame.previous_unjittered_view_projection` expose the corresponding camera matrices without jitter. A resized target
or a history reset also resets the previous jitter and matrix. Jitter defaults to false and does not enable a pack's
temporal accumulation automatically; the pack must implement its own resolve/history passes.
Enabling jitter requires at least one `[[post]]` resolve stage. Geometry and `Frame` use the same world-target size
and the same stored offset, computed once per frame after applying the capture presentation clock.

## Entity material

An optional `entity_material = { file = "entities.wgsl" }` stage defines
`fn entity_material(input: EntityMaterialInput) -> Surface`. `EntityMaterialInput` contains camera-relative `position`, `normal`, unlit
`albedo: vec4<f32>`, vanilla `light: vec2<f32>` and `lightmap: vec3<f32>`, and `kind: u32`. Ordinary entity kinds are
the entity registry's numeric IDs; block entities set bit 31 and put the block-state ID in the remaining bits. This
stage shades mobs, players and block-entity geometry in the entity pass, including translucent item-sheet geometry.
Without it, the vanilla entity pipeline remains selected. The adapter preserves vanilla fog, cutouts, overlays and
glint around the returned surface color.

## Textures

Each `[[textures]]` entry is a PNG in the pack that every stage reads under the name of its `id`, with a sampler named
`<id>_sampler`. Eight at most, without mipmaps; `filter` is `linear` (the default) or `nearest`, `wrap` is `repeat` (the
default) or `clamp`:

```toml
[[textures]]
id = "noise"
file = "noise.png"
```

```wgsl
let n = textureSample(noise, noise_sampler, input.uv);
```

## Coloured block light

Each `[[lights]]` entry gives the blocks it names a colour. The material stage reads the colour at a point with
`block_light_color(input.position)`: rgb the colours of the sources nearby, averaged by how near each is, and a how
strongly they reach it (0 where none does). Block light keeps its own brightness, so walls still stop it: this is only its
colour, and only what a pack does with it shows.

```toml
[[lights]]
color = [0.3, 0.7, 1.0]
blocks = ["soul_torch", "soul_wall_torch", "soul_lantern"]
```

```wgsl
let tint = block_light_color(input.position);
let block = mix(vec3(1.0), tint.rgb, min(tint.a, 1.0)) * input.light.x;
```

It covers 128 blocks around the camera, moved in steps of 16, and is rebuilt on the CPU when the camera steps or a source
changes, from at most 2048 sources, the nearest first.

## Options

A picked pack with options gets a Shader Options... button at the top of the Shader Packs page. A click picks an
option's next value, a right click the one before, shift-click its default, and Reset all of them; hovering one shows its
`description`. Each change is saved to `shaderpacks/<pack>.txt` (`id=value` lines, values at their defaults left out) and
the pack reloads with it within half a second.

The page shows every option two a row, unless the pack lays them out in `[[pages]]`: it opens on the first page, and an
item `"page:<id>"` is a button opening another, so options can be grouped into categories. `""` leaves a slot empty, and
`columns = 1` gives a page full-width buttons:

```toml
[[pages]]
id = "main"
name = "My Pack"
items = ["page:lighting", "page:water"]

[[pages]]
id = "lighting"
name = "Lighting"
description = "Shadows and the light of blocks."   # the tooltip of the button opening it
items = ["shadow_softness", "block_light"]
```

Every page has to be reached from the first and every option has to be on a page, or the check refuses the pack.

Each `[[options]]` entry is a WGSL `override` of the same name, set when the game builds the pipeline: no text
substitution, no recompiling a different program per value. A choice used by `shadow.resolution_option` need not also
have a WGSL override; if it does, the override has the usual `u32` choice index.

```wgsl
override saturation: f32 = 1.15;   // type = "float"; bool → bool, int → i32, choice → u32 (the value's index)
```

The game shows the options on the pack's settings screen. Branches on an option are on a constant, so the GPU
skips the side that isn't taken.

## What the check does

The game runs the same check when it loads a pack (a pack that fails is refused with the message, and the last good
one keeps drawing), the launcher before it installs one, and the registry's CI before a pack is listed:

1. The manifest and the package: known keys, valid ids, every file used, sizes.
2. Each stage built as the renderer builds it and validated by naga.
3. The stage function's signature, and the rules above.
4. Each `override` matches its option's type (or has a default); each option is used.
5. Budgets: 20 000 expressions and 64 texture reads per stage.
6. Every entry point translated for Vulkan (SPIR-V), Metal and DirectX 12 (HLSL), with the options at their
   defaults, minimums and maximums.

`[budget] gpu_ms` is the GPU time your pack aims to add to a frame. To compare, open F3 in the game: while it is
open, each pass is timed on the GPU and the overlay shows the pack's total and its costliest passes, by their `name`
(the median of the last 15 timed frames). Timing waits for nothing, so it does not slow the game. In a timed frame the sky
stage draws in a pass of its own (it shares the world's pass otherwise), so its time includes clearing the picture. The
material stage runs inside terrain's own draws, so it has no time of its own: compare the frame's GPU time with and
without the pack.

## Next

The overlay and lighting stages, and option values other than the defaults.
