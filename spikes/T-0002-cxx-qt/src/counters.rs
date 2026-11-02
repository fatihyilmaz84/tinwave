//! UI-originated counters (Impression / ArtworkHover / PageDwell): QML forwards raw cheap events,
//! Rust debounces on its own thread (qml-ui.md: "Debounce in Rust, not QML"). Spike code.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type Counters = super::CountersRust;

        /// Pointer entered/left row `index` (HoverHandler). Debounced: >= 800 ms => ArtworkHover.
        #[qinvokable]
        fn hover(self: &Counters, index: i32, entered: bool);

        /// Visible range changed (every frame while scrolling). Settled >= 500 ms => Impressions.
        #[qinvokable]
        fn viewport(self: &Counters, first: i32, last: i32);

        /// Page became visible/hidden; dwell measured on leave => PageDwell.
        #[qinvokable]
        #[cxx_name = "pageVisible"]
        fn page_visible(self: &Counters, name: &QString, visible: bool);

        #[qinvokable]
        fn snapshot(self: &Counters) -> QString;
    }

    impl cxx_qt::Initialize for Counters {}
}

use core::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cxx_qt_lib::QString;

const HOVER_MIN: Duration = Duration::from_millis(800);
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
struct Totals {
    viewport_events: AtomicU64,
    hover_events: AtomicU64,
    impression_batches: AtomicU64,
    impressions: AtomicU64,
    artwork_hovers: AtomicU64,
    page_dwells: AtomicU64,
    last_dwell_ms: AtomicU64,
}

enum Msg {
    Hover(i32, bool),
    Viewport(i32, i32),
}

/// Rust state of `Counters`.
pub struct CountersRust {
    tx: Option<Sender<Msg>>,
    totals: Arc<Totals>,
    page_since: Mutex<Option<(String, Instant)>>,
}

impl Default for CountersRust {
    fn default() -> Self {
        Self {
            tx: None,
            totals: Arc::new(Totals::default()),
            page_since: Mutex::new(None),
        }
    }
}

/// Debounce thread: blocks on the channel (zero cost while idle), wakes at the next deadline.
fn debouncer(rx: Receiver<Msg>, totals: Arc<Totals>) {
    let mut viewport: Option<(Instant, i32, i32)> = None;
    let mut hover: Option<(Instant, i32)> = None;
    loop {
        let next = [viewport.map(|v| v.0), hover.map(|h| h.0)].into_iter().flatten().min();
        let msg = match next {
            Some(d) => rx.recv_timeout(d.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        let now = Instant::now();
        match msg {
            Ok(Msg::Viewport(a, b)) => viewport = Some((now + SETTLE, a, b)),
            Ok(Msg::Hover(i, true)) => hover = Some((now + HOVER_MIN, i)),
            Ok(Msg::Hover(_, false)) => hover = None,
            Err(RecvTimeoutError::Timeout) => {
                if let Some((d, a, b)) = viewport {
                    if d <= now {
                        totals.impression_batches.fetch_add(1, Relaxed);
                        totals.impressions.fetch_add((b - a + 1).max(0) as u64, Relaxed);
                        viewport = None;
                    }
                }
                if let Some((d, _)) = hover {
                    if d <= now {
                        totals.artwork_hovers.fetch_add(1, Relaxed);
                        hover = None;
                    }
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

impl cxx_qt::Initialize for qobject::Counters {
    fn initialize(mut self: Pin<&mut Self>) {
        use cxx_qt::CxxQtType;
        let (tx, rx) = channel();
        let totals = Arc::clone(&self.totals);
        self.as_mut().rust_mut().tx = Some(tx);
        std::thread::Builder::new()
            .name("ui-debounce".into())
            .spawn(move || debouncer(rx, totals))
            .expect("spawn debounce thread");
    }
}

impl qobject::Counters {
    pub fn hover(&self, index: i32, entered: bool) {
        self.totals.hover_events.fetch_add(1, Relaxed);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Hover(index, entered));
        }
    }

    pub fn viewport(&self, first: i32, last: i32) {
        self.totals.viewport_events.fetch_add(1, Relaxed);
        if let Some(tx) = &self.tx {
            let _ = tx.send(Msg::Viewport(first, last));
        }
    }

    pub fn page_visible(&self, name: &QString, visible: bool) {
        let mut slot = self.page_since.lock().unwrap_or_else(|e| e.into_inner());
        if visible {
            *slot = Some((name.to_string(), Instant::now()));
        } else if let Some((_, t)) = slot.take() {
            self.totals.page_dwells.fetch_add(1, Relaxed);
            self.totals.last_dwell_ms.store(t.elapsed().as_millis() as u64, Relaxed);
        }
    }

    pub fn snapshot(&self) -> QString {
        let t = &self.totals;
        QString::from(
            format!(
                "viewport_events={} hover_events={} impression_batches={} impressions={} artwork_hovers={} page_dwells={} last_dwell_ms={}",
                t.viewport_events.load(Relaxed),
                t.hover_events.load(Relaxed),
                t.impression_batches.load(Relaxed),
                t.impressions.load(Relaxed),
                t.artwork_hovers.load(Relaxed),
                t.page_dwells.load(Relaxed),
                t.last_dwell_ms.load(Relaxed),
            )
            .as_str(),
        )
    }
}
