use indexmap::IndexSet;
use ordered_float::OrderedFloat;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::{VecDeque, HashMap};
use std::marker::PhantomData;
use anyhow::Result;

use bindings::{
    actor, broadcast, get_game_state, item_at, load_store, save_store, visible_creatures,
    visible_tiles, Command, Guest, Loc,
};

use crate::{
    behaviors::{avoidance_sets, move_towards},
    crdt::{Crdt, CrdtMap, Lww},
    distance,
};

#[derive(Serialize, Deserialize)]
pub struct DummyMap;
impl Map for DummyMap {
    fn update(&mut self) {
    }
}
#[derive(Serialize, Deserialize)]
pub struct DummyBroadcast;
impl Crdt for DummyBroadcast {
}


pub struct Component<State, B = DummyBroadcast, M = DummyMap>(PhantomData<State>, PhantomData<B>, PhantomData<M>);

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

pub trait State<Broadcast=DummyBroadcast, Map=DummyMap> {
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


#[derive(Default, Debug, Serialize, Deserialize)]
pub struct ExplorableMap {
    pub maps: HashMap<i64, (CrdtMap<Loc, bool, Lww>, CrdtMap<Loc, Option<String>, Lww>, bool)>,
    pub unexplored_locs: IndexSet<Loc>,
    pub explore_target: Option<Loc>,
    pub current_path: Option<VecDeque<Loc>>,
}

impl Map for ExplorableMap {
    fn update(&mut self) {
        let game_state = get_game_state();
        let (map, seen_items, _) = &mut self.maps.entry(game_state.level_id).or_insert_with(|| (Default::default(), Default::default(), game_state.level_is_stable));
        let now = get_game_state().turn;
        for (loc, tile) in visible_tiles() {
            self.unexplored_locs.shift_remove(&loc);
            map.insert(loc, tile.passable, now);
            if tile.passable {
                for dx in -1..2 {
                    for dy in -1..2 {
                        let mut n = loc;
                        n.x += dx;
                        n.y += dy;
                        if !map.contains_key(&n) {
                            self.unexplored_locs.insert(n);
                        }
                    }
                }
                if let Some(item) = item_at(loc) {
                    seen_items.insert(loc, Some(item.name), now);
                } else {
                    seen_items.insert(loc, None, now);
                }
            }
        }

        self.maps.retain(|id, (_, _, is_stable)| *id == game_state.level_id || *is_stable);
    }
}

impl Crdt for ExplorableMap {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (id, (map, seen_items, _)) in self.maps.iter_mut() {
            if let Some((other_map, other_seen_items, _)) = other.maps.get(id) {
                map.merge(other_map)?;
                seen_items.merge(other_seen_items)?;
            }
        }
        Ok(())
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
            let (blocked, avoid) = avoidance_sets(1, None);
            if let Some((map, _, _)) = self.maps.get(&get_game_state().level_id) {
                move_towards(&mut self.current_path, map, &blocked, &avoid, loc)
            } else {
                None
            }
        } else {
            None
        }
    }

    pub fn nearest(&mut self, tys: &[impl AsRef<str>]) -> Option<Loc> {
        let mut nearest = None;
        let mut nearest_ty = None;
        let mut nearest_d = std::f32::INFINITY;
        let (current_loc, _) = actor();
        if let Some((_, seen_items, _)) = self.maps.get(&get_game_state().level_id) {
            for (loc, l_ty) in seen_items.iter() {
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
        if let Some((map, _, _)) = self.maps.get(&get_game_state().level_id) {
            let (blocked, avoid) = avoidance_sets(1, Some(loc));
            move_towards(&mut self.current_path, map, &blocked, &avoid, loc)
        } else {
            None
        }
    }
}
