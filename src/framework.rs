use indexmap::IndexSet;
use ordered_float::OrderedFloat;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::VecDeque;
use std::marker::PhantomData;

use bindings::{
    actor, broadcast, get_game_state, item_at, load_store, save_store, visible_creatures,
    visible_tiles, Command, Guest, Loc,
};

use crate::{
    self as client_utils,
    behaviors::{avoidance_sets, move_towards},
    crdt::{Crdt, CrdtContainer, CrdtMap, Lww},
    distance,
};

pub struct Component<State, B, M>(PhantomData<State>, PhantomData<B>, PhantomData<M>);

impl<S, B, M> Guest for Component<S, B, M>
where
    S: State<B, M> + Serialize + DeserializeOwned + Default,
    B: Crdt + Serialize + DeserializeOwned,
    M: Map + Serialize + DeserializeOwned,
{
    fn step() -> Command {
        let mut memory = match bincode::deserialize::<S>(&load_store()) {
            Ok(memory) => memory,
            Err(e) => {
                println!("Reinitialized memory: {e}");
                S::default()
            }
        };

        if let Some(map) = memory.map() {
            map.update();
        }
        if let Some(broadcast) = memory.broadcast() {
            let (_, actor) = actor();
            for (_, creature) in visible_creatures() {
                if actor.faction == creature.faction {
                    if let Some(other) = creature.broadcast {
                        if let Ok(other) = bincode::deserialize(&other) {
                            broadcast.merge(&other).unwrap();
                        }
                    }
                }
            }
            broadcast.cleanup(get_game_state().turn);
        }
        let command = memory.run();
        if let Some(to_broadcast) = memory.broadcast() {
            broadcast(Some(&bincode::serialize(to_broadcast).unwrap()));
        }
        save_store(&bincode::serialize(&memory).unwrap());
        command
    }

    fn editor_config() -> Option<Vec<u8>> {
        None
    }
}

pub trait State<Broadcast, Map> {
    fn run(&mut self) -> Command {
        Command::Nothing
    }
    fn broadcast(&mut self) -> Option<&mut Broadcast> {
        None
    }
    fn map(&mut self) -> Option<&mut Map> {
        None
    }
}

pub trait Map {
    fn update(&mut self);
}

#[derive(Default, Debug, Serialize, Deserialize, CrdtContainer)]
pub struct ExplorableMap {
    #[crdt]
    pub map: CrdtMap<Loc, bool, Lww>,
    #[crdt]
    pub seen_items: CrdtMap<Loc, Option<String>, Lww>,
    pub unexplored_locs: IndexSet<Loc>,
    pub explore_target: Option<Loc>,
    pub current_path: Option<VecDeque<Loc>>,
}

impl Map for ExplorableMap {
    fn update(&mut self) {
        let now = get_game_state().turn;
        for (loc, tile) in visible_tiles() {
            self.unexplored_locs.shift_remove(&loc);
            self.map.insert(loc, tile.passable, now);
            if tile.passable {
                for dx in -1..2 {
                    for dy in -1..2 {
                        let mut n = loc;
                        n.x += dx;
                        n.y += dy;
                        if !self.map.contains_key(&n) {
                            self.unexplored_locs.insert(n);
                        }
                    }
                }
                if let Some(item) = item_at(loc) {
                    self.seen_items.insert(loc, Some(item.name), now);
                } else {
                    self.seen_items.insert(loc, None, now);
                }
            }
        }
    }
}

impl ExplorableMap {
    pub fn explore(&mut self) -> Option<Command> {
        if let Some(loc) = self.explore_target {
            if visible_tiles().into_iter().any(|(l, _)| l == loc) {
                self.explore_target = None;
            }
        }

        let (current_loc, _) = actor();

        if self.explore_target.is_none() {
            if !self.unexplored_locs.is_empty() {
                let loc = self
                    .unexplored_locs
                    .iter()
                    .min_by_key(|loc| OrderedFloat(distance(**loc, current_loc)));
                self.explore_target = loc.copied();
            }
        }

        if let Some(loc) = self.explore_target {
            let (blocked, avoid) = avoidance_sets(1);
            move_towards(&mut self.current_path, &self.map, &blocked, &avoid, loc)
        } else {
            None
        }
    }

    pub fn nearest(&mut self, tys: &[impl AsRef<str>]) -> Option<Loc> {
        let mut nearest = None;
        let mut nearest_ty = None;
        let mut nearest_d = std::f32::INFINITY;
        let (current_loc, _) = actor();
        for (loc, l_ty) in self.seen_items.iter() {
            if let Some(l_ty) = l_ty {
                if let Some(i) = tys.iter().position(|ty| ty.as_ref() == l_ty) {
                    if nearest_ty.map(|ty_i| ty_i >= i).unwrap_or(true) {
                        let d = distance(current_loc, *loc);
                        if nearest_ty.map(|ty_i| ty_i > i).unwrap_or(true) || d < nearest_d {
                            nearest_ty = Some(i);
                            nearest = Some(*loc);
                            nearest_d = d;
                        }
                    }
                }
            }
        }

        nearest
    }

    pub fn move_towards_nearest(&mut self, tys: &[impl AsRef<str>]) -> Option<Command> {
        let nearest = self.nearest(tys);

        if let Some(loc) = nearest {
            self.move_towards(loc)
        } else {
            None
        }
    }

    pub fn move_towards(&mut self, loc: Loc) -> Option<Command> {
        let (blocked, avoid) = avoidance_sets(1);
        move_towards(&mut self.current_path, &self.map, &blocked, &avoid, loc)
    }
}
