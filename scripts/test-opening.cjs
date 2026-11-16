const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src/ui.html'), 'utf8');
const start = source.indexOf('// Opening feedback');
const code = source.slice(start, source.indexOf('\nconst uuid=', start));
class Element {
  constructor(id, tagName = 'DIV') {
    this.dataset = {entry: id}; this.tagName = tagName; this.attributes = {}; this.children = [];
    const classes = new Set();
    this.classList = {toggle: (name, on) => on ? classes.add(name) : classes.delete(name), contains: name => classes.has(name)};
  }
  setAttribute(name, value) { this.attributes[name] = value; }
  removeAttribute(name) { delete this.attributes[name]; }
  append(child) { child.parent = this; this.children.push(child); }
  remove() { this.parent.children = this.parent.children.filter(child => child !== this); }
  querySelector(selector) { return this.children.find(child => selector === 'td' ? child.tagName === 'TD' : child.className === selector.slice(1)) || this.children.map(child => child.querySelector(selector)).find(Boolean); }
}
const rows = [new Element('a'), new Element('b'), new Element('table', 'TR')];
rows[2].append(new Element('cell', 'TD'));
const status = new Element('status'); const label = new Element('label'); label.className = 'opening-label'; status.append(label);
const sent = [], errors = [], frames = [], timers = new Map();
let now = 0, sequence = 0;
const context = vm.createContext({
  $: () => status, $$: () => rows, entity: id => rows.some(row => row.dataset.entry === id) ? {name: id.toUpperCase()} : null,
  document: {createElement: () => new Element('indicator', 'SPAN')},
  window: {switchboardPostMessage: value => sent.push(JSON.parse(value))},
  performance: {now: () => now}, hideHover: () => {}, toast: error => errors.push(error),
  requestAnimationFrame: fn => { frames.push(fn); return frames.length; },
  setTimeout: (fn, delay) => { const id = ++sequence; timers.set(id, {fn, at: now + delay}); return id; },
  clearTimeout: id => timers.delete(id),
});
vm.runInContext(code, context);
function paint() { while (frames.length) frames.shift()(); }
function advance(ms) {
  const end = now + ms;
  while (true) {
    const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) break;
    timers.delete(next[0]); now = next[1].at; next[1].fn();
  }
  now = end;
}
context.beginOpen({ids: ['a']});
assert.equal(rows[0].attributes['aria-busy'], 'true');
assert(rows[0].querySelector('.opening-indicator'));
assert.equal(sent.length, 0, 'feedback must exist before sending the open command');
paint(); const first = sent[0].requestId;
context.beginOpen({ids: ['a']}); paint(); assert.equal(sent.length, 1, 'duplicate pending click must not relaunch');
rows[0].children = []; context.updateLoadingUI(); assert(rows[0].querySelector('.opening-indicator'), 'indicator survives redraws');
context.beginOpen({ids: ['b']}); paint(); const second = sent.at(-1).requestId;
assert(label.textContent.includes('2 references'));
context.openFinished({requestId: first}); advance(349); assert.equal(rows[0].attributes['aria-busy'], 'true');
advance(1); assert.equal(rows[0].attributes['aria-busy'], undefined); assert.equal(rows[1].attributes['aria-busy'], 'true');
context.beginOpen({ids: ['a']}); paint(); const third = sent.at(-1).requestId;
context.openFinished({requestId: first, error: 'stale error'}); assert.equal(rows[0].attributes['aria-busy'], 'true');
context.openFinished({requestId: second, error: 'Cannot open B'}); advance(0);
assert.deepEqual(errors, ['Cannot open B']); assert.equal(rows[1].attributes['aria-busy'], undefined);
context.openFinished({requestId: third}); context.openFinished({requestId: third}); advance(350);
assert(status.classList.contains('hidden')); assert.equal(rows[0].attributes['aria-busy'], undefined);
context.beginOpen({ids: ['table', 'table']}); paint();
assert(rows[2].querySelector('td').querySelector('.opening-indicator'));
assert.equal(sent.at(-1).ids.length, 1);
context.openFinished({requestId: sent.at(-1).requestId}); advance(350);
context.window.switchboardPostMessage = () => { throw Error('IPC unavailable'); };
context.beginOpen({ids: ['a']}); paint(); advance(350);
assert.equal(errors.at(-1), 'IPC unavailable'); assert(status.classList.contains('hidden'));
console.log('Opening checks passed: immediate feedback, duplicate clicks, redraws, concurrent requests, errors, stale replies, and table rows.');
