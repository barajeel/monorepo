//! A memory efficient index structure for a journal that maps keys to locations.

use crate::index::Translator;
use commonware_runtime::Metrics;
use prometheus_client::metrics::counter::Counter;
use std::collections::{hash_map::Entry, HashMap};

/// Each key is mapped to a `Record` that contains a potential location of the requested value in
/// the journal, and an optional link to another potential location to allow for collisions.
///
/// The `Index` uses a `Translator` to transform keys into a compressed representation, resulting in
/// non-negligible probability of collisions. Collision resolution is the responsibility of the
/// user.
struct Record {
    location: u64,

    next: Option<Box<Record>>,
}

pub struct LocationIterator<'a> {
    next: Option<&'a Record>,
}

impl LocationIterator<'_> {
    /// Create a `LocationIterator` that returns no items.
    fn empty() -> Self {
        LocationIterator { next: None }
    }
}

impl Iterator for LocationIterator<'_> {
    type Item = u64;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next {
            Some(next) => {
                let loc = next.location;
                self.next = next.next.as_deref();
                Some(loc)
            }
            None => None,
        }
    }
}

impl Record {
    fn iter(&self) -> LocationIterator {
        LocationIterator { next: Some(self) }
    }
}

pub struct Index<T: Translator, E: Metrics> {
    translator: T,

    // We store the first location associated with a key within the HashMap entry itself to
    // significantly reduce the number of random reads we need to do on the heap.
    map: HashMap<T::Key, Record>,

    collisions: Counter,
    keys_pruned: Counter,

    _metrics: std::marker::PhantomData<E>,
}

impl<T: Translator, E: Metrics> Index<T, E> {
    /// Create a new index.
    pub fn init(context: E, translator: T) -> Self {
        let s = Self {
            translator,
            map: HashMap::new(),
            collisions: Counter::default(),
            keys_pruned: Counter::default(),
            _metrics: std::marker::PhantomData,
        };
        context.register(
            "keys_pruned",
            "Number of keys pruned",
            s.keys_pruned.clone(),
        );

        s
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn collisions(&self) -> u64 {
        self.collisions.get()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Insert a new record into the index.
    pub fn insert(&mut self, key: &[u8], location: u64) {
        let translated_key = self.translator.transform(key);

        match self.map.entry(translated_key) {
            Entry::Occupied(entry) => {
                let entry: &mut Record = entry.into_mut();
                entry.next = Some(Box::new(Record {
                    location,
                    next: entry.next.take(),
                }));
                self.collisions.inc();
            }
            Entry::Vacant(entry) => {
                entry.insert(Record {
                    location,
                    next: None,
                });
            }
        };
    }

    pub fn get(&self, key: &[u8]) -> LocationIterator {
        let translated_key = self.translator.transform(key);
        match self.map.get(&translated_key) {
            Some(head) => head.iter(),
            None => LocationIterator::empty(),
        }
    }

    /// Remove locations associated with the key that match the `prune` predicate.
    pub fn remove(&mut self, key: &[u8], prune: impl Fn(u64) -> bool) {
        let translated_key = self.translator.transform(key);
        let head = match self.map.get_mut(&translated_key) {
            Some(head) => head,
            None => return,
        };

        // Advance the head of the linked list to the first entry that will be retained, if any.
        loop {
            if !prune(head.location) {
                break;
            }

            self.keys_pruned.inc();

            match head.next {
                Some(ref mut next) => {
                    head.location = next.location;
                    head.next = next.next.take();
                }
                None => {
                    // No retained entries, so remove the key from the map.
                    self.map.remove(&translated_key);
                    return;
                }
            }
        }

        // Prune the remainder of the list.
        let mut cursor = head;
        while let Some(location) = cursor.next.as_ref().map(|next| next.location) {
            if prune(location) {
                cursor.next = cursor.next.as_mut().unwrap().next.take();
                self.keys_pruned.inc();
                continue;
            }
            cursor = cursor.next.as_mut().unwrap();
        }
    }
}
