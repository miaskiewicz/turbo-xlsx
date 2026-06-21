//! The row-by-row streaming writer for large sheets.
//!
//! Payroll exports can be tens of thousands of rows. Rather than build the whole
//! typed model in memory, the caller starts a sheet, pushes rows as it pages
//! query results, ends the sheet, and finishes the package. No [`Row`] is
//! retained: each is serialized to XML on arrival and the typed value dropped.
//! The shared style table and the per-sheet XML buffer are the only state that
//! grows, so per-row work stays O(1) in the number of rows already written.

use crate::error::{Diagnostics, Result};
use crate::model::{Row, Sheet, WriteOptions};
use crate::package::{finish_package, WriteResult, DEFAULT_LOCALE};
use crate::style::StyleTable;
use crate::worksheet::{row_xml, sheet_prefix, sheet_suffix, ColCache, ColumnData};

/// A sheet currently being streamed: its metadata plus the accumulating row XML
/// and the running dimensions needed to size `<dimension>` at close.
struct OpenSheet {
    meta: Sheet,
    body: String,
    rows: u32,
    max_cols: u32,
    /// Per-column number-format cache, persisted across the sheet's rows.
    cache: ColCache,
}

/// A streaming workbook writer. Build it with [`WorkbookWriter::new`], then
/// `start_sheet` / `write_row` / `end_sheet` per sheet and `finish` once.
pub struct WorkbookWriter {
    locale: String,
    opts: WriteOptions,
    table: StyleTable,
    diags: Diagnostics,
    metas: Vec<Sheet>,
    sheets: Vec<String>,
    open: Option<OpenSheet>,
}

impl WorkbookWriter {
    /// Create a writer. `locale` defaults to `en-US` when `None`; `opts` carries
    /// the document metadata for the finished package.
    pub fn new(locale: Option<String>, opts: WriteOptions) -> Self {
        WorkbookWriter {
            locale: locale.unwrap_or_else(|| DEFAULT_LOCALE.to_string()),
            opts,
            table: StyleTable::new(),
            diags: Diagnostics::default(),
            metas: Vec::new(),
            sheets: Vec::new(),
            open: None,
        }
    }

    /// Begin a new sheet from its metadata (name, columns, merges, freeze,
    /// outline). Its `rows` are ignored — stream them with [`Self::write_row`].
    /// Auto-closes a previously open sheet, propagating its seal error (e.g. a
    /// bad merge range) rather than swallowing it.
    pub fn start_sheet(&mut self, mut meta: Sheet) -> Result<()> {
        self.close_open()?;
        meta.rows = Vec::new();
        self.open = Some(OpenSheet {
            meta,
            body: String::new(),
            rows: 0,
            max_cols: 0,
            cache: ColCache::new(),
        });
        Ok(())
    }

    /// Stream one row into the open sheet. No-op (returns `Ok`) if no sheet is
    /// open, so a stray row can never corrupt the package.
    pub fn write_row(&mut self, row: &Row) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            return Ok(());
        };
        row_xml(
            &mut open.body,
            &open.meta,
            row,
            open.rows,
            &self.locale,
            &mut self.table,
            &mut self.diags,
            &mut open.cache,
        )?;
        open.rows += 1;
        open.max_cols = open.max_cols.max(row.cells.len() as u32);
        Ok(())
    }

    /// Stream a block of columns (the columnar fast path) into the open sheet:
    /// each column carries a fixed type/format and a contiguous value vector,
    /// emitted row-major with the format interned once per column. No-op when no
    /// sheet is open.
    pub fn write_columns(&mut self, columns: Vec<ColumnData>) -> Result<()> {
        let Some(open) = self.open.as_mut() else {
            return Ok(());
        };
        let emitted = crate::worksheet::write_columns(
            &mut open.body,
            &columns,
            open.rows,
            &mut self.table,
            &self.locale,
        );
        open.rows += emitted;
        open.max_cols = open.max_cols.max(columns.len() as u32);
        Ok(())
    }

    /// Close the open sheet, sealing its worksheet XML. Idempotent.
    pub fn end_sheet(&mut self) -> Result<()> {
        self.close_open()
    }

    /// Finish every sheet and ZIP the package, returning the bytes + diagnostics.
    pub fn finish(mut self) -> Result<WriteResult> {
        self.close_open()?;
        let styles = self.table.to_xml();
        let xlsx = finish_package(&self.metas, &self.opts, &styles, &self.sheets);
        crate::build_result(xlsx, &self.opts, self.diags)
    }

    /// Seal the currently open sheet (if any) into a finished worksheet part.
    fn close_open(&mut self) -> Result<()> {
        let Some(open) = self.open.take() else {
            return Ok(());
        };
        let mut xml = sheet_prefix(&open.meta, open.rows, open.max_cols, &mut self.diags);
        xml.push_str(&open.body);
        xml.push_str(&sheet_suffix(
            &open.meta.merges,
            !open.meta.images.is_empty(),
            &mut self.diags,
        )?);
        self.sheets.push(xml);
        self.metas.push(open.meta);
        Ok(())
    }
}
