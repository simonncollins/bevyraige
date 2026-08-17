//! Pressing UI from outside, by name or by pixel.
//!
//! # Why this exists at all
//!
//! `bevy_ui::focus::ui_focus_system` only hit-tests cameras whose render target
//! is a *window*: it resolves `NormalizedRenderTarget::Window` and returns early
//! for anything else. [Headless mode](super::headless) renders to an image, so
//! `Interaction` is never set there by the engine — every button is unpressable.
//! This restores the missing half against the same layout data the engine uses:
//! [`UiStack`] for depth order, [`ComputedNode::contains_point`] for the test.
//!
//! # Two ways to press, and when each is right
//!
//! [`PressTarget::Named`] finds a button by its [`Name`] and presses it wherever
//! it happens to be. That is what a *behavioural* test wants — it keeps working
//! when the layout moves, which is most of the time and not a bug.
//!
//! [`PressTarget::Point`] presses whatever is under a pixel. That is what a
//! *layout* test wants, and it is the only one of the two that can fail when a
//! button drifts somewhere unreachable, is covered by an overlay, or is scrolled
//! out of a clipped list. A named press sails straight past all three, because it
//! never asks where the button is.
//!
//! Use the named form to drive, and the pixel form to check that what the player
//! sees agrees with what the agent is driving.

use bevy::prelude::*;
use bevy::ui::{ComputedNode, Interaction, Node, Overflow, UiGlobalTransform, UiStack, UiSystems};

/// What to press.
#[derive(Debug, Clone, PartialEq)]
pub enum PressTarget {
    /// The button carrying this [`Name`].
    Named(String),
    /// Whatever is topmost under this point, in physical pixels from the
    /// top-left of the frame. Finding nothing there is an error.
    Point(Vec2),
    /// Whatever is topmost under this point, **and nothing is fine**.
    ///
    /// What a click at a pixel means: it lands on a button or it lands on the
    /// game, and only the caller knows which it wanted. `agent/click` issues one
    /// of these alongside the mouse tap so that a single verb works on both,
    /// which is how a real click behaves — and reporting "nothing pressable"
    /// every time an agent clicks the board would bury every genuine miss.
    Pointer(Vec2),
    /// A specific entity, for a caller that already resolved one.
    Entity(Entity),
}

/// A press waiting to be applied, or being held for its frame.
#[derive(Resource, Default)]
pub struct AgentUi {
    queued: Vec<PressTarget>,
    /// Pressed last frame; released this one.
    holding: Vec<Entity>,
    /// Why the last press failed, if it did.
    last_error: Option<String>,
}

impl AgentUi {
    /// Queue a press for the next frame.
    pub fn press(&mut self, target: PressTarget) {
        self.queued.push(target);
    }

    /// The reason the most recent press found nothing, if it found nothing.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// One pressable node, as [`interactable_nodes`] reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct UiProbe {
    pub entity: Entity,
    pub name: Option<String>,
    /// Physical-pixel rect, top-left origin.
    pub rect: Rect,
    pub interaction: Interaction,
    pub visible: bool,
}

/// Restores UI pressing when the engine cannot do it.
pub struct AgentUiPlugin;

impl Plugin for AgentUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AgentUi>();
        // **After `UiSystems::Focus`.** With no window there is no cursor, so
        // `ui_focus_system` sets every `Interaction` back to `None` each frame —
        // running before it would have the engine undo the press in the same
        // frame it was made.
        app.add_systems(PreUpdate, apply_presses.after(UiSystems::Focus));
    }
}

/// The query every hit-test and dump reads.
///
/// Public so a host's own BRP handlers can take the same parameter shape —
/// spelling the tuple inline gives it fresh lifetimes that will not unify.
pub type UiNodes<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static ComputedNode,
        &'static UiGlobalTransform,
        &'static InheritedVisibility,
        Option<&'static Name>,
        Option<&'static Interaction>,
    ),
>;

/// Reading `Interaction` to find a target and writing it to press one are
/// conflicting accesses to the same component, so they cannot be two live
/// queries — Bevy rejects the system at startup (`B0001`). Resolving happens
/// entirely through `p0` and pressing entirely through `p1`, in that order.
///
/// Spelled inline rather than behind the [`UiNodes`] alias: a `ParamSet` member
/// needs its lifetimes elided by the system macro, and naming them through an
/// alias produces a type that is not a valid `SystemParam`. That is also why
/// clippy's "very complex type" is allowed here rather than factored out —
/// factoring it out is the thing that does not compile.
#[allow(clippy::type_complexity)]
fn apply_presses(
    mut agent: ResMut<AgentUi>,
    stack: Res<UiStack>,
    clipping: Query<(&ComputedNode, &UiGlobalTransform, &Node)>,
    parents: Query<&ChildOf>,
    mut access: ParamSet<(
        Query<(
            Entity,
            &ComputedNode,
            &UiGlobalTransform,
            &InheritedVisibility,
            Option<&Name>,
            Option<&Interaction>,
        )>,
        Query<&mut Interaction>,
    )>,
) {
    // Last frame's press comes up first — a press is exactly one frame down, so
    // a handler watching `Changed<Interaction>` sees it once.
    let releasing = std::mem::take(&mut agent.holding);
    if !releasing.is_empty() {
        let mut interactions = access.p1();
        for entity in releasing {
            if let Ok(mut interaction) = interactions.get_mut(entity) {
                interaction.set_if_neq(Interaction::None);
            }
        }
    }

    // Resolve every target before touching any of them.
    let queued = std::mem::take(&mut agent.queued);
    let resolved: Vec<(PressTarget, Option<Entity>)> = {
        let nodes = access.p0();
        queued
            .into_iter()
            .map(|target| {
                let entity = match &target {
                    PressTarget::Entity(entity) => Some(*entity),
                    PressTarget::Named(name) => nodes
                        .iter()
                        .find(|(_, _, _, visible, node_name, interaction)| {
                            interaction.is_some()
                                && visible.get()
                                && node_name.is_some_and(|n| n.as_str() == name)
                        })
                        .map(|(entity, ..)| entity),
                    PressTarget::Pointer(point) | PressTarget::Point(point) => {
                        topmost_at(*point, &stack, &nodes, &clipping, &parents)
                    }
                };
                (target, entity)
            })
            .collect()
    };

    for (target, entity) in resolved {
        let Some(entity) = entity else {
            agent.last_error = Some(match &target {
                // A click that found no button hit the game instead, which is
                // not a failure and must not look like one.
                PressTarget::Pointer(_) => continue,
                PressTarget::Named(name) => format!("no visible pressable node named {name:?}"),
                PressTarget::Point(p) => format!("nothing pressable at ({}, {})", p.x, p.y),
                PressTarget::Entity(e) => format!("{e} is not pressable"),
            });
            continue;
        };

        match access.p1().get_mut(entity) {
            Ok(mut interaction) => {
                *interaction = Interaction::Pressed;
                agent.holding.push(entity);
                agent.last_error = None;
            }
            Err(_) => {
                agent.last_error = Some(format!("{entity} has no Interaction component"));
            }
        }
    }
}

/// The topmost pressable node under `point`.
///
/// [`UiStack::uinodes`] is back-to-front, so the first hit walking it backwards
/// is the one a click would land on — an overlay covering a button wins, which is
/// the whole reason a pixel press can fail where a named one cannot.
pub fn topmost_at(
    point: Vec2,
    stack: &UiStack,
    nodes: &UiNodes,
    clipping: &Query<(&ComputedNode, &UiGlobalTransform, &Node)>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    stack.uinodes.iter().rev().copied().find(|&entity| {
        let Ok((_, node, transform, visible, _, interaction)) = nodes.get(entity) else {
            return false;
        };
        interaction.is_some()
            && visible.get()
            && node.contains_point(*transform, point)
            && within_ancestor_clips(point, entity, clipping, parents)
    })
}

/// Whether `point` survives every clipping ancestor.
///
/// `bevy_ui`'s own version of this is private, and without it a node scrolled out
/// of a clipped list still answers a hit test — so an agent would happily "press"
/// an opponent it cannot see.
fn within_ancestor_clips(
    point: Vec2,
    entity: Entity,
    clipping: &Query<(&ComputedNode, &UiGlobalTransform, &Node)>,
    parents: &Query<&ChildOf>,
) -> bool {
    let mut current = entity;
    while let Ok(parent) = parents.get(current) {
        current = parent.parent();
        let Ok((node, transform, ui_node)) = clipping.get(current) else {
            continue;
        };
        if ui_node.overflow == Overflow::visible() {
            continue;
        }
        let clip = node.resolve_clip_rect(ui_node.overflow, ui_node.overflow_clip_margin);
        let Some(local) = transform
            .try_inverse()
            .map(|inverse| inverse.transform_point2(point))
        else {
            return false;
        };
        if !clip.contains(local) {
            return false;
        }
    }
    true
}

/// Every pressable node on screen, topmost first.
///
/// This is the discovery call: it answers "what can I press, and where is it"
/// without a screenshot, which is the difference between an agent that reasons
/// about the UI and one that guesses coordinates off a picture.
pub fn interactable_nodes(stack: &UiStack, nodes: &UiNodes) -> Vec<UiProbe> {
    stack
        .uinodes
        .iter()
        .rev()
        .copied()
        .filter_map(|entity| {
            let (_, node, transform, visible, name, interaction) = nodes.get(entity).ok()?;
            let interaction = (*interaction?).to_owned();
            let half = node.size() / 2.0;
            Some(UiProbe {
                entity,
                name: name.map(|n| n.as_str().to_owned()),
                rect: Rect::from_corners(
                    transform.translation - half,
                    transform.translation + half,
                ),
                interaction,
                visible: visible.get(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A press is one frame down, so a handler watching `Changed<Interaction>`
    /// fires exactly once — the same shape a real click has.
    #[test]
    fn a_press_lasts_one_frame() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<AgentUi>();
        app.add_systems(PreUpdate, apply_presses);
        app.init_resource::<UiStack>();

        let button = app.world_mut().spawn(Interaction::None).id();
        app.world_mut()
            .resource_mut::<AgentUi>()
            .press(PressTarget::Entity(button));

        app.update();
        assert_eq!(
            *app.world().entity(button).get::<Interaction>().unwrap(),
            Interaction::Pressed,
            "down on the frame it was asked for"
        );

        app.update();
        assert_eq!(
            *app.world().entity(button).get::<Interaction>().unwrap(),
            Interaction::None,
            "and up again on the next one"
        );
    }

    /// A press that finds nothing has to say so. Silently doing nothing is the
    /// failure mode that makes an agent believe a broken screen works.
    #[test]
    fn a_press_that_finds_nothing_reports_why() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<AgentUi>();
        app.init_resource::<UiStack>();
        app.add_systems(PreUpdate, apply_presses);

        app.world_mut()
            .resource_mut::<AgentUi>()
            .press(PressTarget::Named("Nonexistent".into()));
        app.update();

        let error = app.world().resource::<AgentUi>().last_error().unwrap_or("");
        assert!(error.contains("Nonexistent"), "{error}");
    }

    #[test]
    fn a_successful_press_clears_the_previous_error() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<AgentUi>();
        app.init_resource::<UiStack>();
        app.add_systems(PreUpdate, apply_presses);

        app.world_mut()
            .resource_mut::<AgentUi>()
            .press(PressTarget::Named("Nope".into()));
        app.update();
        assert!(app.world().resource::<AgentUi>().last_error().is_some());

        let button = app.world_mut().spawn(Interaction::None).id();
        app.world_mut()
            .resource_mut::<AgentUi>()
            .press(PressTarget::Entity(button));
        app.update();
        assert!(app.world().resource::<AgentUi>().last_error().is_none());
    }
}
