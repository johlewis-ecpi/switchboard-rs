const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const source = fs.readFileSync(path.join(__dirname, '../src/viewer.js'), 'utf8');
const sent = [], handlers = {}, timers = new Map(), elements = {};
let timerId = 0;
class Element {
  constructor(tag = 'div') { this.tagName = tag; this.children = []; this.style = {}; this.attributes = {}; this.classList = {add(){},toggle(){}}; this.contentWindow = {postMessage: message => sent.push(message)}; }
  set innerHTML(value) { this.html = value; if (value.includes('readerHost')) elements['#readerHost'] = new Element(); }
  get innerHTML() { return this.html; }
  setAttribute(key,value) { this.attributes[key] = value; }
  append(...children) { this.children.push(...children); }
  focus() {}
  getBoundingClientRect() { return {x:10,y:20,width:800,height:500}; }
}
elements['#dialogs .body'] = new Element(); elements['#readerName'] = new Element();
const context = vm.createContext({
  window: {addEventListener: (name, callback) => handlers[name] = callback},
  document: {createElement: tag => new Element(tag)},
  $: selector => elements[selector], dialogKind: 'Content viewer',
  esc: text => String(text).replaceAll('<','&lt;'),
  send: (action,args) => sent.push({action,...args}),
  ResizeObserver: class { observe(){} disconnect(){sent.push({disconnected:true});} },
  atob: text => Buffer.from(text,'base64').toString('binary'), closeDialog: () => sent.push({closed:true}),
  setTimeout: fn => {timers.set(++timerId,fn);return timerId;}, clearTimeout: id => timers.delete(id),
});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);
run("documentViewer={id:'a',request:7,name:'big.zip'}");
context.receiveDocument({request:7,document:{renderer:'package',apiVersion:2,size:1000,readerName:'Zip',extension:'zip',readerHtml:'<script>reader</script>',text:null}});
const frame = run('documentViewer.frame'), reader = frame.contentWindow;
const toHost = (message, source = reader) => handlers.message({source, data: message});
const read = (id, offset, length, request = 7) => toHost({apiVersion:2, type:'switchboard:read', request, id, offset, length});
const last = () => sent[sent.length - 1];
toHost({apiVersion:1, type:'switchboard:ready'});
assert.equal(sent.length, 0, 'a v2 document ignores v1 messages');
read(1, 0, 10);
assert.equal(sent.length, 0, 'reads before readiness are ignored');
toHost({apiVersion:2, type:'switchboard:ready'});
assert.deepEqual(JSON.parse(JSON.stringify(last())), {type:'switchboard:open', apiVersion:2, request:7, document:{name:'big.zip', extension:'zip', size:1000, readOnly:true}});
assert(!('text' in last().document), 'v2 readers never receive document contents');
sent.length = 0;
read(1, 990, 10);
assert.deepEqual(JSON.parse(JSON.stringify(sent)), [{action:'readViewerRange', request:7, id:1, offset:990, length:10}]);
sent.length = 0;
read(1, 0, 5);
assert.equal(sent.length, 0, 'duplicate in-flight ids are dropped');
read(2, 0, 5, 6); toHost({apiVersion:2, type:'switchboard:read', request:7, id:3, offset:0, length:5}, {});
assert.equal(sent.length, 0, 'stale requests and unrelated frames are ignored');
for (const [id, offset, length] of [[10, 996, 5], [11, -1, 1], [12, 0, 8 * 1024 * 1024 + 1], [13, 1.5, 1], [14, 0, '1'], [15, Number.MAX_SAFE_INTEGER, 2]]) {
  sent.length = 0; read(id, offset, length);
  assert.equal(last().type, 'switchboard:range-error', `rejects ${id}`);
  assert.equal(last().message, 'Invalid range');
}
sent.length = 0; read(20, 0, 0);
assert.equal(last().type, 'switchboard:range'); assert.equal(last().data.byteLength, 0);
// Concurrency cap: one id (1) is pending; 7 more fit, the 9th is refused.
sent.length = 0;
for (let id = 100; id < 107; id++) read(id, 0, 1);
assert.equal(sent.length, 7); sent.length = 0;
read(200, 0, 1); assert.equal(last().message, 'Too many outstanding reads');
// Replies.
sent.length = 0;
context.receiveViewerRange({request:7, id:1, data:Buffer.from('0123456789').toString('base64')});
assert.equal(last().type, 'switchboard:range'); assert.equal(last().id, 1); assert.equal(last().data.byteLength, 10);
context.receiveViewerRange({request:7, id:1, data:Buffer.from('0123456789').toString('base64')});
assert.equal(sent.length, 1, 'a reply is delivered once');
context.receiveViewerRange({request:6, id:100, data:'AA=='}); context.receiveViewerRange({request:7, id:999, data:'AA=='});
assert.equal(sent.length, 1, 'stale or unknown replies are dropped');
context.receiveViewerRange({request:7, id:100, data:Buffer.from('xx').toString('base64')});
assert.equal(last().type, 'switchboard:range-error'); assert.match(last().message, /unexpected range length/);
context.receiveViewerRange({request:7, id:101, error:'x'.repeat(900)});
assert.equal(last().type, 'switchboard:range-error'); assert.equal(last().message.length, 500);
// Errors and rendering reports keep working; closing the viewer stops delivery.
toHost({apiVersion:2, type:'switchboard:rendered', request:7});
toHost({apiVersion:2, type:'switchboard:error', request:7, message:'broken'});
assert(run('documentViewer.failed'), 'reader errors are surfaced');
run("documentViewer=null"); sent.length = 0;
context.receiveViewerRange({request:7, id:102, data:'AA=='});
assert.equal(sent.length, 0, 'no delivery after the viewer is closed');
// v1 documents are unchanged and cannot use the range channel.
run("documentViewer={id:'b',request:8,name:'a.gan'}");
context.receiveDocument({request:8,document:{renderer:'package',apiVersion:1,readerName:'Gantt',extension:'gan',readerHtml:'<script>r</script>',text:'<project/>'}});
const v1 = run('documentViewer.frame'); sent.length = 0;
handlers.message({source:v1.contentWindow, data:{apiVersion:1, type:'switchboard:ready'}});
assert.equal(last().apiVersion, 1); assert.equal(last().document.text, '<project/>');
sent.length = 0; handlers.message({source:v1.contentWindow, data:{apiVersion:1, type:'switchboard:read', request:8, id:1, offset:0, length:1}});
assert.equal(sent.length, 0, 'v1 readers cannot request ranges');
console.log('Viewer range checks passed: v2 handshake, bounds, concurrency cap, reply validation, supersession, cleanup, v1 isolation.');
