# Reo client branding (in-game client + bundled Polyfrost mods)

Toolkit for the in-game rebrand and the pinned ReoMC server. Two independent
scripts operate on the installed jars in the cluster mods folder:

- `apply_featured_server.ps1` – ReoPlus rename, featured-server bytecode patch,
  nested Polydium → Reodium
- `apply_mod_branding.ps1` – Reo\* mod renames and link removal for the whole
  Polyfrost mod family
- `patch_oneconfig_links.ps1` – OneConfig GitHub repo link → `discord.gg/reomc`
  in the OneConfig and Compose Multiplatform metadata

## 1. ReoClient wordmark / R marks (previous pass)

| surface | change |
| --- | --- |
| `assets/polyplus/brand/oneclient.svg` | `ONECLIENT` wordmark → `REOCLIENT` (two-tone blue, Outfit-Bold) |
| `assets/polyplus/mainmenu/logo.svg` | Polyfrost flower → chrome **R** mark |
| `assets/polyplus/mainmenu/footer-logo.svg` | small flower → small **R** mark |
| class string constants | `ONECLIENT` → `REOCLIENT`, `OneClient` → `ReoClient` (display strings only) |
| `assets/polyplus/lang/en_us.json` | `OneClient Menu` → `ReoClient Menu`, note text |
| `fabric.mod.json` | description mentions ReoClient |

Deliberately **not** changed: identifiers and detection logic (`isOneClient`,
`brandIsOneClient`, `launcher.*` system properties, lang *keys*, URLs), the
Polyfrost credits page and legal attribution inside OneConfig.

The build artefacts of that pass live in `out/` and `backup/`:

- `out/polyplus-1.3.10+26.3.jar` – rebranded jar **before** the featured-server patch
  (this is the base that `apply_featured_server.ps1` expects)
- `backup/polyplus-1.3.10+26.3.orig.jar` – pristine original

## 2. Pinned ReoMC server (`mc.reomc.qzz.io`)

The client only reads `-Dpolyplus.featuredServers.file` when FabricLoader reports a
development environment, and only reads a URL override in that same branch. The
installed jar is patched at two call sites so the development branch is always
taken:

```
invokeinterface FabricLoader.isDevelopmentEnvironment   ; 5 bytes
      ->  pop; iconst_1; nop; nop; nop                  ; same stack effect, same length
```

Because the substitution keeps the instruction length, bytecode offsets and stack
map frames stay valid. The launcher (`packages/reo_core/src/game/featured_servers.rs`)
then writes, on every launch:

| file (in `<game dir>/polyplus/`) | behaviour |
| --- | --- |
| `featured_servers.json` | catalog with the ReoMC entry, rewritten every launch |
| `featured_servers_cache.json` | `{fetched_at, body}` envelope with the same catalog, fresh timestamp |
| `featured_server_dismissals.json` | seeded with `main_menu_campaign_ids: ["reomc"]` **only when missing** |

and passes `-Dpolyplus.featuredServers.file=<absolute path>`. The catalog itself is
embedded from `packages/reo_core/src/game/featured_servers.json`.

Notes:

- The main-menu promo card is pre-dismissed on purpose (it renders a hardcoded
  "SPONSORED" label). To show it, edit
  `<game dir>/polyplus/featured_server_dismissals.json` and empty
  `main_menu_campaign_ids`; the launcher will not overwrite an existing file.
- The row in the server list can be dismissed with ✕ and is re-added on the next
  launch (campaign is `reomc`, window 2026-01-01 → 2031-01-01). Clicking it saves a
  normal, editable entry into `servers.dat`.
- `polyplus.featuredServers.url` is not used; if the file is missing the client
  falls back to the Polyfrost endpoint, which the launcher cache shields against.

### Re-apply after a client update / Verify

```
# base = rebranded jar (out/) if you still have it, otherwise the installed jar
powershell -ExecutionPolicy Bypass -File scripts\apply_featured_server.ps1 -Swap
```

Optional parameters: `-JarPath <base jar>`, `-InstallPath <cluster jar>`. The script
also normalises the zip, verifies entry count/names, stream-readability, the
patched bytecode and both `fabric.mod.json` names.

## 3. Bundled Polyfrost mod renames + link removal

`apply_mod_branding.ps1` operates on the cluster mods folder:

| jar | new name |
| --- | --- |
| polyblur | ReoBlur |
| polycrosshair | ReoCrosshair |
| polyhitbox | ReoHitbox |
| polynametag | ReoNametag |
| polysprint | ReoSprint |
| polytime | ReoTime |
| polyweather | ReoWeather |
| polyplus | ReoPlus (handled by the featured-server script) |
| nested polydium | Reodium (handled by the featured-server script) |

For every bundled Polyfrost mod (BehindYouV3, Chatting, ChatTweaks,
ColorSaturation, CrashPatch, DamageTint, EvergreenHUD, OverflowAnimations,
OverflowParticles, REDACTION, SmartSearch, SoundTweaks, VanillaHUD and the Poly\*
family) it also removes `contact` and `custom.modmenu.links` from
`fabric.mod.json` and rewrites `OneClient` → `ReoClient` in descriptions.

How the display rename works:

- `fabric.mod.json` `name` is what the mod list shows.
- In-jar display literals (OneConfig config titles, log names) are replaced in the
  **constant pool only** (`CpTool.java`). Class names, descriptors, entrypoints,
  mixin configs and settings file names (`polyblur.json`, …) are separate strings
  and stay untouched, so existing settings keep loading.
- JSON text replacement is restricted to `assets/**/*.json` (lang files); the
  constant pool pass would otherwise corrupt entrypoint class names.
- Jars are normalised after editing and verified: entry count, forward-slash
  names, stream-readability, new name present / old name absent, contact gone.
- Pristine copies are stored in `backup/mods/`.

```
powershell -ExecutionPolicy Bypass -File scripts\apply_mod_branding.ps1        # dry run
powershell -ExecutionPolicy Bypass -File scripts\apply_mod_branding.ps1 -Swap  # install
```

### OneConfig repository link

`patch_oneconfig_links.ps1` replaces the `https://github.com/Polyfrost/OneConfig`
contact link with `https://discord.gg/reomc` in:

- `OneConfig-*.jar` → `fabric.mod.json`
- `OneConfig-*.jar` → `META-INF/jars/26.3-fabric-*.jar` → `fabric.mod.json`
- `compose-bundle-*.jar` → `fabric.mod.json`

The nested OneConfig jar is rebuilt and normalised before being put back, so the
outer jar stays stream-readable. `homepage` / `issues` contacts are left as-is.

```
powershell -ExecutionPolicy Bypass -File scripts\patch_oneconfig_links.ps1        # dry run
powershell -ExecutionPolicy Bypass -File scripts\patch_oneconfig_links.ps1 -Swap  # install
```

## 4. ReoMods distribution build (`C:\Users\drako\Documents\Default Project\ReoMods`)

`build_reomods.ps1` produces a rebranded distribution copy of every Polyfrost mod
used by the `26.3 Fabric` client. The task manifests are derived from the client's
own bundle indices (Modrinth CDN URLs + sha1), so the exact "used" files are built:

- `manifests/reomods-26.3-fabric.all.json` – all 23 used projects
- `manifests/reomods-26.3-fabric.part-a.json` – Poly\* family + OneConfig + Compose + PolyPlus
- `manifests/reomods-26.3-fabric.part-b.json` – the remaining mods

For each task the script downloads the used file into `ReoMods\_originals` (sha1
verified), then applies:

| transform | detail |
| --- | --- |
| display names | `PolyBlur…PolyWeather` → `Reo*` (`fabric.mod.json` + constant pool) |
| authors | `["drako"]` (outer jar and explicitly listed nested mod jars) |
| description | `OneClient` → `ReoClient`, `Polyfrost` → `Reo` |
| contact | removed, except OneConfig / Compose Multiplatform → `https://discord.gg/reomc` |
| Mod Menu links | `custom.modmenu.links` removed |
| file names | `poly*` jars renamed `reo*`, `polyplus` → `reoplus` |

Nested `META-INF/jars/*` are patched only when listed in the manifest
(`26.3-fabric-*.jar` inside OneConfig, `polydium-1.1.5+26.3.jar` inside PolyPlus);
all other nested libraries keep their own metadata. PolyPlus is taken from the
already fully-branded + featured-server-patched cluster jar (the pristine download
is kept in `_originals`). The script validates entry count, authors, name, contact
policy, old/new display names and stream-readability before publishing.

```
powershell -ExecutionPolicy Bypass -File scripts\build_reomods.ps1 -TasksPath manifests\reomods-26.3-fabric.all.json
```

Re-running is safe: originals are cached in `ReoMods\_originals` and re-verified by
sha1. Use `-OutDir` / `-WorkDir` to build somewhere else.

## 5. All-versions hosting set (`ReoHosting`) — in-game bundle catalog

`generate_reomods_all_manifest.ps1`, `build_reomods.ps1`, `build_reo_bundles.ps1`
and `verify_reo_hosting.ps1` build a hosting-ready replacement for the whole
bundle catalog across all supported versions (1.8.9 ornithe, 1.21.1, 1.21.10,
1.21.11, 26.1.2, 26.2, 26.3 fabric):

- every cached bundle (`…\metadata\caches\bundles\bundles\*.mrpack`) is scanned;
  Modrinth CDN entries whose project id belongs to the Polyfrost organization
  become one task per distinct URL (155 files / 31 bundles / ~768 MB)
- each file gets the same branding policy as the 26.3 set: `Poly*` display/jar
  names → `Reo*`, `authors=["drako"]`, contact removed (OneConfig + Compose →
  `discord.gg/reomc`); OneConfig (`oneconfigv1`) and PolyPlus (`polydium`)
  nested platform jars are auto-detected per version, PolyPlus additionally gets
  the featured-server bytecode patch
- `build_reo_bundles.ps1 -ModBaseUrl <base>` rewrites each index: replaced
  entries drop their cdn.modrinth.com URL (kept, the launcher would treat them
  as Modrinth-managed and download the pristine jar), point at
  `<base>/<outName>`, carry the built jar's sha1/sha512/size plus display
  overrides, and every other entry is copied byte-identical; bundle sha1s and
  `metadata.json` are refreshed
- `verify_reo_hosting.ps1 -ModBaseUrl <base>` re-checks the catalog, entry
  names, external conversion, hashes, overrides and untouched entries

Output layout (`C:\Users\drako\Documents\Default Project\ReoHosting`):

```
oneclient/bundles/metadata.json          catalog, sha1s refreshed
oneclient/bundles/generated/*.mrpack     31 rebuilt bundles
mods/*.jar                               155 rebranded jars (hosted in the repo under ReoMods/)
mods-manifest.json                       name/slug/sha1/sha512/size/url
_originals/                              pristine Polyfrost downloads
```

Hosted on `drako1k/reo-launcher`: jars live in `ReoMods/` on `main` and the
catalog is committed under `oneclient/bundles/` (jar base
`https://raw.githubusercontent.com/drako1k/reo-launcher/main/ReoMods`).

Launcher pointer: `BUNDLES_URL_BASE` in
`packages/reo_common/src/constants.rs` defaults to the Reo repo raw URL
(`https://raw.githubusercontent.com/drako1k/reo-launcher/main`); a
`custom_meta_url_base` override still redirects both the data base and the
bundle base together for local testing. `META_URL_BASE` (versions, TOS,
bad-mods, art) deliberately stays on the live Polyfrost endpoints.

```powershell
# 1. task manifest from the cached bundles (org id map from api.modrinth.com/v3)
powershell -ExecutionPolicy Bypass -File scripts\generate_reomods_all_manifest.ps1 -OrgProjectsPath <polyfrost_org_projects.json>
# 2. build the four partitions (downloads + rebrand + validate)
powershell -ExecutionPolicy Bypass -File scripts\run_reomods_all_build.ps1
# 3. rebuild bundles against the release base + refresh metadata.json
powershell -ExecutionPolicy Bypass -File scripts\build_reo_bundles.ps1 -ModBaseUrl <jar-base>
# 4. verify everything before uploading
powershell -ExecutionPolicy Bypass -File scripts\verify_reo_hosting.ps1 -ModBaseUrl <jar-base>
```

## Layout

- `scripts/trace.mjs` – traces `assets/mask.bin` (alpha mask of the R mark) into polygon loops
- `scripts/gen_assets.ps1` – builds the three SVGs (`REOCLIENT` wordmark + two R marks) and previews
- `scripts/patch_classes.mjs` – early constant-pool patcher (same-length strings, dry-run by default)
- `scripts/finalize.ps1` – lang/json edits, SVG swaps, repack, `javap` validation, `-Swap`
- `scripts/apply_featured_server.ps1` – ReoPlus + featured-server bytecode + nested Polydium
- `scripts/apply_mod_branding.ps1` – Poly\* renames and link removal for all bundled Polyfrost mods
- `scripts/patch_oneconfig_links.ps1` – OneConfig / Compose metadata GitHub link → ReoMC Discord
- `scripts/build_reomods.ps1` – ReoMods distribution build from a task manifest
  (auto Poly\* names, nested platform auto-patching, featured-server patch)
- `scripts/generate_reomods_all_manifest.ps1` – all-version task manifest from the cached bundles
- `scripts/run_reomods_all_build.ps1` – runs the four build partitions in parallel
- `scripts/build_reo_bundles.ps1` – rebuilds the 31 `.mrpack` bundles + `metadata.json` for a host
- `scripts/verify_reo_hosting.ps1` – end-to-end verification of a built hosting tree
- `scripts/rebrand_fabric_mod.js` – fabric.mod.json rewrite (name, authors, contact, description)
- `scripts/CpTool.java` – constant-pool string tool (`--scan`, `--scan-sub`, `--apply`, `--normalize`)
- `scripts/edit_fabric_mod.js` – fabric.mod.json rewrite (name, description, links)
- `manifests/` – ReoMods task manifests (Modrinth URLs + sha1 + branding policy)
- `scripts/capture.ps1` – screenshots the running game window (PrintWindow)
- `assets/` – mask, traced loops, generated SVGs, mock previews
- `out/` – built jars, `backup/` – pristine originals (plus `backup/mods/`)

## 6. Free cosmetics + catalog refresh

- `scripts/ReoCosmeticsFree.java` (compiled against ASM 9.11.1 in `lib/`) is
  applied by `build_reomods.ps1` to every built `polyplus` jar. The cosmetics
  catalog reports every cosmetic/emote as owned and all store/cart prices as 0,
  so the in-game store renders everything as FREE.
- `scripts/refresh_reo_catalog.ps1` re-hashes every hosted jar inside the
  committed bundles and rewrites catalog paths to `/reo/bundles/...`. Run it
  after rebuilding jars, before committing the hosting repo.

## Re-applying after a client update

Once the launcher points at the hosted catalog (section 5), a bundle sync or
"Verify" re-installs the Reo-branded jars directly — no manual re-apply needed.
The steps below remain for local setups that do not use the hosted catalog yet,
or for hand-patched versions before they are rebuilt into the hosting set.
PolyPlus / mod updates restore pristine jars; to re-apply:

1. wordmark pass: rebuild with `gen_assets.ps1` + `finalize.ps1 -Swap` (see the script
   header for paths), producing `out/polyplus-<version>.jar`,
2. featured server: `apply_featured_server.ps1 -Swap` (update the version in
   `-JarPath`/`-InstallPath` if it changed),
3. mod family: `apply_mod_branding.ps1 -Swap` (update filenames in the table if versions changed),
4. OneConfig link: `patch_oneconfig_links.ps1 -Swap`.

The launcher detects hand-replaced jars on the next launch and imports them as local
mods (the bundle-owned originals are switched off automatically), so the patches
survive normal launches. Internal ids (`polyplus`, `polyblur`, …) and asset
namespaces never change.
