# bevyraige

Bevy tooling that is not about any particular game: play it with no window and
drive it over the wire, capture frames, boot past the menus, diff two runs — and
ship the thing to the web without discovering the six failure modes one CI run at
a time.

Extracted from a working project after every piece here had earned its place by
fixing something.

```toml
[dependencies]
bevyraige = { git = "https://github.com/simonncollins/bevyraige" }
```

```rust
app.add_plugins(DevToolsPlugin::<AppState>::new("MYGAME"));
```

That is the whole installation. Everything is driven by environment variables and
is **inert without them**, so it can ship in a release build for the cost of a
branch at startup.

```bash
MYGAME_HEADLESS=1280x720      # no window at all; drive it over BRP on :15702
MYGAME_START_STATE=playing    # boot past the menus
MYGAME_CAPTURE=/tmp/shot.png  # write a frame
MYGAME_CAPTURE_AT=500         # on this frame (default 180)
MYGAME_CAPTURE_EXIT=1         # and quit
MYGAME_OPTIONS='{"music_volume":0}'   # a game-defined settings payload
```

`AppState` must implement `FromStr` — that is what turns `MYGAME_START_STATE` into
a state. The compiler will tell you; the crate docs show the impl.

## What is in here

| | |
|---|---|
| `src/devtools/` | The code. Headless mode, synthetic input, UI hit-testing by name, tick-exact stepping, the BRP transport, frame capture, run diffing. |
| `src/platform.rs` | The clock, which **panics** on `wasm32-unknown-unknown` if you use `std::time`. |
| `docs/web-deployment.md` | How to ship a Bevy game to the web, and the six ways it fails first. Read this one before your first web build. |
| `docs/agent-bridge.md` | Writing the game-specific half — the verbs that made agent-driven testing worth doing. |
| `docs/harness.md` | The conformance-harness pattern. A shape, not a library: its types name your game's states. |
| `template/` | Files to copy: the web build script, an HTML shell that works, a CI workflow with the pins already in it. |

## Why headless, and the one thing it costs you

**A game window takes the focus of whatever machine it opens on.** That confines
agent-driven testing to a machine nobody is using, and on macOS it cannot be
fixed with a window at all — winit calls `activateIgnoringOtherApps`
unconditionally and Bevy exposes no hook. So headless removes the window and
renders offscreen on the real GPU. Input is injected inside the process, where it
can neither escape onto the desktop nor be corrupted by someone typing.

The cost, and it is worth knowing before you design a test: **Bevy's
`ui_focus_system` only hit-tests cameras rendering to a window**, and headless
retargets every camera to an image. `Interaction` is therefore never set, however
good your coordinates are.

So `agent/press {"name": "Play"}` matches on the `Name` component, and **a button
worth driving in a test needs a `Name`**. That is the contract. It is a small
price and it makes tests read better than pixel coordinates ever did.

## The two-suite division

Worth stating because it took a while to see, and because getting it wrong hides
real bugs:

| | |
|---|---|
| **A conformance harness** | Is the mechanic right? Static data, no rendering, hundreds of cases. |
| **This** | Can a player *get* to it? One live game, real UI, real input, a handful of cases. |

We had a role that could not be activated in the game at all, sitting behind 1,236
green tests, because every test spawned an entity already in the state the role
produces. The first suite cannot see that. The second sees it immediately.

Build both. `docs/harness.md` covers the other half.

## Status

The code compiles for native and `wasm32-unknown-unknown`, and its own tests pass
on both. It is a real extraction rather than a published crate: version 0.1.0, no
release cadence, and the API is whatever the source says.

Bevy 0.18.
