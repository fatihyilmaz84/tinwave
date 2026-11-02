//! Async image provider: a ~80 line C++ shim (`cpp/image_provider.cpp`, QQuickAsyncImageProvider +
//! QThreadPool) calls this Rust function for the pixels. Rust owns the bounded LRU thumbnail cache.
//! (CXX-Qt 0.10 binds `QQmlEngine::addImageProvider`, but cannot subclass the non-QObject
//! `QQuickAsyncImageProvider`/`QQuickImageResponse` itself.)

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qqmlimageproviderbase.h");
        type QQmlImageProviderBase = cxx_qt_lib::QQmlImageProviderBase;

        include!("image_provider.h");
        /// Creates the C++ provider (ownership goes to the QML engine).
        fn tw_make_image_provider() -> *mut QQmlImageProviderBase;
    }
}

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

pub use qobject::tw_make_image_provider;

const CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
struct Cache {
    map: HashMap<(u32, i32, i32), Arc<Vec<u8>>>,
    order: VecDeque<(u32, i32, i32)>,
    bytes: usize,
    sizes: HashMap<(i32, i32), u64>,
}

static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
static REQUESTS: AtomicU64 = AtomicU64::new(0);
static HITS: AtomicU64 = AtomicU64::new(0);

fn cache() -> &'static Mutex<Cache> {
    CACHE.get_or_init(|| Mutex::new(Cache::default()))
}

/// Synthetic "decode": cover-like gradient with a ring, BGRA (QImage::Format_RGB32 on little endian).
fn render(key: u32, w: i32, h: i32) -> Vec<u8> {
    std::thread::sleep(Duration::from_millis(3)); // stand-in for file read + JPEG decode + downscale
    let hue = (key.wrapping_mul(47) % 360) as f32;
    let (w_us, h_us) = (w as usize, h as usize);
    let mut out = vec![0u8; w_us * h_us * 4];
    for y in 0..h_us {
        for x in 0..w_us {
            let fx = x as f32 / w as f32;
            let fy = y as f32 / h as f32;
            let d = ((fx - 0.5).powi(2) + (fy - 0.5).powi(2)).sqrt();
            let ring = if (0.18..0.26).contains(&d) { 0.35 } else { 0.0 };
            let l = 0.35 + 0.35 * (fx + fy) / 2.0 + ring;
            let (r, g, b) = hsl(hue, 0.55, l.min(0.95));
            let i = (y * w_us + x) * 4;
            out[i] = b;
            out[i + 1] = g;
            out[i + 2] = r;
            out[i + 3] = 255;
        }
    }
    out
}

fn hsl(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8)
}

/// Called from a Qt worker thread (QThreadPool). Fills `out` (w*h*4 bytes). Returns 1 on success.
///
/// # Safety
/// `id` must point to `id_len` readable bytes and `out` to `w*h*4` writable bytes.
#[no_mangle]
pub unsafe extern "C" fn tw_render_thumbnail(
    id: *const u8,
    id_len: usize,
    w: i32,
    h: i32,
    out: *mut u8,
) -> i32 {
    // SAFETY: guaranteed by the caller contract above.
    let id = unsafe { std::slice::from_raw_parts(id, id_len) };
    let Some(key) = std::str::from_utf8(id).ok().and_then(|s| s.parse::<u32>().ok()) else {
        return 0;
    };
    if w <= 0 || h <= 0 || w > 1024 || h > 1024 {
        return 0;
    }
    REQUESTS.fetch_add(1, Relaxed);
    let k = (key, w, h);
    let hit = {
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        *c.sizes.entry((w, h)).or_default() += 1;
        let hit = c.map.get(&k).cloned();
        if hit.is_some() {
            c.order.retain(|x| *x != k);
            c.order.push_back(k);
        }
        hit
    };
    let px = if let Some(p) = hit {
        HITS.fetch_add(1, Relaxed);
        p
    } else {
        let p = Arc::new(render(key, w, h));
        let mut c = cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = c.map.insert(k, Arc::clone(&p)) {
            // Two workers raced on the same key: replace, do not double count.
            c.bytes -= old.len();
            c.order.retain(|x| *x != k);
        }
        c.bytes += p.len();
        c.order.push_back(k);
        while c.bytes > CACHE_MAX_BYTES {
            let Some(old) = c.order.pop_front() else { break };
            if let Some(v) = c.map.remove(&old) {
                c.bytes -= v.len();
            }
        }
        p
    };
    // SAFETY: `out` has room for w*h*4 bytes (caller contract) and `px` has exactly that length.
    unsafe { std::ptr::copy_nonoverlapping(px.as_ptr(), out, px.len()) };
    1
}

pub fn stats_string() -> String {
    let c = cache().lock().unwrap_or_else(|e| e.into_inner());
    let mut sizes: Vec<_> = c.sizes.iter().map(|((w, h), n)| format!("{w}x{h}:{n}")).collect();
    sizes.sort();
    format!(
        "image requests={} cache_hits={} cached_entries={} cached_mb={:.1} sizes=[{}]",
        REQUESTS.load(Relaxed),
        HITS.load(Relaxed),
        c.map.len(),
        c.bytes as f64 / 1048576.0,
        sizes.join(" ")
    )
}
