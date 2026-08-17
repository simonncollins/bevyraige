# Writing the game-specific bridge

`beveraige::devtools::agent` gives you generic verbs — press a named button, move
the pointer, click, type, step the simulation, screenshot. Those are enough to
*operate* a game and not enough to *understand* one. The bridge is the layer that
speaks your game's vocabulary, and it is the part you write.

This is the worked example the module docs point at, from a real project.

## Registering

```rust
app.add_plugins(
    agent::control::with_agent_methods(RemotePlugin::default())
        .with_method("game/place_trigger", place_trigger)
        .with_method("game/terrain_ascii", terrain_ascii)
        .with_method("game/sim_state", sim_state)
        .with_method("game/level_info", level_info),
);
```

One endpoint, two namespaces: `agent/*` is generic, `game/*` is yours. A verb is
an ordinary system taking `In<Option<Value>>` and returning `BrpResult`, with the
whole `World` available.

`RemotePlugin::with_method` **consumes** the plugin, so all registration has to
happen in one place. Ours is a `with_game_methods` function chained from the
debug plugin; a `Plugin::build` that tries to register a method later cannot,
which is why our spatial-reasoning plugin is an empty stub and that is correct.

## The four verbs that mattered

Ranked by how often they were used, which is not the order we built them in.

### `game/step {"ticks": N}` — advance exactly N and stop

**The most important verb, and the least obvious.** An agent inspects, and
inspection over a network takes time the simulation does not stop for. Ask where
the entities are, then ask what the terrain under them looks like, and the two
answers describe two different worlds — with no way to tell from the answers.

That is not hypothetical. Two reads a few seconds apart looked exactly like a
physics bug: entities in state `Walking` standing on empty air. They were not.
3,700 ticks had passed between the calls, the stalemate cutoff had fired,
everyone had been force-bombed, and the explosions had destroyed the ground.
**Two correct readings, one wrong conclusion.**

Pausing fixes the race but not the diagnosis: a paused world still cannot say
*which* tick it is showing, so a finding cannot be replayed. Stepping fixes both.

Use it instead of sleeping. Always.

### `game/terrain_ascii` — the board as text

```text
     0    8   16   24   32
  0  ....................
  8  ....####............
 16  ....#..#....@.......
     # solid  @ entity  ~ bridge
```

**This is how an agent reads a level; a screenshot is not.** A screenshot needs a
vision model, costs tokens, and cannot be diffed. Text can be grepped, compared
between ticks, and pasted into a bug report.

Give it `x`, `y`, `width`, `height` and a **`step`** to subsample, so the whole
level fits in one call at low resolution and detail is a second call. Include
rulers and a legend in the output — an agent that has to remember what `#` means
will eventually get it wrong.

### `game/place_trigger` — set up a position in one call

The pointer can do this, and should be *able* to, but a test that has to drag
things around to reach its starting state spends most of its steps on setup. A
verb that puts the world in a named state directly is worth having even when the
long way round works.

Keep both. The pointer path is the only way to exercise dragging, deletion, and
anything needing two clicks — and those broke twice in ways the direct verb could
never have caught.

### `game/sim_state` — the scoreboard in one line

```json
{"tick": 90, "alive": [5, 0], "rescued": [0, 0], "entity_count": 5}
```

Cheap, and it answers "did anything happen" without a screenshot.

## What makes a verb good

**Return what the agent needs to decide the next step, not `Ok`.** A verb that
reports nothing forces a screenshot after every call, and that is the difference
between a test that costs a few thousand tokens and one that costs a hundred
thousand.

**Reflect your resources instead of adding verbs to read them.** Anything
`#[derive(Reflect)]` + `#[reflect(Resource)]` is readable over
`world.get_resources` for free, which is better than a bespoke getter. We
reflected the selected role, the role counts, the pointer position in sim
coordinates, the army sizes, the exit zones and the spawn points — and deleted
four verbs.

**Name every button you might drive.** Not because pixels are brittle, but
because **pixels do not work at all**: Bevy's `ui_focus_system` only hit-tests
cameras rendering to a *window*, and headless retargets every camera to an image,
so `Interaction` is never set however good the coordinates are. `agent/press`
matching on `Name` exists precisely to replace it.

If two screens name buttons after the same things, prefix them. Ours has sixteen
roles in a shop *and* sixteen in an in-match bar; `Role Builder` and `Builder`
are different buttons, and "Builder" meaning either is exactly the ambiguity
`agent/press` cannot resolve.

**Give a reset verb that really resets.** Ours revives triggers rather than
removing them, so `game/clear_triggers` exists as the way to start a plan over.
Find out what your reset actually does before you rely on it.

## The workflow this enables

```bash
BEVY_ASSET_ROOT=$PWD MYGAME_HEADLESS=1280x720 MYGAME_START_STATE=playing ./target/debug/mygame &
sleep 6
brp agent/ui '{}'                          # what is on screen, and its names
brp agent/press '{"name":"Play"}'
brp game/terrain_ascii '{"step":8}'        # read the level
brp game/place_trigger '{"x":300,"y":600,"role":"Builder"}'
brp game/step '{"ticks":60}'               # advance a known amount
brp game/sim_state '{}'                    # did it work
brp agent/screenshot '{"path":"/tmp/s.png"}'   # only when a human needs to look
```

The screenshot is last on purpose. It is for the questions text cannot answer —
and it does answer some: a banner rendered white-on-white, a dialog tab that was
light text on a light block, and a minimap sitting on top of the goal were all
things every test passed and one look caught.
