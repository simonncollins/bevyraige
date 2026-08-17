#!/usr/bin/env bash
#
# Build the web bundle into `web-build/`.
#
# TEMPLATE. Two things to change: `CRATE` below, and the
# `*_FIREBASE_CONFIG_JSON` variable if your game has a backend to bake in (drop
# that block entirely if it does not).
#
#   ./build/web/package.sh                 # release, optimised if wasm-opt is around
#   GAME_BACKEND_CONFIG_JSON="$(cat firebase_config.json)" ./build/web/package.sh
#
# The output is exactly what `pixelfireplace.dev` expects: an `index.html`, the
# wasm-bindgen glue, `assets/`, and `pixelfireplace.json`. `build/web/publish.sh`
# posts it; CI does the same thing through the reusable workflow.
#
# The one thing this script cannot infer is the backend. A browser tab has no
# filesystem and no environment, so `firebase_config.json` cannot be read at
# runtime — `GAME_BACKEND_CONFIG_JSON` bakes it in at compile time
# instead. Without it the bundle still builds and still plays; it just has no
# opponents, because it has no database to find them in.

set -euo pipefail

CRATE=${CRATE:-CHANGEME}
OUT=${OUT:-web-build}
TARGET=wasm32-unknown-unknown
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

cd "$ROOT"

# ── Prerequisites ─────────────────────────────────────────────────────────────

if ! rustup target list --installed | grep -qx "$TARGET"; then
  echo "Adding the ${TARGET} target..."
  rustup target add "$TARGET"
fi

# **The CLI version must match the crate exactly.** wasm-bindgen's generated
# glue carries a schema version and refuses to run against a different one, and
# the crate side is not ours to move: `js-sys` pins `wasm-bindgen = "=x.y.z"`.
# So `cargo install wasm-bindgen-cli --locked`, which installs the latest, is
# almost guaranteed to be wrong — the failure is a twenty-line message about
# schema versions in the middle of a build. Read the pin and ask for that.
BINDGEN_VERSION="$(
  awk '/^name = "wasm-bindgen"$/ { found = 1; next }
       found && /^version = / { gsub(/[version ="]/, ""); print; exit }' Cargo.lock
)"
[ -n "$BINDGEN_VERSION" ] || { echo "error: no wasm-bindgen in Cargo.lock" >&2; exit 1; }

install_hint="cargo install -f wasm-bindgen-cli --version $BINDGEN_VERSION"

if ! command -v wasm-bindgen >/dev/null 2>&1; then
  echo "error: wasm-bindgen not found. Install the version this project pins:" >&2
  echo "  $install_hint" >&2
  exit 1
fi

HAVE="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$HAVE" != "$BINDGEN_VERSION" ]; then
  echo "error: wasm-bindgen $HAVE is installed, but this project pins $BINDGEN_VERSION." >&2
  echo "       The two must match exactly — the glue carries a schema version." >&2
  echo "  $install_hint" >&2
  exit 1
fi

if [ -z "${GAME_BACKEND_CONFIG_JSON:-}" ]; then
  echo "note: no GAME_BACKEND_CONFIG_JSON — this bundle will run offline."
fi

# ── Build ─────────────────────────────────────────────────────────────────────

echo "Building ${CRATE} for ${TARGET}..."
cargo build --profile web --target "$TARGET" --bin "$CRATE"

# `--profile web` outputs under `web/`, not `release/`.
WASM="target/$TARGET/web/${CRATE//-/_}.wasm"
[ -f "$WASM" ] || { echo "error: expected $WASM" >&2; exit 1; }

rm -rf "$OUT" && mkdir -p "$OUT"

# `--target web` emits an ES module the shell imports directly, with no bundler.
# `--out-name game` is what index.html imports; keep the two in step.
wasm-bindgen --no-typescript --target web --out-dir "$OUT" --out-name game "$WASM"

# ── Shrink ────────────────────────────────────────────────────────────────────
#
# Optional, and worth a lot: a Bevy build is tens of megabytes before this and
# the browser downloads every byte before the first frame.
#
# **Every feature current rustc emits has to be named**, or wasm-opt refuses the
# module its own toolchain just produced:
#
#   [wasm-validator error in function 0] unexpected false:
#   all used features should be allowed, on (i32.extend16_s ...)
#
# `i32.extend16_s` is sign-extension, on by default in rustc for
# wasm32-unknown-unknown and *not* in an older binaryen's default feature set.
# The list below is what the target enables; naming them explicitly means a
# binaryen too old to assume them still works.
#
# It is also why `apt-get install binaryen` is not enough on its own — Ubuntu
# 24.04 ships **version 108**, from early 2023. See MIN_WASM_OPT.
WASM_OPT_FEATURES=(
  --enable-sign-ext
  --enable-mutable-globals
  --enable-multivalue
  --enable-reference-types
  --enable-bulk-memory
  --enable-nontrapping-float-to-int
)

# Below this, binaryen predates features rustc now emits unconditionally.
MIN_WASM_OPT=116

if command -v wasm-opt >/dev/null 2>&1; then
  HAVE_OPT="$(wasm-opt --version | awk '{print $3}')"
  if [ "${HAVE_OPT:-0}" -lt "$MIN_WASM_OPT" ] 2>/dev/null; then
    echo "error: wasm-opt $HAVE_OPT is older than $MIN_WASM_OPT." >&2
    echo "       It will reject instructions this toolchain emits." >&2
    echo "       Ubuntu's apt version is too old; take a release from" >&2
    echo "       https://github.com/WebAssembly/binaryen/releases" >&2
    exit 1
  fi
  echo "Optimising with wasm-opt ${HAVE_OPT} ($(du -h "$OUT/game_bg.wasm" | cut -f1) before)..."
  wasm-opt -Oz "${WASM_OPT_FEATURES[@]}" \
    -o "$OUT/game_bg.wasm.opt" "$OUT/game_bg.wasm"
  mv "$OUT/game_bg.wasm.opt" "$OUT/game_bg.wasm"
  echo "  $(du -h "$OUT/game_bg.wasm" | cut -f1) after"
else
  echo "note: wasm-opt not found (brew install binaryen) — bundle will be large."
fi

# ── Payload ───────────────────────────────────────────────────────────────────

cp -r assets "$OUT/assets"
cp index.html "$OUT/index.html"
cp pixelfireplace.json "$OUT/pixelfireplace.json"
# The card art. Outside `assets/` on purpose: it is for the site, not the game,
# and nothing should download it to play. Without it the card shows the title's
# first letter.
cp cover.png "$OUT/cover.png"

echo
echo "Built $OUT ($(du -sh "$OUT" | cut -f1)):"
ls -la "$OUT"
echo
echo "Serve it with any static server that sets the wasm MIME type, e.g."
echo "  python3 -m http.server -d $OUT 8080"
