//! Native parse benchmark: **turbo-xlsx-core** vs **calamine**.
//!
//! Both read the *same* DEFLATEd `.xlsx` bytes into a fully-materialized value
//! grid, N times; we report the median wall-clock and the ratio. The fixture is
//! built by turbo's own writer (STORED) and then re-zipped with DEFLATE, so both
//! readers run their real inflate + zip-walk + XML parse path on Excel-style
//! compressed input. See `hotspot.rs` for the per-phase profiler.
//!
//! Run:  cargo run --release --manifest-path benches/parse-native/Cargo.toml

mod common;

use common::{build_fixture, deflate_xlsx, median_ms, read_calamine, read_turbo};

fn main() {
    println!("native parse: turbo-xlsx-core vs calamine (read DEFLATEd .xlsx -> value grid)\n");
    for &rows in &[1_000usize, 50_000] {
        let bytes = deflate_xlsx(&build_fixture(rows));
        let iters = if rows >= 50_000 { 20 } else { 100 };

        // Warm up + a correctness cross-check: both readers must see the same
        // number of cells, or the comparison is meaningless.
        let (t_cells, c_cells) = (read_turbo(&bytes), read_calamine(&bytes));
        assert_eq!(
            t_cells, c_cells,
            "turbo and calamine disagree on cell count"
        );

        let turbo = median_ms(iters, || {
            let _ = read_turbo(&bytes);
        });
        let calamine = median_ms(iters, || {
            let _ = read_calamine(&bytes);
        });
        println!(
            "{rows:>6} rows ({kb}KB deflated, {cells} cells):  \
             turbo {turbo:6.2}ms   calamine {calamine:6.2}ms   -> {ratio:.2}x",
            kb = bytes.len() / 1024,
            cells = t_cells,
            ratio = calamine / turbo,
        );
    }
}
