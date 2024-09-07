use std::collections::BTreeMap;
use std::marker::PhantomData;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{Loc, LocMap, LocSet, LocSetIter};

pub use client_utils_derive::CrdtContainer;

pub trait Crdt {
    fn merge(&mut self, _other: &Self) -> Result<()> {
        Ok(())
    }
    fn cleanup(&mut self, _now: i64) {}
}

#[derive(Default, Serialize, Deserialize)]
pub struct ExpiringFWWRegister<T> {
    value: Option<T>,
    written: i64,
    expires: i64,
}

impl<T> ExpiringFWWRegister<T> {
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn set(&mut self, now: i64, expires: i64, value: T) {
        self.value = Some(value);
        self.written = now;
        self.expires = expires;
    }

    pub fn update_expiry(&mut self, expires: i64) {
        self.expires = expires;
    }
}

impl<T: Clone + PartialEq> Crdt for ExpiringFWWRegister<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        if other.value.is_some() {
            if other.written < self.written {
                self.value = other.value.clone();
                self.written = other.written;
                self.expires = other.expires;
            } else if self.value == other.value && self.expires < other.expires {
                self.expires = other.expires;
            }
        }
        Ok(())
    }

    fn cleanup(&mut self, now: i64) {
        if now >= self.expires {
            self.value = None;
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct ExpiringSet<T: Ord>(BTreeMap<T, i64>);

impl<T: Ord> ExpiringSet<T> {
    pub fn insert(&mut self, v: T, expires: i64) {
        self.0.insert(v, expires);
    }

    pub fn contains(&mut self, v: &T) -> bool {
        self.0.contains_key(v)
    }
}

impl<T: Ord + Clone> Crdt for ExpiringSet<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (v, e) in &other.0 {
            if let Some(expires) = self.0.get_mut(v) {
                if e > expires {
                    *expires = *e;
                }
            } else {
                self.0.insert(v.clone(), *e);
            }
        }
        Ok(())
    }

    fn cleanup(&mut self, now: i64) {
        self.0.retain(|_, expires| *expires < now);
    }
}

#[derive(Serialize, Deserialize)]
pub struct SizedFWWExpiringSet<T: Ord>(BTreeMap<T, (i64, i64)>, usize);

impl<T: Ord> SizedFWWExpiringSet<T> {
    pub fn new(size: usize) -> Self {
        Self(BTreeMap::new(), size)
    }

    pub fn insert(&mut self, v: T, now: i64, expires: i64) {
        self.0.insert(v, (now, expires));
    }

    pub fn contains(&mut self, v: &T) -> bool {
        self.0.contains_key(v)
    }
}

impl<T: Ord + Clone> Crdt for SizedFWWExpiringSet<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (v, (written, expires)) in &other.0 {
            if self.0.len() < self.1 {
                self.0.insert(v.clone(), (*written, *expires));
            } else {
                let mut newest = None;
                let mut newest_written = None;
                for (v, (w, _)) in &self.0 {
                    if w > written {
                        if let Some(n) = newest_written {
                            if n < w {
                                newest_written = Some(w);
                                newest = Some(v.clone());
                            }
                        } else {
                            newest_written = Some(w);
                            newest = Some(v.clone());
                        }
                    }
                }
                if let Some(newest) = newest {
                    self.0.remove(&newest);
                    self.0.insert(v.clone(), (*written, *expires));
                }
            }
        }
        Ok(())
    }

    fn cleanup(&mut self, now: i64) {
        self.0.retain(|_, (_, expires)| *expires < now);
    }
}

#[derive(Debug)]
pub struct Lww;
#[derive(Debug)]
pub struct Fww;

pub struct CrdtMapIter<'a, K, V>(std::collections::btree_map::Iter<'a, K, (V, i64)>);

impl<'a, K, V> Iterator for CrdtMapIter<'a, K, V> {
    type Item = (&'a K, &'a V);
    fn next(&mut self) -> Option<Self::Item> {
        if let Some((k, (v, _))) = self.0.next() {
            Some((k, v))
        } else {
            None
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrdtMap<K: Ord, V, P>(BTreeMap<K, (V, i64)>, PhantomData<P>);

impl<K: Ord, V, P> Default for CrdtMap<K, V, P> {
    fn default() -> Self {
        Self(BTreeMap::new(), PhantomData::default())
    }
}

impl<K: Ord, V, P> CrdtMap<K, V, P> {
    pub fn insert(&mut self, k: K, v: V, now: i64) {
        self.0.insert(k, (v, now));
    }

    pub fn contains_key(&mut self, k: &K) -> bool {
        self.0.contains_key(k)
    }

    pub fn iter(&self) -> CrdtMapIter<K, V> {
        CrdtMapIter(self.0.iter())
    }
}

impl<K: Ord + Clone, V: Ord + Clone> Crdt for CrdtMap<K, V, Lww> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (k, (v, written)) in &other.0 {
            if let Some((lv, lw)) = self.0.get_mut(k) {
                if *lw < *written || (*lw == *written && *lv < *v) {
                    *lw = *written;
                    *lv = v.clone();
                }
            } else {
                self.0.insert(k.clone(), (v.clone(), *written));
            }
        }
        Ok(())
    }
}

impl<K: Ord + Clone, V: Ord + Clone> Crdt for CrdtMap<K, V, Fww> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (k, (v, written)) in &other.0 {
            if let Some((lv, lw)) = self.0.get_mut(k) {
                if *lw > *written || (*lw == *written && *lv > *v) {
                    *lw = *written;
                    *lv = v.clone();
                }
            } else {
                self.0.insert(k.clone(), (v.clone(), *written));
            }
        }
        Ok(())
    }
}

impl<V, P> LocSet for CrdtMap<Loc, V, P> {
    fn contains_loc(&self, loc: &Loc) -> bool {
        self.0.contains_key(loc)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn iter(&self) -> LocSetIter {
        LocSetIter {
            inner: Box::new(self.0.keys().copied()),
        }
    }
}

impl<P> LocMap for CrdtMap<Loc, bool, P> {
    fn get_loc(&self, loc: &Loc) -> Option<bool> {
        self.0.get(loc).copied().map(|(l, _)| l)
    }
}
