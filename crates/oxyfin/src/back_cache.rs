//! Detail pages left for another page of the same kind, kept so that Back shows them again at
//! once, without asking the server.
//!
//! A page is kept for its place in the stack: its depth and its screen. It is valid only while
//! the stack still holds that screen at that depth, and only for the generation that loaded it.

use crate::screens::collection::ParkedCollection;
use crate::screens::movie::ParkedMovie;
use crate::screens::person::ParkedPerson;
use crate::screens::series::ParkedSeries;
use crate::state::{GenerationId, Screen};

/// The content of a detail page kept for Back.
pub(crate) enum Parked {
    /// A movie page.
    Movie(Box<ParkedMovie>),
    /// A series page.
    Series(Box<ParkedSeries>),
    /// A person page.
    Person(Box<ParkedPerson>),
    /// A collection page.
    Collection(Box<ParkedCollection>),
}

/// Pages kept at most; the oldest one is forgotten first.
pub(crate) const KEPT_PAGES: usize = 8;

struct Entry<T> {
    depth: usize,
    screen: Screen,
    generation: GenerationId,
    page: T,
}

/// The pages kept below the current screen.
pub(crate) struct BackCache<T> {
    entries: Vec<Entry<T>>,
}

impl<T> Default for BackCache<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T> BackCache<T> {
    /// Keeps `page`, the content of `screen` at `depth` in the stack, loaded by `generation`.
    pub(crate) fn park(&mut self, depth: usize, screen: Screen, generation: GenerationId, page: T) {
        self.entries.retain(|entry| entry.depth != depth);
        self.entries.push(Entry {
            depth,
            screen,
            generation,
            page,
        });
        while self.entries.len() > KEPT_PAGES {
            self.entries.remove(0);
        }
    }

    /// Takes the page kept for `screen` at `depth`, when `generation` loaded it.
    pub(crate) fn take(
        &mut self,
        depth: usize,
        screen: &Screen,
        generation: GenerationId,
    ) -> Option<T> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.depth == depth && entry.screen == *screen)?;
        let entry = self.entries.remove(index);
        (entry.generation == generation).then_some(entry.page)
    }

    /// Forgets the pages whose screen left its place in `stack`.
    pub(crate) fn prune(&mut self, stack: &[Screen]) {
        self.entries
            .retain(|entry| stack.get(entry.depth) == Some(&entry.screen));
    }

    /// Forgets every page.
    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// Number of pages kept.
    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::LibraryTab;

    const G1: GenerationId = GenerationId(1);

    fn movie(id: &str) -> Screen {
        Screen::Movie(id.into())
    }

    #[test]
    fn a_page_comes_back_once_at_its_place() {
        let mut cache = BackCache::default();
        cache.park(1, movie("a"), G1, "a");
        assert_eq!(cache.take(2, &movie("a"), G1), None);
        assert_eq!(cache.take(1, &movie("b"), G1), None);
        assert_eq!(cache.take(1, &movie("a"), G1), Some("a"));
        assert_eq!(cache.take(1, &movie("a"), G1), None);
    }

    #[test]
    fn a_page_of_another_generation_is_refused() {
        let mut cache = BackCache::default();
        cache.park(1, movie("a"), G1, "a");
        assert_eq!(cache.take(1, &movie("a"), GenerationId(2)), None);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn pages_off_the_stack_are_pruned() {
        let mut cache = BackCache::default();
        cache.park(1, movie("a"), G1, "a");
        cache.park(2, Screen::Person("p".into()), G1, "p");
        let stack = [Screen::Library(LibraryTab::Movies), movie("a")];
        cache.prune(&stack);
        assert_eq!(cache.len(), 1);
        cache.prune(&[Screen::Library(LibraryTab::Movies), movie("b")]);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn a_new_page_at_a_depth_replaces_the_old_one_and_the_oldest_goes_first() {
        let mut cache = BackCache::default();
        cache.park(1, movie("a"), G1, "a");
        cache.park(1, movie("b"), G1, "b");
        assert_eq!(cache.len(), 1);
        for depth in 2..=KEPT_PAGES + 1 {
            cache.park(depth, movie("c"), G1, "c");
        }
        assert_eq!(cache.len(), KEPT_PAGES);
        assert_eq!(cache.take(1, &movie("b"), G1), None);
        assert_eq!(cache.take(2, &movie("c"), G1), Some("c"));
    }
}
