//! Order of the image downloads: a bounded number runs, the most wanted one starts first.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio::sync::oneshot;

/// How much an image is wanted; the greater goes first.
///
/// Images of a newer page go before those of an older one, then the artwork of a page before its
/// rows. [`ImagePriority::BACKGROUND`] comes after every page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ImagePriority {
    page: u64,
    artwork: bool,
}

impl ImagePriority {
    /// For images nothing shows yet.
    pub const BACKGROUND: Self = Self {
        page: 0,
        artwork: false,
    };

    /// For the images of page number `page`, a number that grows with every page shown.
    pub const fn of_page(page: u64) -> Self {
        Self {
            page: page.saturating_add(1),
            artwork: false,
        }
    }

    /// The same for the main artwork of the page: its poster, backdrop or banner.
    pub const fn artwork(self) -> Self {
        Self {
            page: self.page,
            artwork: true,
        }
    }
}

type Waiting = BTreeMap<(Reverse<ImagePriority>, u64), oneshot::Sender<Slot>>;

#[derive(Default)]
struct State {
    running: usize,
    background: usize,
    asked: u64,
    waiting: Waiting,
}

/// Download slots of a queue.
#[derive(Clone, Copy)]
pub(crate) struct Slots {
    /// All of them.
    pub(crate) all: usize,
    /// Those only the artwork of a page takes.
    pub(crate) artwork: usize,
    /// Those the images nothing shows yet may take.
    pub(crate) background: usize,
}

/// Hands out download slots.
pub(crate) struct FetchQueue {
    slots: Slots,
    state: Mutex<State>,
}

/// A download slot, given back when dropped.
pub(crate) struct Slot {
    queue: Arc<FetchQueue>,
    background: bool,
}

impl Drop for Slot {
    fn drop(&mut self) {
        {
            let mut state = self.queue.lock();
            state.running -= 1;
            state.background -= usize::from(self.background);
        }
        self.queue.start_waiting();
    }
}

impl FetchQueue {
    /// A queue handing out `slots`.
    pub(crate) fn new(slots: Slots) -> Arc<Self> {
        Arc::new(Self {
            slots,
            state: Mutex::default(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Waits for a slot. Dropping the future leaves the queue.
    pub(crate) async fn slot(self: &Arc<Self>, priority: ImagePriority) -> Option<Slot> {
        let (sender, receiver) = oneshot::channel();
        {
            let mut state = self.lock();
            state.asked += 1;
            let place = (Reverse(priority), state.asked);
            state.waiting.insert(place, sender);
        }
        self.start_waiting();
        receiver.await.ok()
    }

    /// Gives the free slots to the most wanted downloads waiting.
    fn start_waiting(self: &Arc<Self>) {
        while let Some((sender, background)) = self.next() {
            // A slot refused because its download was dropped comes back on its own.
            let _ = sender.send(Slot {
                queue: Arc::clone(self),
                background,
            });
        }
    }

    fn next(&self) -> Option<(oneshot::Sender<Slot>, bool)> {
        let mut state = self.lock();
        state.waiting.retain(|_, sender| !sender.is_closed());
        let (&place, _) = state
            .waiting
            .iter()
            .find(|((Reverse(priority), _), _)| self.has_slot(&state, *priority))?;
        let sender = state.waiting.remove(&place)?;
        let background = place.0.0 == ImagePriority::BACKGROUND;
        state.running += 1;
        state.background += usize::from(background);
        Some((sender, background))
    }

    fn has_slot(&self, state: &State, priority: ImagePriority) -> bool {
        let Slots {
            all,
            artwork,
            background,
        } = self.slots;
        if priority.artwork {
            return state.running < all;
        }
        state.running < all - artwork
            && (priority != ImagePriority::BACKGROUND || state.background < background)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;
    use tokio::task::{JoinHandle, yield_now};

    type Started = mpsc::UnboundedReceiver<(&'static str, Slot)>;

    struct Bench {
        queue: Arc<FetchQueue>,
        sender: mpsc::UnboundedSender<(&'static str, Slot)>,
        started: Started,
    }

    impl Bench {
        fn new(all: usize, artwork: usize, background: usize) -> Self {
            let (sender, started) = mpsc::unbounded_channel();
            Self {
                queue: FetchQueue::new(Slots {
                    all,
                    artwork,
                    background,
                }),
                sender,
                started,
            }
        }

        async fn ask(&self, name: &'static str, priority: ImagePriority) -> JoinHandle<()> {
            let (queue, sender) = (Arc::clone(&self.queue), self.sender.clone());
            let task = tokio::spawn(async move {
                if let Some(slot) = queue.slot(priority).await {
                    let _ = sender.send((name, slot));
                }
            });
            settle().await;
            task
        }

        async fn started(&mut self) -> Vec<(&'static str, Slot)> {
            settle().await;
            let mut all = Vec::new();
            while let Ok(one) = self.started.try_recv() {
                all.push(one);
            }
            all
        }
    }

    async fn settle() {
        for _ in 0..5 {
            yield_now().await;
        }
    }

    fn names(slots: &[(&'static str, Slot)]) -> Vec<&'static str> {
        slots.iter().map(|(name, _)| *name).collect()
    }

    #[tokio::test]
    async fn the_limit_holds_and_the_first_asked_goes_first() {
        let page = ImagePriority::of_page(3);
        let mut bench = Bench::new(2, 0, 1);
        for name in ["a", "b", "c", "d"] {
            bench.ask(name, page).await;
        }
        let first = bench.started().await;
        assert_eq!(names(&first), ["a", "b"]);
        drop(first);
        assert_eq!(names(&bench.started().await), ["c", "d"]);
    }

    #[tokio::test]
    async fn a_newer_page_and_the_artwork_of_a_page_go_first() {
        let mut bench = Bench::new(1, 0, 1);
        bench.ask("running", ImagePriority::of_page(1)).await;
        let mut running = bench.started().await;
        bench.ask("old", ImagePriority::of_page(1)).await;
        bench.ask("behind", ImagePriority::BACKGROUND).await;
        bench.ask("photo", ImagePriority::of_page(2)).await;
        bench
            .ask("poster", ImagePriority::of_page(2).artwork())
            .await;
        for expected in ["poster", "photo", "old", "behind"] {
            drop(running);
            running = bench.started().await;
            assert_eq!(names(&running), [expected]);
        }
    }

    #[tokio::test]
    async fn rows_leave_a_slot_to_the_artwork_of_a_page() {
        let page = ImagePriority::of_page(1);
        let mut bench = Bench::new(3, 1, 1);
        for name in ["photo 1", "photo 2", "photo 3"] {
            bench.ask(name, page).await;
        }
        let photos = bench.started().await;
        assert_eq!(names(&photos), ["photo 1", "photo 2"]);
        bench.ask("poster", page.artwork()).await;
        assert_eq!(names(&bench.started().await), ["poster"]);
    }

    #[tokio::test]
    async fn the_background_never_takes_every_slot() {
        let mut bench = Bench::new(3, 0, 1);
        bench.ask("behind 1", ImagePriority::BACKGROUND).await;
        bench.ask("behind 2", ImagePriority::BACKGROUND).await;
        let behind = bench.started().await;
        assert_eq!(names(&behind), ["behind 1"]);
        bench.ask("shown", ImagePriority::of_page(1)).await;
        let shown = bench.started().await;
        assert_eq!(names(&shown), ["shown"]);
        drop(behind);
        assert_eq!(names(&bench.started().await), ["behind 2"]);
    }

    #[tokio::test]
    async fn a_dropped_download_leaves_the_queue_and_gives_its_slot_back() {
        let page = ImagePriority::of_page(1);
        let mut bench = Bench::new(1, 0, 1);
        bench.ask("running", page).await;
        let running = bench.started().await;
        let dropped = bench.ask("dropped", page).await;
        bench.ask("kept", page).await;
        dropped.abort();
        settle().await;
        drop(running);
        assert_eq!(names(&bench.started().await), ["kept"]);
    }

    #[tokio::test]
    async fn a_slot_granted_to_a_download_dropped_before_using_it_is_not_lost() {
        let queue = Bench::new(1, 0, 1).queue;
        let mut granted = Box::pin(queue.slot(ImagePriority::of_page(1)));
        let waker = std::task::Waker::noop();
        let running = match granted
            .as_mut()
            .poll(&mut std::task::Context::from_waker(waker))
        {
            std::task::Poll::Ready(slot) => slot,
            std::task::Poll::Pending => None,
        };
        assert!(running.is_some());
        let mut waiting = Box::pin(queue.slot(ImagePriority::of_page(1)));
        assert!(
            waiting
                .as_mut()
                .poll(&mut std::task::Context::from_waker(waker))
                .is_pending()
        );
        drop(running);
        drop(waiting);
        assert!(queue.slot(ImagePriority::of_page(1)).await.is_some());
        assert_eq!(queue.lock().running, 0);
    }
}
