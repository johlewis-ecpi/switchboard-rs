const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const sent = [], listeners = {}, elements = {}, timers = new Map();
let timerId = 0;
class Element {
  constructor() { this.dataset={};this.attributes={};this.children=[];this.classList={add(){},toggle(){}};this.contentWindow={postMessage:message=>sent.push(message)}; }
  set innerHTML(value) { this.html=value;if(value.includes('readerHost'))elements['#readerHost']=new Element(); }
  get innerHTML() { return this.html; }
  setAttribute(name,value) { this.attributes[name]=value; }
  append(...items) { this.children.push(...items); }
  prepend(item) { elements['#'+item.id]=item; }
  querySelector(selector) { return elements['#dialogs .modal '+selector]; }
  focus() {}
}
for(const key of ['#dialogs .body','#dialogs .modal','#dialogs .modal header','#dialogs .modal footer','#readerName','#viewerTestStatus','#confirmViewerRendering'])elements[key]=new Element();
const context = vm.createContext({
  window:{addEventListener:(name,fn)=>listeners['window:'+name]=fn},
  document:{createElement:()=>new Element(),addEventListener:(name,fn)=>listeners[name]=fn},
  $:selector=>elements[selector], esc:value=>String(value??'').replaceAll('<','&lt;').replaceAll('"','&quot;'),attr:value=>String(value),
  send:(action,args)=>sent.push({action,...args}),hideHover(){},toast:message=>sent.push({toast:message}),
  setTimeout:fn=>{timers.set(++timerId,fn);return timerId;},clearTimeout:id=>timers.delete(id),
  dialogKind:'',Date,
});
vm.runInContext(fs.readFileSync(path.join(__dirname,'../src/viewer.js'),'utf8'),context);
vm.runInContext(`function modal(title){disposeDocumentViewer();dialogKind=title;} function closeDialog(){disposeDocumentViewer();dialogKind='';}`,context);
vm.runInContext(fs.readFileSync(path.join(__dirname,'../src/viewer_manager.js'),'utf8'),context);
const run = code=>vm.runInContext(code,context);
const click = dataset=>listeners.click({target:{closest:()=>({dataset})}});
context.viewerSettings();
assert.equal(sent.at(-1).action,'listViewers');
const inventory={directory:'/tmp/readers',readers:[
  {key:'builtin:text',name:'Text',source:'bundled',enabled:true,extensions:[],validationError:null,conflicts:[],lastTest:null},
  {key:'user:broken',name:'<broken>',source:'installed',enabled:true,extensions:['gan'],validationError:'Invalid manifest',conflicts:[],path:'/tmp/readers/broken',lastTest:null},
  {key:'user:good',name:'Gantt',source:'installed',version:'1.0',enabled:true,extensions:['gan'],validationError:null,conflicts:[],path:'/tmp/readers/good',testStale:true,
    lastTest:{checkedAt:'2026-11-11T12:00:00Z',sampleName:'plan.gan',sampleExtension:'gan',startup:'passed',rendering:'reported',detail:'Reader reports completion'}},
]};
context.receiveViewerInventory(inventory);
const html=elements['#dialogs .body'].innerHTML;
assert(html.includes('&lt;broken>'));assert(html.includes('Invalid manifest'));
assert(html.includes('Not tested'));assert(html.includes('visual check needed'));assert(html.includes('changed since this test'));
assert(!html.includes('Working properly'),'package validity and self-reported rendering must not claim correctness');
click({viewerOp:'toggle',viewerKey:'user:good'});
assert.equal(sent.at(-1).action,'setViewerEnabled');assert.equal(sent.at(-1).enabled,false);
click({viewerOp:'test',viewerKey:'user:good'});
const request=sent.at(-1).request;
assert.equal(sent.at(-1).action,'testViewer');
const result={startup:'waiting',rendering:'unverified',detail:'Awaiting startup'};
context.receiveViewerSample({request,key:'user:good',name:'sample.gan',result,document:{renderer:'package',readerName:'Gantt',extension:'gan',text:'<project/>',readerHtml:'reader'}});
const frame=run('documentViewer.frame');
assert.equal(elements['#confirmViewerRendering'].disabled,true);
listeners['window:message']({source:frame.contentWindow,data:{type:'switchboard:ready',apiVersion:1}});
assert(sent.some(message=>message.action==='viewerTestEvent'&&message.event==='ready'));
listeners['window:message']({source:frame.contentWindow,data:{type:'switchboard:rendered',apiVersion:1,request}});
assert.equal(sent.at(-1).event,'rendered');
assert(!sent.some(message=>message.action==='viewerTestVerdict'),'renderer cannot visually confirm itself');
context.receiveViewerTestStatus({request,result:{startup:'passed',rendering:'reported',detail:'Check visually'}});
assert.equal(elements['#confirmViewerRendering'].disabled,false);
click({viewerVerdict:'pass'});
assert.equal(sent.at(-1).action,'viewerTestVerdict');assert.equal(sent.at(-1).passed,true);
context.receiveViewerTestStatus({request:request-1,result:{startup:'failed',rendering:'failed',detail:'stale'}});
assert.equal(elements['#confirmViewerRendering'].disabled,false,'ignore stale test status');
context.readerFailure('Reader failed');
assert.equal(sent.at(-1).event,'error');
context.receiveViewerTestStatus({request,result:{startup:'passed',rendering:'failed',detail:'failed'}});
assert.equal(elements['#confirmViewerRendering'].disabled,true,'failed, removed content cannot be confirmed');
context.disposeDocumentViewer();
assert.equal(sent.at(-1).request,request);
// A never-ready reader produces a startup failure rather than a rendering pass.
context.viewerSettings();context.requestViewerTest('user:good');
const second=sent.at(-1).request;
context.receiveViewerSample({request:second,key:'user:good',name:'sample.gan',result,document:{renderer:'package',readerName:'Gantt',readerHtml:'silent',text:'sample'}});
for(const fn of [...timers.values()])fn();
assert.equal(sent.at(-1).event,'error');assert(sent.at(-1).message.includes('15 seconds'));
console.log('Viewer manager checks passed: inventory errors, stale results, enable/disable, direct sample tests, handshake, visual confirmation, errors, deadlines, and cleanup.');
