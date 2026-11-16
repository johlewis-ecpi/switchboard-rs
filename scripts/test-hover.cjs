// Exercise the actual hover handlers without a native window or user library.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const html = fs.readFileSync(require('node:path').join(__dirname, '../src/ui.html'), 'utf8');
const script = html.split('<script>')[1].split('</script>')[0];
new vm.Script(script); // Check syntax of the entire bundled interface, too.
const handlers = {};
const surface = name => ({addEventListener: (event, fn) => { handlers[name + ':' + event] = fn; }});
const content = surface('content'), document = surface('document'), window = surface('window');
const classes = new Set(['hidden']);
const hover = {dataset: {}, style: {}, classList: {add: c => classes.add(c), remove: c => classes.delete(c)}};
const entry = {id: 'one', name: 'Example', kind: 'file', location: '/tmp/example', tags: [], added: 0};
let now = 0, sequence = 0;
const timers = new Map(), requests = [];
const context = vm.createContext({
  document, window, $: selector => selector === '#content' ? content : hover,
  state: {minimal: true}, data: {content: {}}, dialogKind: '', dragId: null,
  minimalZoomActive: false, hoverId: null, hoverTimer: null, entity: id => id === entry.id ? entry : null,
  requestAvailability: () => {}, availabilityDetails: () => '', positionHover: () => {}, esc: String, date: () => new Date(0), innerWidth: 900, innerHeight: 600,
  previewCache: {}, previewRequests: new Set(), showPreview: () => {},
  send: (...args) => requests.push(args), resetBloom: () => {}, renderMinimalTiles: () => {},
  setTimeout: (fn, delay) => { const id = ++sequence; timers.set(id, {fn, at: now + delay}); return id; },
  clearTimeout: id => timers.delete(id),
});
vm.runInContext(script.slice(script.indexOf('function hideHover()'), script.indexOf('function resetBloom()')), context);
function advance(ms) {
  const end = now + ms;
  while (true) {
    const next = [...timers].filter(([, t]) => t.at <= end).sort((a, b) => a[1].at - b[1].at)[0];
    if (!next) break;
    timers.delete(next[0]); now = next[1].at; next[1].fn();
  }
  now = end;
}
const row = {dataset: {entry: 'one'}};
const child = () => ({closest: () => row});
const event = {target: child(), relatedTarget: null, clientX: 100, clientY: 100, buttons: 0};
const enter = () => handlers['content:mouseover'](event);
enter(); advance(449); assert(classes.has('hidden'));
// Moving within a tile must preserve the original 450 ms deadline.
handlers['content:mouseout']({...event, relatedTarget: child()});
handlers['content:mouseover']({...event, relatedTarget: child()});
advance(1); assert(!classes.has('hidden')); assert(hover.innerHTML.includes('Example'));
assert.equal(requests[0][0], 'preview');
handlers['content:mouseout'](event); advance(79); assert(!classes.has('hidden'));
advance(1); assert(classes.has('hidden'));
for (const name of ['document:keydown', 'document:mousedown', 'content:scroll', 'window:blur']) {
  enter(); advance(450); assert(!classes.has('hidden'));
  handlers[name](); assert(classes.has('hidden'), name + ' should dismiss');
}
enter(); advance(100); handlers['content:mouseout'](event); advance(500); assert(classes.has('hidden'));
for (const guard of ['dialogKind', 'dragId']) {
  context[guard] = 'active'; enter(); advance(450); assert(classes.has('hidden')); context[guard] = '';
}
context.data.launcher = true; enter(); advance(450); assert(classes.has('hidden'));
// Revisit a cached thumbnail to validate its file metadata in the backend.
context.data.launcher = false;
context.previewCache.one = {image: 'data:image/png;base64,example'};
context.previewRequests.delete('one');
const previousRequests = requests.length;
enter(); advance(450);
assert.equal(requests.length, previousRequests + 1);
handlers['document:mousedown']();
console.log('Hover checks passed: Minimal mode, child transitions, delay, exit, dismissal, and interaction guards.');

// Late image responses must appear in the reserved preview above the metadata,
// and a tall popup must stay inside the viewport after the image loads.
const preview = {innerHTML: '', querySelector: () => ({addEventListener: (_, fn) => fn()})};
hover.offsetWidth = 400; hover.offsetHeight = 500;
hover.dataset = {x: 850, y: 550};
context.esc = value => String(value).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
context.$ = selector => selector === '#hover .file-preview' ? preview : hover;
vm.runInContext(script.slice(script.indexOf('function positionHover('), script.indexOf('\nconst labels=')), context);
context.previewCache.one = {image: 'data:image/png;base64,example', text: 'source text'};
vm.runInContext("showPreview('one')", context);
assert(preview.innerHTML.includes('<img'));
assert(!preview.innerHTML.includes('<pre'));
assert.equal(hover.style.left, '492px');
assert.equal(hover.style.top, '92px');
context.previewCache.one = {error: 'Native renderer failed <test>'};
vm.runInContext("showPreview('one')", context);
assert(preview.innerHTML.includes('Native renderer failed &lt;test&gt;'));
assert(preview.innerHTML.includes('Hover again to retry'));
context.previewCache.one = {viewer: 'GanttProject <Viewer>'};
vm.runInContext("showPreview('one')", context);
assert(preview.innerHTML.includes('GanttProject &lt;Viewer&gt;'));
assert(preview.innerHTML.includes('View in App'));
assert(!preview.innerHTML.includes('Preview unavailable'));
assert(!preview.innerHTML.includes('Hover again to retry'));
console.log('Preview checks passed: image priority, late response positioning, unavailable state, and registered viewer guidance.');

// A fresh Minimal session must request document thumbnails without a hover,
// then replace generic icons when cached previews arrive from the backend.
for (let session = 0; session < 2; session++) {
  const entries = [
    {id: 'word', kind: 'file', location: '/tmp/document.docx'},
    {id: 'app', kind: 'file', location: '/Applications/Example.app'},
    {id: 'folder', kind: 'folder', location: '/tmp/folder'},
  ];
  const icons = entries.map(e => ({dataset: {entry: e.id}, icon: {innerHTML: ''}, querySelector() { return this.icon; }}));
  const sent = [];
  const fresh = vm.createContext({
    state: {minimal: true}, lib: {view: 'mediumCards'}, data: {launcher: false},
    iconCache: {}, previewCache: {}, previewRequests: new Set(), hoverId: null,
    $$: () => icons, entity: id => entries.find(e => e.id === id), symbol: () => 'generic',
    send: (...args) => sent.push(args), Object,
  });
  vm.runInContext(script.slice(script.indexOf('function usesThumbnail('), script.indexOf('function positionHover(')), fresh);
  vm.runInContext('requestThumbnailIcons(); requestThumbnailIcons();', fresh);
  assert.equal(sent.length, 1);
  assert.equal(sent[0][1].id, 'word');
  vm.runInContext("receiveIcons({word:'native-word',app:'native-app',folder:'native-folder'}); receivePreview({id:'word',preview:{image:'cached-page'}})", fresh);
  assert(icons[0].icon.innerHTML.includes('cached-page'));
  assert(icons[1].icon.innerHTML.includes('native-app'));
  assert(icons[2].icon.innerHTML.includes('native-folder'));
  vm.runInContext("state.minimal=false; receiveIcons({})", fresh);
  assert(icons[0].icon.innerHTML.includes('native-word'));
  vm.runInContext("lib.view='thumbnails'; receiveIcons({})", fresh);
  assert(icons[0].icon.innerHTML.includes('cached-page'));
  vm.runInContext("data.launcher=true; receiveIcons({})", fresh);
  assert(icons[0].icon.innerHTML.includes('native-word'));
}
console.log('Thumbnail icon checks passed: fresh sessions, eager loading, async replacement, apps/folders, and display modes.');
