# The conformance harness pattern

This one is a **shape, not a library**. Its types name your game's states, roles
and terrain operations, so it cannot be handed over as code — but the shape is
worth copying, and the mistakes are worth not repeating.

## What it is for

A scenario is a small piece of static data: a starting world, an input, and
assertions about what should be true at particular ticks.

```rust
Scenario {
    name: "Walker turns at a wall",
    terrain: vec![
        TerrainOp::Fill { x: 0, y: 560, w: 400, h: 160 },
        TerrainOp::Fill { x: 200, y: 520, w: 16, h: 40 },   // the wall
    ],
    entities: vec![Spawn { x: 100, y: 548, state: Walking, dir: Right }],
    assertions: vec![
        Assertion { tick: 20, entity: 0, expect: Direction(Left) },
    ],
}
```

The runner builds a minimal `App` — `MinimalPlugins` + your simulation plugin,
nothing else — applies the terrain, spawns the entities, drives `FixedUpdate` by
hand, and checks the assertions as the ticks pass.

That is all. The value is not in the machinery; it is in what the machinery makes
possible.

## Why it beats ordinary unit tests

**It is transliterated, not invented.** Ours came from the original game's own
fixtures, which meant the suite was an arbiter of *correctness against a
reference*, not a record of what the port happened to do. If you are porting
anything, this is the single highest-value thing you can build.

**It is static data, so it is cheap to have hundreds.** 690 assertions across 19
categories, and adding one is adding a line to a table.

**It steps the same way the agent steps.** Make your live-driving `game/step`
verb advance `FixedUpdate` exactly as the runner does, and a bug an agent finds
poking at a real match converts into a scenario without reinterpretation.

## The trap that makes it worthless

**Do not hand-construct a mid-state to make a mechanic testable.**

We had a role — the Digger — that could not be activated in the game at all. Its
tests were green for months, because every one of them spawned an entity that was
*already digging*. The tests proved the digging code worked. Nobody could reach
it.

So: place the trigger, let the role activate, then assert. Test **reachability,
not just behaviour**. If a scenario cannot express "the player did this thing",
that is a gap in the harness, not a reason to skip the step.

## Assertions that earn their keep

Four features paid for themselves; the rest was noise.

**`known_divergence`** — an assertion you expect to fail, which fails the suite
if it ever starts passing. We proved one fixture was internally inconsistent (no
dig speed satisfies all 74 of its assertions; 2 is the best at 71) and marked the
three that cannot hold. Without this the choice is deleting evidence or living
with a red suite.

**A tolerance window** — `early_by`, `late_by`, `slack`. Some behaviour is right
but a tick out, and a rigid tick number turns a correct port into a failing one.
Be honest about which assertions get slack and why.

**`probe_assertions`** — run a scenario and *print* what actually happened at
each assertion tick instead of failing. This is how you derive a fixture's real
numbers rather than guessing them.

**`trace_scenario`** — dump every tick's state as a table. When an assertion
fails at tick 40, the interesting thing usually happened at tick 12.

## Determinism is a precondition, not a bonus

None of this works if two runs of the same scenario differ. In Bevy that means:

- **Integer positions only.** Mixing float `Transform` with integer simulation
  positions breaks reproducibility. Keep `Transform` render-only.
- **Sort before you mutate.** ECS query iteration order is not stable. Every
  multi-entity system does collect → sort by a stable index → mutate.
- **Order every system explicitly.** Use system sets with a declared order, and
  write down *why* the order is what it is — ours has a load-bearing
  Terrain-before-Movement rule, because the reference implementation is
  entity-major and this port is system-major, and getting it backwards makes
  followers read terrain one tick stale.
- **Restore destructible state on reset.** Keep a pristine copy of the terrain
  and restore it every run, or damage leaks between scenarios and nothing is
  reproducible.

Those four are the difference between a harness and a random number generator.

## Where the agent fits

`bevyraige::devtools::agent` is the other half of the same idea, for the cases a
scenario cannot reach: does the *game* work, as opposed to the simulation.

The division that has held up:

| | |
|---|---|
| **Harness** | Is the mechanic right? Static data, no rendering, no UI, hundreds of cases. |
| **Agent** | Can a player get to it? One live game, real UI, real input, a handful of cases. |

The Digger bug was invisible to the first and obvious to the second. Build both.
