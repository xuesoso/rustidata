//! Time a full out-of-core sort and frequency table.
//!
//!     cargo run --release -p rustidata-core --example sort_bench -- FILE COL

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use rustidata_core::column::Column;
use rustidata_core::jobs::CancelToken;
use rustidata_core::sheet::Sheet;
use rustidata_core::store::{MmapTextStore, RowSource};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: sort_bench FILE [COL]");
    let col: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let store = MmapTextStore::open(std::path::Path::new(&path), None).expect("open");
    store.index_remainder(&AtomicBool::new(false), || {});
    let columns: Vec<Column> = store
        .headers
        .iter()
        .enumerate()
        .map(|(i, h)| Column::source(h.clone(), i))
        .collect();
    let nrows = store.len();
    let sheet = Sheet::new("bench", Arc::new(store), columns);

    let t = Instant::now();
    let perm = rustidata_core::sort::sort_permutation(&sheet, &[(col, true)], &CancelToken::new())
        .expect("sort");
    println!(
        "sort {} rows by col {}: {:?} (first rowid {})",
        nrows,
        col,
        t.elapsed(),
        perm[0]
    );

    let t = Instant::now();
    let freq =
        rustidata_core::transform::freq::freq_table(&sheet, &[col.min(2)], &[], &CancelToken::new())
            .expect("freq");
    println!(
        "freq {} rows on col {}: {:?} ({} groups)",
        nrows,
        col.min(2),
        t.elapsed(),
        freq.nrows()
    );
}
