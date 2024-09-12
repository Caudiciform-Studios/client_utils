use std::collections::{BTreeMap, BTreeSet};
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

impl<T: PartialOrd + PartialEq> ExpiringFWWRegister<T> {
    pub fn get(&self) -> Option<&T> {
        self.value.as_ref()
    }

    pub fn set(&mut self, value: T, now: i64, expires: i64) {
        if Some(&value) == self.value.as_ref() {
            self.written = self.written.min(now);
            self.expires = self.expires.max(expires);
        } else if self.value.is_none() || now < self.written || (now == self.written && self.value.is_some() && &value < self.value.as_ref().unwrap()) {
            self.value = Some(value);
            self.written = now;
            self.expires = expires;
        }
    }
}

impl<T: Clone + PartialEq + PartialOrd> Crdt for ExpiringFWWRegister<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        if other.value.is_some() {
            if other.written < self.written || (other.written == self.written && other.value < self.value) {
                self.value = other.value.clone();
                self.written = other.written;
                self.expires = other.expires;
            } else if self.value == other.value {
                self.written = self.written.min(other.written);
                self.expires = self.expires.max(other.expires);
            }
        }
        Ok(())
    }

    fn cleanup(&mut self, now: i64) {
        if now >= self.expires {
            self.value = None;
            self.written = i64::MAX;
            self.expires= i64::MIN;
        }
    }
}

#[cfg(test)]
mod expiring_register_tests {
    use super::*;

    #[test]
    fn basic_setting() {
        let mut r = ExpiringFWWRegister::default();
        assert!(r.get().is_none());

        r.set("test".to_string(), 0, 3);
        r.cleanup(0);
        assert_eq!(r.get().unwrap(), "test");

        r.set("newer".to_string(), 1, 3);
        r.cleanup(1);
        assert_eq!(r.get().unwrap(), "test");
    }

    #[test]
    fn basic_expiry() {
        let mut r = ExpiringFWWRegister::default();
        r.set("test".to_string(), 0, 3);
        r.cleanup(0);
        r.cleanup(4);
        assert!(r.get().is_none());

        r.set("newer".to_string(), 5, 8);
        r.cleanup(5);
        assert_eq!(r.get().unwrap(), "newer");
    }

    #[test]
    fn test_merge() {
        let mut a = ExpiringFWWRegister::default();
        a.set("a".to_string(), 0, 3);
        a.cleanup(0);
        let mut b = ExpiringFWWRegister::default();
        b.set("b".to_string(), 1, 3);
        b.cleanup(1);

        let mut na = a.clone();
        let mut nb = b.clone();

        na.merge(&b).unwrap();
        na.cleanup(2);
        assert_eq!(na.get().unwrap(), "a");

        nb.merge(&a).unwrap();
        nb.cleanup(2);
        assert_eq!(na.value, nb.value);
        assert_eq!(na.written, nb.written);
        assert_eq!(na.expires, nb.expires);
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GrowOnlySet<T: Ord>(pub BTreeSet<T>);

impl <T: Ord> Default for GrowOnlySet<T> {
    fn default() -> Self {
        GrowOnlySet(BTreeSet::new())
    }
}

impl<T: Ord> GrowOnlySet<T> {
    pub fn insert(&mut self, v: T) {
        self.0.insert(v);
    }

    pub fn contains(&mut self, v: &T) -> bool {
        self.0.contains(v)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }
}

impl<T: Ord + Clone> Crdt for GrowOnlySet<T> {
    fn merge(&mut self, other: &Self) -> Result<()> {
        self.0.extend(other.0.iter().cloned());
        Ok(())
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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
        for (other_value, (other_written, other_expires)) in &other.0 {
            if let Some((local_written, local_expires)) = self.0.get_mut(other_value) {
                *local_written = (*other_written).min(*local_written);
                *local_expires = (*other_expires).max(*local_expires);
            } else if self.0.len() < self.1 {
                self.0.insert(other_value.clone(), (*other_written, *other_expires));
            } else {
                let mut oldest = None;
                let mut oldest_written = None;
                for (local_value, (local_written, _)) in &self.0 {
                    if local_written > other_written || (local_written == other_written && local_value > other_value) {
                        if let Some(t) = oldest_written {
                            if t < local_written {
                                oldest_written = Some(t);
                                oldest = Some(local_value.clone());
                            }
                        } else {
                            oldest_written = Some(local_written);
                            oldest = Some(local_value.clone());
                        }
                    }
                }
                if let Some(oldest) = oldest {
                    self.0.remove(&oldest);
                    self.0.insert(other_value.clone(), (*other_written, *other_expires));
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

    #[test]
    fn multi_way_merge() {
        let mut a = SizedFWWExpiringSet::new(3);
        a.insert("a".to_string(), 0, 10);
        a.insert("b".to_string(), 1, 10);
        a.insert("c".to_string(), 2, 10);

        let mut b = SizedFWWExpiringSet::new(3);
        b.insert("d".to_string(), 2, 10);

        let mut c = SizedFWWExpiringSet::new(3);
        c.insert("e".to_string(), 2, 10);

        let mut na = a.clone();
        let mut nb = b.clone();
        let mut nc = c.clone();

        na.merge(&b).unwrap();
        na.merge(&c).unwrap();
        na.cleanup(4);
        assert!(na.contains("a"));
        assert!(na.contains("b"));
        assert!(na.contains("c"));
        assert!(!na.contains("d"));
        assert!(!na.contains("e"));

        nb.merge(&a).unwrap();
        nb.merge(&c).unwrap();
        nb.cleanup(4);
        assert_eq!(nb.0, na.0);

        nc.merge(&b).unwrap();
        nc.merge(&a).unwrap();
        nc.cleanup(4);
        assert_eq!(nc.0, na.0);
        assert_eq!(nc.0, nb.0);
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
