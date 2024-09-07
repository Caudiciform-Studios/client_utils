use indexmap::IndexMap;
use ordered_float::OrderedFloat;
use std::collections::VecDeque;

#[cfg(not(feature = "wit-bindings"))]
use serde::{Deserialize, Serialize};

#[cfg(feature = "wit-bindings")]
pub use bindings::Loc;

#[cfg(not(feature = "wit-bindings"))]
#[derive(
    Default, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Copy, Debug, Serialize, Deserialize,
)]
pub struct Loc {
    pub x: i32,
    pub y: i32,
}

#[cfg(feature = "wit-bindings")]
pub mod behaviors;
pub mod crdt;
#[cfg(feature = "wit-bindings")]
pub mod framework;

pub struct LocSetIter<'a> {
    pub inner: Box<dyn Iterator<Item = Loc> + 'a>,
}

impl<'a> Iterator for LocSetIter<'a> {
    type Item = Loc;
    fn next(&mut self) -> Option<Self::Item> {
        self.inner.next()
    }
}

pub trait LocSet {
    fn contains_loc(&self, loc: &Loc) -> bool;
    fn is_empty(&self) -> bool;
    fn iter(&self) -> LocSetIter;
}

impl LocSet for std::collections::HashSet<Loc> {
    fn contains_loc(&self, loc: &Loc) -> bool {
        self.contains(loc)
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> LocSetIter {
        LocSetIter {
            inner: Box::new(self.iter().copied()),
        }
    }
}

impl LocSet for indexmap::IndexSet<Loc> {
    fn contains_loc(&self, loc: &Loc) -> bool {
        self.contains(loc)
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> LocSetIter {
        LocSetIter {
            inner: Box::new(self.iter().copied()),
        }
    }
}

pub trait LocMap: LocSet {
    fn get_loc(&self, loc: &Loc) -> Option<bool>;
}

impl LocSet for std::collections::HashMap<Loc, bool> {
    fn contains_loc(&self, loc: &Loc) -> bool {
        self.contains_key(loc)
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> LocSetIter {
        LocSetIter {
            inner: Box::new(self.keys().copied()),
        }
    }
}

impl LocMap for std::collections::HashMap<Loc, bool> {
    fn get_loc(&self, loc: &Loc) -> Option<bool> {
        self.get(loc).copied()
    }
}

impl LocSet for indexmap::IndexMap<Loc, bool> {
    fn contains_loc(&self, loc: &Loc) -> bool {
        self.contains_key(loc)
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn iter(&self) -> LocSetIter {
        LocSetIter {
            inner: Box::new(self.keys().copied()),
        }
    }
}
impl LocMap for indexmap::IndexMap<Loc, bool> {
    fn get_loc(&self, loc: &Loc) -> Option<bool> {
        self.get(loc).copied()
    }
}

pub fn distance(a: Loc, b: Loc) -> f32 {
    (((a.x - b.x) as f32).powi(2) + ((a.y - b.y) as f32).powi(2)).sqrt()
}

pub fn astar(
    current_location: Loc,
    goal: Loc,
    explored_tiles: &dyn LocMap,
    blocked: &dyn LocSet,
    avoid: &dyn LocSet,
) -> Option<VecDeque<Loc>> {
    let mut open_set = std::collections::BinaryHeap::new();
    let mut g_scores = IndexMap::new();
    let mut came_from = IndexMap::new();
    open_set.push(std::cmp::Reverse((
        distance(current_location, goal).into(),
        goal,
    )));
    g_scores.insert(goal, 0.0);
    while let Some(std::cmp::Reverse((_, loc))) = open_set.pop() {
        if loc == current_location {
            let mut path = VecDeque::new();
            let mut current = loc;
            while came_from.contains_key(&current) {
                current = came_from[&current];
                path.push_back(current);
            }
            return Some(path);
        }

        let base_score = g_scores.get(&loc).copied().unwrap_or(std::f32::MAX) + 1.0;
        for dx in -1..2 {
            for dy in -1..2 {
                if dx == 0 && dy == 0 {
                    continue;
                }
                let neighboor = Loc {
                    x: loc.x + dx,
                    y: loc.y + dy,
                };
                if explored_tiles.get_loc(&neighboor).unwrap_or(true)
                    && !blocked.contains_loc(&neighboor)
                {
                    let mut score = base_score;
                    if avoid.contains_loc(&neighboor) {
                        score += 10.0;
                    }
                    if score < g_scores.get(&neighboor).copied().unwrap_or(std::f32::MAX) {
                        came_from.insert(neighboor, loc);
                        g_scores.insert(neighboor, score);
                        let f = score + distance(current_location, neighboor);
                        if open_set
                            .iter()
                            .position(|std::cmp::Reverse((_, l))| *l == neighboor)
                            .is_none()
                        {
                            open_set.push(std::cmp::Reverse((OrderedFloat(f), neighboor)));
                        }
                    }
                }
            }
        }
    }
    None
}
