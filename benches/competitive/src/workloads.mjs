// Shared logical workloads, expressed in a neutral cell form each adapter
// translates into its own library's model. Keeping the *logical* content
// identical across libraries is what makes the perf and conformance comparisons
// apples-to-apples.

/** A neutral cell: kind + value, plus an optional `header` style flag. */
export function tabularRows(rowCount, colCount) {
  const rows = [];
  // Bold, filled header band.
  const header = [];
  for (let c = 0; c < colCount; c++)
    header.push({ kind: "string", value: `Col ${c}`, header: true });
  rows.push(header);
  // Data rows: a label column then currency amounts (minor units).
  for (let r = 0; r < rowCount; r++) {
    const row = [{ kind: "string", value: `Empleado ${r}` }];
    for (let c = 1; c < colCount; c++) row.push({ kind: "currency", value: (r * c + 1) * 100 });
    rows.push(row);
  }
  return rows;
}

/** A small, feature-rich workbook used to probe formatting fidelity. Every cell
 *  exercises a capability the conformance harness reads back. */
export function featureRows() {
  return [
    [
      { kind: "string", value: "Departamento", header: true },
      { kind: "string", value: "Bruto", header: true },
      { kind: "string", value: "Tasa", header: true },
      { kind: "string", value: "Fecha", header: true },
    ],
    [
      { kind: "string", value: "Ingeniería" },
      { kind: "currency", value: 1234567, negative: "red-parens" },
      { kind: "percent", value: 0.16 },
      { kind: "date", value: "2026-06-20" },
    ],
    [
      { kind: "string", value: "Total", total: true },
      { kind: "currency", value: 1234567, total: true },
      { kind: "blank" },
      { kind: "blank" },
    ],
  ];
}

/** The currency amount in major units (libraries other than turbo-xlsx take the
 *  decimal value directly, not integer minor units). */
export function toMajor(minorUnits) {
  return minorUnits / 100;
}
