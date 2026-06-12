//! Time the out-of-core open and full index of a text file.
//!
//!     cargo run --release -p rustidata-core --example index_bench -- FILE

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use rustidata_core::store::{MmapTextStore, RowSource};

fn main() {
    let path = std::env::args().nth(1).expect("usage: index_bench FILE");
    let path = std::path::Path::new(&path);

    let t0 = Instant::now();
    let store = MmapTextStore::open(path, None).expect("open");
    let t_open = t0.elapsed();
    println!(
        "open + sniff + first-screen index: {:?} ({} rows visible, delimiter={:?})",
        t_open,
        store.len(),
        store.dialect.delimiter as char,
    );

    let t1 = Instant::now();
    store.index_remainder(&AtomicBool::new(false), || {});
    let t_index = t1.elapsed();
    let rows = store.len();
    println!(
        "full index: {:?} ({} rows, {:.0} rows/s, {:.1} MB/s)",
        t_index,
        rows,
        rows as f64 / t_index.as_secs_f64(),
        store.file_len() as f64 / 1e6 / t_index.as_secs_f64(),
    );

    // random access sample
    let t2 = Instant::now();
    let mut checksum = 0u64;
    for i in (0..rows).step_by((rows as usize / 10000).max(1)) {
        if let Some(s) = store.raw(i, 0).as_str() {
            checksum = checksum.wrapping_add(s.len() as u64);
        }
    }
    println!("10k random row parses: {:?} (checksum {})", t2.elapsed(), checksum);
}
