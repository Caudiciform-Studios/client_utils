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

#[derive(Debug, Serialize, Deserialize)]
pub struct ExpiringFWWRegister<T> {
    pub value: Option<T>,
    pub written: i64,
    pub expires: i64,
}

impl <T> Default for ExpiringFWWRegister<T> {
    fn default() -> Self {
        Self {
            value: None,
            written: i64::MAX,
            expires: i64::MIN,
        }
    }
}

impl<T> ExpiringFWWRegister<T> {
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn set(&mut self, value: T, now: i64, expires: i64) {
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

#[derive(Serialize, Deserialize)]
pub struct ExpiringSet<T: Ord>(pub BTreeMap<T, i64>);

impl <T: Ord> Default for ExpiringSet<T> {
    fn default() -> Self {
        ExpiringSet(BTreeMap::new())
    }
}

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

#[derive(Debug, Serialize, Deserialize)]
pub struct SizedFWWExpiringSet<T: Ord>(pub BTreeMap<T, (i64, i64)>, pub usize);

impl<T: Ord> SizedFWWExpiringSet<T> {
    pub fn new(size: usize) -> Self {
        Self(BTreeMap::new(), size)
    }

    pub fn insert(&mut self, v: T, now: i64, expires: i64) {
        if let Some((_, e)) = self.0.get_mut(&v) {
            *e = expires;
        } else if self.0.len() < self.1 {
            self.0.insert(v, (now, expires));
        }
    }

    pub fn contains<Q>(&mut self, v: &Q) -> bool
    where
        T: std::borrow::Borrow<Q>,
        Q: Ord + ?Sized,
        {
        self.0.contains_key(v)
    }
}

impl<T: Ord + Clone> Crdt for SizedFWWExpiringSet<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        for (v, (written, expires)) in &other.0 {
            if let Some((w, e)) = self.0.get_mut(v) {
                *w = (*w).min(*written);
                *e = (*e).max(*expires);
            } else if self.0.len() < self.1 {
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
        self.0.retain(|_, (_, expires)| *expires > now);
    }
}


#[cfg(test)]
mod sized_set_tests {
    use super::*;

    #[test]
    fn test_insert_with_capacity() {
        let mut s = SizedFWWExpiringSet::new(3);
        let now = 0;
        s.insert("a".to_string(), now, now+10);
        assert!(s.contains("a"));
        s.insert("b".to_string(), now, now+10);
        assert!(s.contains("b"));
        s.insert("c".to_string(), now, now+10);
        assert!(s.contains("c"));
        s.insert("d".to_string(), now, now+10);
        assert!(!s.contains("d"));
    }

    #[test]
    fn test_cleanup() {
        let mut s = SizedFWWExpiringSet::new(3);
        let now = 0;
        s.insert("a".to_string(), now, now+1);
        s.insert("b".to_string(), now, now+2);
        s.insert("c".to_string(), now, now+3);

        s.cleanup(now);
        assert!(s.contains("a"));
        assert!(s.contains("b"));
        assert!(s.contains("c"));

        let now = 1;
        s.cleanup(now);
        assert!(!s.contains("a"));
        assert!(s.contains("b"));
        assert!(s.contains("c"));

        let now = 2;
        s.cleanup(now);
        assert!(!s.contains("a"));
        assert!(!s.contains("b"));
        assert!(s.contains("c"));

        let now = 3;
        s.cleanup(now);
        assert!(!s.contains("a"));
        assert!(!s.contains("b"));
        assert!(!s.contains("c"));

    }

    #[test]
    fn test_merge() {
        let mut a = SizedFWWExpiringSet::new(3);
        a.insert("a".to_string(), 0, 10);
        a.insert("b".to_string(), 1, 10);
        a.insert("c".to_string(), 2, 10);

        let mut b = SizedFWWExpiringSet::new(3);
        b.insert("d".to_string(), 3, 10);

        a.merge(&b).unwrap();
        assert!(a.contains("a"));
        assert!(a.contains("b"));
        assert!(a.contains("c"));
        assert!(!a.contains("d"));

        let mut c = SizedFWWExpiringSet::new(3);
        c.insert("d".to_string(), 0, 10);
        a.merge(&c).unwrap();

        assert!(a.contains("a"));
        assert!(a.contains("b"));
        assert!(!a.contains("c"));
        assert!(a.contains("d"));
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
pub struct CrdtMap<K: Ord, V, P>(pub BTreeMap<K, (V, i64)>, PhantomData<P>);

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
