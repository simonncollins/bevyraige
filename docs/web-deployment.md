# Shipping a Bevy game to the web

Six ways it fails, in the order you will meet them. Every one of these cost a CI
run or a browser session to find; none of them is in the Bevy book.

The short version, if you only read one thing:

> **Pin every tool.** `wasm-bindgen` and `binaryen` both refuse to work against a
> version they disagree with, and both are things a package manager will happily
> give you the wrong version of. Two of the six failures below are that same
> mistake made twice.

## 0. The build

```bash
rustup target add wasm32-unknown-unknown
cargo install -f wasm-bindgen-cli --version "$(pinned_version)"   # see §1
cargo build --profile web --target wasm32-unknown-unknown --bin mygame
wasm-bindgen --no-typescript --target web --out-dir web-build --out-name game \
  target/wasm32-unknown-unknown/web/mygame.wasm
wasm-opt -Oz <features…> -o web-build/game_bg.wasm web-build/game_bg.wasm   # §2
cp -r assets web-build/assets
```

`template/build/web/package.sh` is this, with the checks. Run the *same script*
in CI that you run locally — two build paths for one artifact is how one of them
silently breaks.

## 1. `wasm-bindgen`: the CLI must match the crate exactly

```text
it looks like the Rust project used to create this Wasm file was linked against
version of wasm-bindgen that uses a different bindgen format than this binary:
  rust Wasm file schema version: 0.2.111
     this binary schema version: 0.2.127
```

The generated glue carries a schema version and the CLI refuses to run against a
different one. **The crate side is not yours to choose**: `js-sys` pins
`wasm-bindgen` with `=`, so `cargo update -p wasm-bindgen --precise` fails.

So `cargo install wasm-bindgen-cli --locked` — which installs the newest — is
almost guaranteed to be wrong. Read the pin out of your lockfile:

```bash
VERSION="$(awk '/^name = "wasm-bindgen"$/ { f = 1; next }
                f && /^version = / { gsub(/[version ="]/, ""); print; exit }' Cargo.lock)"
cargo install -f wasm-bindgen-cli --version "$VERSION"
```

## 2. `wasm-opt`: an old binaryen rejects what rustc emits

```text
[wasm-validator error in function 0] unexpected false:
all used features should be allowed, on (i32.extend16_s ...)
```

`i32.extend16_s` is the sign-extension proposal, on by default in rustc for
`wasm32-unknown-unknown`. An older binaryen does not assume it is allowed.

**Ubuntu 24.04 ships binaryen 108**, from early 2023. `apt-get install binaryen`
is not good enough. Take a release:

```bash
BINARYEN=version_132
curl -sSfL -o /tmp/binaryen.tar.gz \
  "https://github.com/WebAssembly/binaryen/releases/download/${BINARYEN}/binaryen-${BINARYEN}-x86_64-linux.tar.gz"
tar -xzf /tmp/binaryen.tar.gz -C /tmp
export PATH="/tmp/binaryen-${BINARYEN}/bin:$PATH"
```

And name the features anyway, so an older one either works or fails loudly:

```bash
wasm-opt -Oz \
  --enable-sign-ext --enable-mutable-globals --enable-multivalue \
  --enable-reference-types --enable-bulk-memory --enable-nontrapping-float-to-int \
  -o out.wasm in.wasm
```

On a current binaryen these flags change nothing — the output is byte-identical.
That is what insurance looks like.

## 3. Fat LTO does not fit on a hosted runner

```text
   Compiling bevy_kira_audio v0.25.0
Error: Process completed with exit code 143
```

**143 is SIGTERM**, and the location is the diagnosis: not a dependency failing
to compile — every one of them had — but the moment your own crate starts to
link.

`lto = true` is *fat* LTO: the whole program's IR in one process. On Bevy's
dependency graph that wants more than a GitHub-hosted runner has (7 GB, 2 vCPU).
Ubuntu 24.04 runs `systemd-oomd`, which sends **SIGTERM** on memory pressure
rather than the kernel OOM killer's SIGKILL — which is why it reads like a
cancellation instead of a crash, and why 143 rather than 137.

It is not ephemeral. A re-run fails identically; your laptop only succeeds
because it has the headroom.

```toml
[profile.web]
inherits = "release"
lto = "thin"     # per-module against summaries, a fraction of the peak
strip = true     # see §6
```

Thin LTO cost 600 KB over the wire on a real game — 6.5 → 7.1 MB brotli — because
`wasm-opt -Oz` does the size work anyway. A build that does not finish has no
size.

## 4. The canvas: name it, or Bevy makes its own

Without a selector Bevy appends a canvas to `<body>` sized to `Window::default()`
— 1280x720, ignoring the frame it is in — and your shell's loading spinner sits
behind it forever because nothing removed it.

```rust
#[cfg(target_arch = "wasm32")]
fn web_canvas(window: &mut Window) {
    window.canvas = Some("#game-canvas".to_string());
    window.fit_canvas_to_parent = true;
    // Leave this off. It swallows F5, F12 and Ctrl+R while the game has focus,
    // and a game embedded in someone else's page has no business taking the
    // browser's own keys.
    window.prevent_default_event_handling = false;
}
```

The canvas's parent **must not be sized by its children**. A parent that grows to
fit its canvas and a canvas that grows to fit its parent is a feedback loop that
inflates the canvas every frame. `position: absolute; inset: 0` takes its size
from the page instead — see `template/index.html`.

## 5. Bevy throws on a *successful* start

```text
Uncaught RuntimeError: Using exceptions for control flow, don't mind me.
This isn't actually an error!
```

winit runs its event loop by unwinding out of `init()`. A shell that treats every
rejection as a failure hides the game behind an error it never earned:

```js
init().then(() => status.remove()).catch((error) => {
  if (String(error).includes('Using exceptions for control flow')) {
    status.remove();
    return;               // this is success
  }
  showFailure(error);
});
```

## 6. Size: measure the compressed number, not the file

A Bevy game is tens of megabytes of wasm on disk. **That is not the download.**
Any sane host serves it compressed, and wasm compresses hard:

| | on disk | gzip | brotli |
|---|---|---|---|
| Default features | 44.6 MB | 13.4 MB | 8.1 MB |
| Without `3d` | 36.4 MB | 10.9 MB | **6.5 MB** |

Check what your host actually does before optimising anything:

```bash
curl -sI -H "Accept-Encoding: br, gzip" https://yourhost/ | grep -i content-encoding
```

The single biggest lever, if you are 2D: **Bevy 0.18's defaults are `2d`, `3d`
and `ui`.** Drop `3d`.

```toml
bevy = { version = "0.18", default-features = false, features = ["2d", "ui", "png"] }
```

`3d` drags in `bevy_pbr`, `bevy_gltf`, `gltf_animation` and `bevy_anti_alias`.
**The browser will tell you** — the console says `Disabling depth of field`,
`Disabling EnvironmentMapGenerationPlugin` while having downloaded all of it.

`png` has to be named explicitly: it is in neither `default_app` nor
`default_platform`, and every sprite is one. Gizmos survive —
`bevy_gizmos_render` is inside `2d_bevy_render`.

`strip = true` takes the 30 MB `name` section off the linked artifact.
`strip = "debuginfo"` does **not** — there is no DWARF to remove. It does not
shrink the download either, since `wasm-opt` drops the same section; what it buys
is 36 MB less for wasm-opt to read on every publish.

Assets are the other half. Audio dominated ours: 8.6 MB of 11 MB. That is a
judgement about how the game should sound, not a build setting.

## 7. No filesystem, no environment, no clock

A browser tab has none of the three, and the failures are all at runtime.

**Config** cannot be read from a file. Bake it at build time, and check it first
on *every* target so there is one mechanism rather than two:

```rust
pub const BAKED_CONFIG: Option<&str> = option_env!("MYGAME_CONFIG_JSON");
```

An unset GitHub Actions `env:` key is set to `""`, not omitted — so treat empty
as *absent*, or a repository with no secret reports "invalid JSON" and sends you
hunting for a malformed document that never existed.

**The clock panics.** `Instant::now()` and `SystemTime::now()` both panic on
`wasm32-unknown-unknown`, and nothing at the call site suggests they might. Use
`bevyraige::platform` — `Instant` (Bevy's, which is `web_time` on the web),
`now_unix`, `now_unix_millis`.

Search your codebase for `SystemTime::now` before your first web build. Ours had
five copies, one of which had already met this and hardcoded a constant seed, so
the web build opened with the same music track every time.

## 8. Do not let the target rot

The web build was broken for months in our repo and nothing noticed, because
`network` was `#[cfg(not(target_arch = "wasm32"))]` and no CI job ever built the
target. Every feature added meanwhile compiled fine on desktop and left the web
build further behind — twenty-one errors deep by the time anyone looked, and
unplayable even if it had compiled.

One job fixes it. Nothing else would have caught it:

```yaml
- run: cargo clippy --lib --bin mygame --target wasm32-unknown-unknown -- -D warnings
```

And the corollary: **gate as little as possible.** Almost nothing is genuinely
platform-specific. `reqwest` compiles for wasm32, where it wraps `fetch`, so
every URL and payload can be shared; only the task spawn differs, because its
wasm futures are `!Send`:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub fn spawn<F: Future + Send + 'static>(&self, f: F) -> Task<F::Output>
where F::Output: Send + 'static { /* tokio runtime + block_on */ }

#[cfg(target_arch = "wasm32")]
pub fn spawn<F: Future + 'static>(&self, f: F) -> Task<F::Output>
where F::Output: 'static { AsyncComputeTaskPool::get().spawn_local(f) }
```

That is the whole difference between "no networking on the web" and a working
backend.

## Verifying it without a browser you can see

```bash
python3 -m http.server -d web-build 8099 &
"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --headless=new --use-gl=angle --use-angle=swiftshader --enable-unsafe-swiftshader \
  --enable-logging=stderr --v=0 --virtual-time-budget=45000 \
  --dump-dom http://localhost:8099/index.html > dom.html 2> chrome.log
```

Then check three things in `chrome.log` and `dom.html`:

- no `panicked`
- exactly **one** `<canvas`, and it is yours — two means §4
- the loading overlay is gone, so `init()` resolved

**`--disable-gpu` produces a false alarm** worth knowing, because the message
points somewhere else entirely:

```text
Failed to create wgpu surface: … "canvas.getContext() returned null;
webgl2 not available or canvas already in use"
```

*"canvas already in use"* reads like two things fighting over one element. There
was simply no WebGL2 at all. Use the swiftshader flags above.
