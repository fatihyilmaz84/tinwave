//! Rust `QAbstractListModel` serving 100 000 synthetic rows lazily (paged, bounded LRU), plus the
//! "Rust task -> Qt thread" result posting demo. Spike code.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++Qt" {
        include!(<QtCore/QAbstractListModel>);
        /// Base class of the model.
        #[qobject]
        type QAbstractListModel;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qvector.h");
        type QVector_i32 = cxx_qt_lib::QVector<i32>;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        type TrackModel = super::TrackModelRust;

        #[qinvokable]
        #[cxx_override]
        fn data(self: &TrackModel, index: &QModelIndex, role: i32) -> QVariant;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TrackModel) -> QHash_i32_QByteArray;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TrackModel, parent: &QModelIndex) -> i32;

        /// Spawn a 200 ms CPU task on a Rust thread; the result is posted back to the Qt thread.
        #[qinvokable]
        #[cxx_name = "runSlowTask"]
        fn run_slow_task(self: Pin<&mut TrackModel>, ms: i32);

        /// Control: the same task run ON the Qt thread (proves the freeze detector works).
        #[qinvokable]
        #[cxx_name = "runSlowTaskBlocking"]
        fn run_slow_task_blocking(self: &TrackModel, ms: i32);

        #[qinvokable]
        fn stats(self: &TrackModel) -> QString;

        /// Emitted on the Qt thread when a posted task finishes. `latency_us` = worker done -> Qt thread ran.
        #[qsignal]
        #[cxx_name = "taskFinished"]
        fn task_finished(self: Pin<&mut TrackModel>, latency_us: i32);
    }

    unsafe extern "RustQt" {
        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut TrackModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QVector_i32,
        );

        #[inherit]
        fn index(self: &TrackModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
    }

    impl cxx_qt::Threading for TrackModel {}
    impl cxx_qt::Initialize for TrackModel {}
}

use core::pin::Pin;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant, QVector,
};

const TOTAL_ROWS: i32 = 100_000;
const PAGE_SIZE: usize = 200;
const MAX_PAGES: usize = 64; // bounded cache: 64 * 200 = 12 800 rows resident
/// Qt::UserRole = 0x0100
const ROLE_TITLE: i32 = 0x0100;
const ROLE_ARTIST: i32 = 0x0101;
const ROLE_ALBUM: i32 = 0x0102;
const ROLE_DURATION: i32 = 0x0103;
const ROLE_ART_KEY: i32 = 0x0104;
const ROLE_LOADED: i32 = 0x0105;

struct Row {
    title: String,
    artist: String,
    album: String,
    duration: String,
    art_key: String,
}

struct Page {
    rows: Vec<Row>,
    last_used: Cell<u64>,
}

/// Rust-side state of `TrackModel`.
pub struct TrackModelRust {
    pages: HashMap<usize, Page>,
    inflight: RefCell<HashSet<usize>>,
    tick: Cell<u64>,
    request_tx: Option<SyncSender<usize>>,
    fetches: u64,
    evictions: u64,
    dropped_requests: Cell<u64>,
}

impl Default for TrackModelRust {
    fn default() -> Self {
        Self {
            pages: HashMap::new(),
            inflight: RefCell::new(HashSet::new()),
            tick: Cell::new(0),
            request_tx: None,
            fetches: 0,
            evictions: 0,
            dropped_requests: Cell::new(0),
        }
    }
}

/// Stand-in for a DB page query (`SELECT ... LIMIT 200 OFFSET n`): deterministic synthetic rows.
fn fetch_page(page: usize) -> Vec<Row> {
    std::thread::sleep(Duration::from_millis(2)); // simulated DB latency
    (0..PAGE_SIZE)
        .map(|i| {
            let n = page * PAGE_SIZE + i;
            let secs = 90 + (n * 37) % 400;
            Row {
                title: format!("Track {n:06} - {}", ["Dawn", "Static", "Harbor", "Lantern", "Ember"][n % 5]),
                artist: format!("Artist {}", n % 997),
                album: format!("Album {}", n / 12),
                duration: format!("{}:{:02}", secs / 60, secs % 60),
                art_key: format!("{}", n / 12),
            }
        })
        .collect()
}

fn worker(rx: Receiver<usize>, qt: cxx_qt::CxxQtThread<qobject::TrackModel>) {
    while let Ok(page) = rx.recv() {
        let rows = fetch_page(page);
        if qt
            .queue(move |model| model.insert_page(page, rows))
            .is_err()
        {
            break; // object destroyed
        }
    }
}

impl cxx_qt::Initialize for qobject::TrackModel {
    fn initialize(mut self: Pin<&mut Self>) {
        // Bounded request queue (rules: bounded channels only).
        let (tx, rx) = sync_channel::<usize>(256);
        self.as_mut().rust_mut().request_tx = Some(tx);
        let qt = self.qt_thread();
        std::thread::Builder::new()
            .name("page-fetch".into())
            .spawn(move || worker(rx, qt))
            .expect("spawn page worker");
    }
}

impl qobject::TrackModel {
    /// Runs on the Qt thread (posted from the worker).
    fn insert_page(mut self: Pin<&mut Self>, page: usize, rows: Vec<Row>) {
        self.inflight.borrow_mut().remove(&page);
        let tick = self.tick.get() + 1;
        self.tick.set(tick);
        {
            let mut this = self.as_mut().rust_mut();
            this.fetches += 1;
            this.pages.insert(page, Page { rows, last_used: Cell::new(tick) });
            while this.pages.len() > MAX_PAGES {
                let victim = this
                    .pages
                    .iter()
                    .min_by_key(|(_, p)| p.last_used.get())
                    .map(|(k, _)| *k);
                match victim {
                    Some(v) if v != page => {
                        this.pages.remove(&v);
                        this.evictions += 1;
                    }
                    _ => break,
                }
            }
        }
        let first = (page * PAGE_SIZE) as i32;
        let last = first + PAGE_SIZE as i32 - 1;
        let tl = self.index(first, 0, &QModelIndex::default());
        let br = self.index(last, 0, &QModelIndex::default());
        self.data_changed(&tl, &br, &QVector::<i32>::default());
    }

    fn want_page(&self, page: usize) {
        if self.inflight.borrow().contains(&page) {
            return;
        }
        if let Some(tx) = &self.request_tx {
            match tx.try_send(page) {
                Ok(()) => {
                    self.inflight.borrow_mut().insert(page);
                }
                Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                    // Not marked in flight: the next data() call for this page retries.
                    self.dropped_requests.set(self.dropped_requests.get() + 1);
                }
            }
        }
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let row = index.row();
        if !(0..TOTAL_ROWS).contains(&row) {
            return QVariant::default();
        }
        let page = row as usize / PAGE_SIZE;
        let Some(p) = self.pages.get(&page) else {
            // Never block the Qt thread: request the page and show a placeholder.
            self.want_page(page);
            return match role {
                ROLE_LOADED => QVariant::from(&false),
                ROLE_TITLE | ROLE_ARTIST | ROLE_ALBUM | ROLE_DURATION | ROLE_ART_KEY => {
                    QVariant::from(&QString::from(""))
                }
                _ => QVariant::default(),
            };
        };
        let t = self.tick.get() + 1;
        self.tick.set(t);
        p.last_used.set(t);
        let r = &p.rows[row as usize % PAGE_SIZE];
        match role {
            ROLE_TITLE => QVariant::from(&QString::from(r.title.as_str())),
            ROLE_ARTIST => QVariant::from(&QString::from(r.artist.as_str())),
            ROLE_ALBUM => QVariant::from(&QString::from(r.album.as_str())),
            ROLE_DURATION => QVariant::from(&QString::from(r.duration.as_str())),
            ROLE_ART_KEY => QVariant::from(&QString::from(r.art_key.as_str())),
            ROLE_LOADED => QVariant::from(&true),
            _ => QVariant::default(),
        }
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
        roles.insert(ROLE_TITLE, QByteArray::from("title"));
        roles.insert(ROLE_ARTIST, QByteArray::from("artist"));
        roles.insert(ROLE_ALBUM, QByteArray::from("album"));
        roles.insert(ROLE_DURATION, QByteArray::from("duration"));
        roles.insert(ROLE_ART_KEY, QByteArray::from("artKey"));
        roles.insert(ROLE_LOADED, QByteArray::from("loaded"));
        roles
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        TOTAL_ROWS
    }

    pub fn run_slow_task(self: Pin<&mut Self>, ms: i32) {
        let qt = self.qt_thread();
        std::thread::spawn(move || {
            busy(ms);
            let done = Instant::now();
            // Result posting: Rust thread -> Qt thread. Closure runs on the Qt thread.
            let _ = qt.queue(move |model| {
                let us = done.elapsed().as_micros().min(i32::MAX as u128) as i32;
                model.task_finished(us);
            });
        });
    }

    pub fn run_slow_task_blocking(&self, ms: i32) {
        busy(ms);
    }

    pub fn stats(&self) -> QString {
        QString::from(
            format!(
                "pages_resident={} fetches={} evictions={} dropped_requests={}",
                self.pages.len(),
                self.fetches,
                self.evictions,
                self.dropped_requests.get()
            )
            .as_str(),
        )
    }
}

/// CPU-bound busy work for `ms` milliseconds (stand-in for a scan/analysis step).
fn busy(ms: i32) {
    let end = Instant::now() + Duration::from_millis(ms.max(0) as u64);
    let mut x = 0u64;
    while Instant::now() < end {
        for i in 0..10_000u64 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(i);
        }
        std::hint::black_box(x);
    }
}
