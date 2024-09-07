use indexmap::IndexSet;
use std::collections::VecDeque;

use bindings::{
    actions, actor, game::auto_rogue::types::ConvertParams, inventory, visible_creatures,
    visible_items, ActionTarget, AttackParams, Command, Loc, MicroAction,
};

use crate::{distance, LocMap, LocSet};

#[macro_export]
macro_rules! find_action {
    ($pattern:pat $(if $guard:expr)?) => {
        {
            let mut found = None;
            'outer: for (i, action) in actions().into_iter().enumerate() {
                for m in &action.micro_actions {
                    if match m {
                        $pattern $(if $guard)? => true,
                        _ => false
                    } {
                        let m = m.clone();
                        found = Some((i, action, m));
                        break 'outer
                    }
                }
            }
            found
        }
    };
    ($t:pat, $item:expr) => {
        {
            let mut found = None;
            'outer: for (i, action) in $item.actions.into_iter().enumerate() {
                for m in &action.micro_actions {
                    if matches!(m, $t) {
                        let m = m.clone();
                        found = Some((i, action, m));
                        break 'outer
                    }
                }
            }
            found
        }
    };
}

pub fn avoidance_sets(creature_margin: u32) -> (IndexSet<Loc>, IndexSet<Loc>) {
    let mut blocked = IndexSet::new();
    let mut creature_margins = IndexSet::new();
    for (loc, _) in visible_creatures() {
        blocked.insert(loc);
        if creature_margin > 0 {
            for dx in -(creature_margin as i32)..creature_margin as i32 + 1 {
                for dy in -(creature_margin as i32)..creature_margin as i32 + 1 {
                    creature_margins.insert(Loc {
                        x: loc.x + dx,
                        y: loc.y + dy,
                    });
                }
            }
        }
    }

    blocked.extend(visible_items().into_iter().filter_map(|(loc, item)| {
        if !item.is_passable || item.name == "Exit" {
            Some(loc)
        } else {
            None
        }
    }));

    (blocked, creature_margins)
}

pub fn move_towards(
    current_path: &mut Option<VecDeque<Loc>>,
    level_map: &dyn LocMap,
    blocked: &dyn LocSet,
    avoid: &dyn LocSet,
    loc: Loc,
) -> Option<Command> {
    println!("move towards: {loc:?}");
    if let Some(path) = current_path {
        if path.iter().last() != Some(&Loc { x: loc.x, y: loc.y }) {
            *current_path = None;
        }
    }
    let (current_loc, _) = actor();
    astar_update_path(current_path, current_loc, loc, level_map, blocked, avoid);
    if let Some(loc) = current_path.as_mut().and_then(|locs| locs.pop_front()) {
        if let Some((id, _, _)) = find_action!(MicroAction::Walk) {
            return Some(Command::UseAction((
                id as u32,
                Some(ActionTarget::Location(Loc { x: loc.x, y: loc.y })),
            )));
        }
    }
    println!("no path");
    None
}

fn astar_update_path(
    path: &mut Option<VecDeque<Loc>>,
    current_location: Loc,
    goal: Loc,
    explored_tiles: &dyn LocMap,
    blocked: &dyn LocSet,
    avoid: &dyn LocSet,
) {
    if let Some(locs) = path {
        for loc in locs {
            if blocked.contains_loc(loc) || avoid.contains_loc(loc) {
                *path = crate::astar(current_location, goal, explored_tiles, blocked, avoid);
                return;
            }
        }
    } else {
        *path = crate::astar(current_location, goal, explored_tiles, blocked, avoid);
    }
}

pub fn attack_nearest() -> Option<Command> {
    if let Some((id, _action, micro_action)) = find_action!(MicroAction::Attack(..)) {
        let mut min_dist = std::f32::INFINITY;
        let mut closest = None;
        for (loc, creature) in visible_creatures() {
            if creature.faction != actor().1.faction {
                let d = distance(Loc { x: 0, y: 0 }, loc);
                if d < min_dist {
                    min_dist = d;
                    closest = Some(loc);
                }
            }
        }

        if let Some(loc) = closest {
            if let MicroAction::Attack(AttackParams { range, .. }) = micro_action {
                if range as f32 >= min_dist {
                    return Some(Command::UseAction((
                        id as u32,
                        Some(ActionTarget::Location(loc)),
                    )));
                }
            }
        }
    }
    None
}

pub fn convert() -> Option<Command> {
    let inventory = inventory();
    if let Some((id, _, ma)) = find_action!(MicroAction::Convert(_)) {
        if let MicroAction::Convert(ConvertParams { input, .. }) = ma {
            let mut to_convert = None;
            for (n, _) in input {
                if let Some(item) = inventory.iter().find(|i| {
                    i.resources
                        .as_ref()
                        .map(|r| r.iter().find(|(nn, _)| nn == &n).is_some())
                        .unwrap_or(false)
                }) {
                    to_convert = Some(item.id);
                    break;
                }
            }
            if let Some(item) = to_convert {
                return Some(Command::UseAction((
                    id as u32,
                    Some(ActionTarget::Items(vec![item])),
                )));
            }
        }
    }
    None
}
