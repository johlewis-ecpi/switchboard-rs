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
  setTimeout: fn => {timers.set(++timerId,fn);return timerId;}, clearTimeout: id => timers.delete(id),
});
vm.runInContext(source, context);
const run = code => vm.runInContext(code, context);
run("documentViewer={id:'a',request:2,name:'Example'}");
context.receiveDocument({request:1,error:'old failure'});
assert.equal(elements['#dialogs .body'].innerHTML, undefined, 'late responses cannot replace the viewer');
context.receiveDocument({request:2,document:{renderer:'package',readerName:'Test',extension:'gan',readerHtml:'<script>reader</script>',text:'<project/>'}});
const frame = run('documentViewer.frame');
assert.equal(frame.attributes.sandbox, 'allow-scripts');
assert(frame.srcdoc.includes("connect-src 'none'"));
handlers.message({source:{},data:{apiVersion:1,type:'switchboard:ready'}});
assert.equal(sent.length,0,'ignore unrelated frames');
handlers.message({source:frame.contentWindow,data:{apiVersion:99,type:'switchboard:ready'}});
assert.equal(sent.length,0,'ignore unsupported versions');
handlers.message({source:frame.contentWindow,data:{apiVersion:1,type:'switchboard:ready'}});
assert.equal(sent[0].document.text,'<project/>');
assert.equal(sent[0].document.readOnly,true);
assert.equal(sent[0].document.path,undefined);
assert.equal(timers.size,0,'ready clears loading deadline');
handlers.message({source:frame.contentWindow,data:{apiVersion:1,type:'switchboard:error',request:1,message:'old'}});
assert(!elements['#dialogs .body'].innerHTML.includes('old'));
context.disposeDocumentViewer();
assert.equal(sent.at(-1).action,'closeViewer');
context.receiveDocument({request:2,error:'closed response'});
assert.equal(run('documentViewer'),null);
run("documentViewer={request:3,name:'Office'}");
context.receiveDocument({request:3,document:{renderer:'native',readerName:'Quick Look'}});
assert.equal(sent.at(-1).action,'layoutViewer');
assert.deepEqual(Array.from(sent.at(-1).rect),[10,20,800,500]);
context.disposeDocumentViewer();
assert(sent.some(item=>item.disconnected));
// Only the bundled HTML adapter can create a separate document frame.
run("documentViewer={request:4,name:'HTML'}");
context.receiveDocument({request:4,document:{renderer:'html',readerName:'HTML Viewer',readerHtml:'reader',text:'<h1>Page</h1>'}});
const htmlFrame=run('documentViewer.frame');
assert(htmlFrame.srcdoc.includes('frame-src about:'));
assert.equal(htmlFrame.attributes.sandbox,'allow-scripts');
context.disposeDocumentViewer();
run("documentViewer={request:5,name:'Installed reader'}");
context.receiveDocument({request:5,document:{renderer:'package',readerName:'HTML Viewer',readerHtml:'reader',text:'<h1>Page</h1>'}});
assert(run('documentViewer.frame').srcdoc.includes("frame-src 'none'"),'a package name cannot grant HTML frame capability');
context.disposeDocumentViewer();
const html = fs.readFileSync(path.join(__dirname, '../src/ui.html'),'utf8').replace('/* VIEWER_MODULE */', source).replace('/* VIEWER_MANAGER */', fs.readFileSync(path.join(__dirname, '../src/viewer_manager.js'), 'utf8'));
new vm.Script(html.match(/<script>([\s\S]*)<\/script>/)[1]);
// Library previews delegate to a native window even when the library is minimal.
context.entity = id => id === 'local' ? {id, name:'Example', kind:'file'} : {id, kind:'url'};
context.hideHover = () => {};
context.toast = message => sent.push({toast:message});
context.state = {minimal:true};
context.viewDocument('local');
assert.equal(sent.at(-1).action, 'openViewerWindow');
assert.equal(sent.at(-1).id, 'local');
assert.equal(run('documentViewer'), null, 'the small library window does not create a viewer dialog');
context.viewDocument('web');
assert(sent.at(-1).toast, 'unsupported references still report a selection error');
context.window.switchboardPreviewWindow = true;
context.createDocumentViewer = (name,id) => run(`documentViewer={id:${JSON.stringify(id)},request:42}`);
context.viewDocument('local');
assert.equal(sent.at(-1).action, 'viewDocument', 'the dedicated window loads the reader without opening another window');
assert.equal(sent.at(-1).request, 42);
console.log('Viewer checks passed: stale replies, frame isolation, handshake, deadlines, read-only payloads, native layout, cleanup, shell syntax.');
