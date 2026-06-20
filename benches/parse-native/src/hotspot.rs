//! Parse **hotspot profiler**: split turbo's parse into its two phases and time
//! each, so optimization effort goes where the time actually is.
//!
//!   * Phase A — unzip + inflate (`parse::read_zip`): DEFLATE-decompress every
//!     OPC part + walk the zip central directory.
//!   * Phase B — XML + value-build (full `parse` minus phase A): tokenize the
//!     sheet/shared-strings XML and materialize typed cell values.
//!
//! calamine is shown alongside as the target. Phase A is exposed only under the
//! core's `bench-internals` feature (this crate enables it).
//!
//! Run:  cargo run --release --bin parse-hotspot \
//!         --manifest-path benches/parse-native/Cargo.toml

mod common;

use common::{build_fixture, deflate_xlsx, median_ms, read_calamine, read_turbo};
use turbo_xlsx_core::parse::read_zip;

/// Phase A in isolation: unzip + inflate every part, summing inflated bytes so
/// the work can't be optimized away.
fn unzip_phase(bytes: &[u8]) -> usize {
    read_zip(bytes)
        .map(|entries| entries.iter().map(|e| e.data.len()).sum())
        .unwrap_or(0)
}

fn main() {
    println!("parse hotspot: turbo-xlsx-core phases vs calamine (DEFLATEd .xlsx)\n");
    for &rows in &[1_000usize, 50_000] {
        let bytes = deflate_xlsx(&build_fixture(rows));
        let iters = if rows >= 50_000 { 20 } else { 100 };

        // Warm caches/branch predictors before timing.
        let _ = (
            unzip_phase(&bytes),
            read_turbo(&bytes),
            read_calamine(&bytes),
        );

        let unzip = median_ms(iters, || {
            let _ = unzip_phase(&bytes);
        });
        let full = median_ms(iters, || {
            let _ = read_turbo(&bytes);
        });
        let calamine = median_ms(iters, || {
            let _ = read_calamine(&bytes);
        });
        let xml = (full - unzip).max(0.0);

        println!("{rows} rows ({}KB deflated):", bytes.len() / 1024);
        println!(
            "  A unzip+inflate : {unzip:7.2}ms  ({:4.1}% of turbo)",
            pct(unzip, full)
        );
        println!(
            "  B xml+value     : {xml:7.2}ms  ({:4.1}% of turbo)",
            pct(xml, full)
        );
        println!("  ─ turbo total   : {full:7.2}ms");
        println!(
            "    calamine total: {calamine:7.2}ms   (turbo is {:.2}x calamine)",
            calamine / full
        );
        println!();
    }
}

/// `part` as a percentage of `whole` (0 when whole is ~0).
fn pct(part: f64, whole: f64) -> f64 {
    if whole <= 0.0 {
        0.0
    } else {
        part / whole * 100.0
    }
}
