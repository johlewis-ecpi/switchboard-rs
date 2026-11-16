/* Format-specific view. Independent of Switchboard's transport and catalog. */
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.SampleReportRenderer = api;
})(globalThis, function () {
  'use strict';
  function renderReport(model, {document: metadata, signal, dom = globalThis.document}) {
    const node = (tag, text, className) => {
      const element = dom.createElement(tag);
      if (text !== undefined) element.textContent = text;
      if (className) element.className = className;
      return element;
    };
    const article = node('article');
    article.append(node('p', metadata.name, 'filename'), node('h1', model.title));
    if (model.description) article.append(node('p', model.description, 'description'));
    const toolbar = node('div', undefined, 'toolbar');
    const searchLabel = node('label', 'Find rows ');
    const search = node('input'); search.type = 'search'; search.placeholder = 'Search this document';
    searchLabel.append(search);
    const wrapLabel = node('label', undefined, 'wrap');
    const wrap = node('input'); wrap.type = 'checkbox'; wrap.checked = true;
    wrapLabel.append(wrap, node('span', 'Wrap text'));
    toolbar.append(searchLabel, wrapLabel); article.append(toolbar);
    const count = node('p', undefined, 'count'); count.setAttribute('role', 'status'); count.setAttribute('aria-live', 'polite');
    article.append(count);
    const scroll = node('div', undefined, 'table-scroll'); scroll.tabIndex = 0; scroll.setAttribute('aria-label', 'Report table');
    const table = node('table'); const caption = node('caption', model.title); caption.className = 'visually-hidden'; table.append(caption);
    const head = node('thead'); const headings = node('tr');
    for (const column of model.columns) { const cell = node('th', column.label); cell.scope = 'col'; headings.append(cell); }
    head.append(headings); table.append(head);
    const body = node('tbody');
    const rows = model.rows.map(values => {
      const tr = node('tr'); for (const value of values) tr.append(node('td', value)); body.append(tr);
      return {element: tr, text: values.join(' ').toLocaleLowerCase()};
    });
    table.append(body); scroll.append(table); article.append(scroll);
    const empty = node('p', '', 'empty'); article.append(empty);
    function filter() {
      const query = search.value.trim().toLocaleLowerCase(); let visible = 0;
      for (const row of rows) { row.element.hidden = !row.text.includes(query); if (!row.element.hidden) visible++; }
      count.textContent = `${visible} of ${rows.length} rows`;
      empty.textContent = rows.length === 0 ? 'This document has no rows.' : visible === 0 ? 'No rows match your search.' : '';
      empty.hidden = !empty.textContent;
    }
    search.addEventListener('input', filter, {signal});
    wrap.addEventListener('change', () => table.classList.toggle('no-wrap', !wrap.checked), {signal});
    filter();
    return article;
  }
  return Object.freeze({renderReport});
});
