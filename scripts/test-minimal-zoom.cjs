// Exercise real gesture handlers, including persistence timing and Safari's
// cumulative scale events, without needing a physical trackpad.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(require('node:path').join(__dirname, '../src/ui.html'), 'utf8');
const begin = source.indexOf('// Minimal icon zoom:');
const code = source.slice(begin, source.indexOf('\nfunction focusSearch(', begin));
const handlers = {}, sent = [], timers = new Map();
let now = 0, sequence = 0, renders = 0;
const context = vm.createContext({
  state: {minimal: true, minimalIconSize: 80}, data: {launcher: false}, dialogKind: '', dragId: null,
  $: () => ({clientHeight: 300, addEventListener: (name, fn) => { handlers[name] = fn; }}),
  window: {addEventListener: (name, fn) => { handlers[name] = fn; }},
  send: (action, args) => sent.push({action, ...args}),
  hideHover: () => {}, resetBloom: () => {}, renderContent: () => { renders++; },
  selected: () => [], revealSelected: () => {},
  requestAnimationFrame: fn => { fn(); return 1; }, cancelAnimationFrame: () => {},
  setTimeout: (fn, delay) => { const id = ++sequence; timers.set(id, {fn, at: now + delay}); return id; },
  clearTimeout: id => timers.delete(id),
});
vm.runInContext(code, context);
function advance(ms) {
  const end = now + ms;
  while (true) {
    const next = [...timers].filter(([, timer]) => timer.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) break;
    timers.delete(next[0]); now = next[1].at; next[1].fn();
  }
  now = end;
}
function event(name, props = {}) {
  let prevented = false;
  handlers[name]({...props, preventDefault: () => { prevented = true; }});
  return prevented;
}
context.minimalMagnify({delta: .25});
assert.equal(context.state.minimalIconSize, 100);
assert.equal(sent.length, 0);
context.minimalMagnify({delta: 0, ended: true});
assert.equal(sent.at(-1).action, 'minimalZoom');
assert.equal(sent.at(-1).size, 100);
assert(renders > 0);
context.state.minimalIconSize = 80;
for (let i = 0; i < 10; i++) context.minimalMagnify({delta: .002});
assert.equal(context.state.minimalIconSize, 82, 'fractional magnification must accumulate');
context.minimalMagnify({delta: 0, ended: true});
context.minimalMagnify({delta: 1000, ended: true}); assert.equal(context.state.minimalIconSize, 160);
context.minimalMagnify({delta: -1000, ended: true}); assert.equal(context.state.minimalIconSize, 48);
context.minimalMagnify({delta: NaN}); assert.equal(context.state.minimalIconSize, 48);

// Safari's scale is cumulative from gesturestart, even after a pause.
context.state.minimalIconSize = 80;
assert(event('gesturestart'));
event('gesturechange', {scale: 1.2}); assert.equal(context.state.minimalIconSize, 96);
advance(300);
event('gesturechange', {scale: 1.25}); assert.equal(context.state.minimalIconSize, 100);
event('gesturechange', {scale: 1.25}); assert.equal(context.state.minimalIconSize, 100);
event('gestureend', {scale: 1.25}); assert.equal(sent.at(-1).size, 100);

// Ordinary scrolling is untouched; wheel zoom is coalesced and saved once idle.
context.state.minimalIconSize = 80;
assert.equal(event('wheel', {ctrlKey: false, deltaY: -20}), false);
assert.equal(context.state.minimalIconSize, 80);
const previousSaves = sent.length;
assert(event('wheel', {ctrlKey: true, deltaY: -20, deltaMode: 0}));
assert(context.state.minimalIconSize > 80);
advance(249); assert.equal(sent.length, previousSaves);
advance(1); assert.equal(sent.length, previousSaves + 1);
assert.equal(sent.at(-1).size, context.state.minimalIconSize);
for (const guard of ['normal', 'launcher', 'dialog', 'drag']) {
  context.state.minimal = guard !== 'normal';
  context.data.launcher = guard === 'launcher';
  context.dialogKind = guard === 'dialog' ? 'Configuration' : '';
  context.dragId = guard === 'drag' ? 'entry' : null;
  const size = context.state.minimalIconSize;
  context.minimalMagnify({delta: .5, ended: true});
  assert.equal(event('gesturestart'), false);
  assert.equal(event('wheel', {ctrlKey: true, deltaY: -20}), false);
  assert.equal(context.state.minimalIconSize, size);
}
context.state.minimal = true; context.data.launcher = false; context.dialogKind = ''; context.dragId = null;
context.minimalMagnify({delta: -.1}); event('blur');
assert.equal(sent.at(-1).size, context.state.minimalIconSize);
console.log('Minimal zoom checks passed: native deltas, fractional input, gesture scale, limits, scrolling, guards, and saved size.');
