/* Pure sample-format parser. Reuse your owning application's parser here. */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.SampleReportFormat = api;
})(globalThis, function () {
  'use strict';
  const limits = Object.freeze({characters: 4 * 1024 * 1024, rows: 2000, columns: 24, cell: 4096});
  const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
  function onlyKeys(value, allowed, label) {
    if (Object.keys(value).some(key => !allowed.includes(key))) throw new Error(`${label} contains an unsupported field`);
  }
  function string(value, maximum, label, required = false) {
    if (typeof value !== 'string' || Array.from(value).length > maximum || (required && !value.trim())) throw new Error(`${label} must be ${required ? 'nonempty ' : ''}text of at most ${maximum} characters`);
    return value;
  }
  function parseReport(text) {
    if (typeof text !== 'string' || text.length > limits.characters) throw new Error('Sample report exceeds the text limit');
    let data;
    try { data = JSON.parse(text.replace(/^\uFEFF/, '')); } catch { throw new Error('Sample report must contain valid JSON'); }
    if (!object(data)) throw new Error('Sample report must be a JSON object');
    onlyKeys(data, ['schemaVersion', 'title', 'description', 'columns', 'rows'], 'Report');
    if (data.schemaVersion !== 1) throw new Error('Unsupported sample report schemaVersion; expected 1');
    const title = string(data.title, 200, 'Title', true);
    const description = string(data.description === undefined ? '' : data.description, 4000, 'Description');
    if (!Array.isArray(data.columns) || !data.columns.length || data.columns.length > limits.columns) throw new Error('Declare 1–24 columns');
    const keys = new Set();
    const columns = data.columns.map(column => {
      if (!object(column)) throw new Error('Each column must be an object');
      onlyKeys(column, ['key', 'label'], 'Column');
      if (typeof column.key !== 'string' || !/^[A-Za-z][A-Za-z0-9_]{0,63}$/.test(column.key) || ['constructor', 'prototype', '__proto__'].includes(column.key) || keys.has(column.key)) throw new Error('Column keys must be unique, safe identifiers');
      keys.add(column.key);
      return Object.freeze({key: column.key, label: string(column.label, 100, 'Column label', true)});
    });
    if (!Array.isArray(data.rows) || data.rows.length > limits.rows) throw new Error('Declare at most 2000 rows');
    const rows = data.rows.map(row => {
      if (!object(row)) throw new Error('Each row must be an object');
      onlyKeys(row, [...keys], 'Row');
      return Object.freeze(columns.map(column => {
        const value = Object.hasOwn(row, column.key) ? row[column.key] : null;
        if (value === null) return '';
        if (typeof value === 'string') return string(value, limits.cell, 'Cell');
        if (typeof value === 'boolean' || (typeof value === 'number' && Number.isFinite(value))) return String(value);
        throw new Error('Cells must be strings, finite numbers, booleans or null');
      }));
    });
    return Object.freeze({title, description, columns: Object.freeze(columns), rows: Object.freeze(rows)});
  }
  return Object.freeze({parseReport, limits});
});
