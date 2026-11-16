const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
const source=fs.readFileSync(path.join(__dirname,'../src/viewer_keys.js'),'utf8');
function setup(main=false){
  const handlers={},messages=[],children=[];const parent={postMessage:message=>messages.push(message)};
  const window={addEventListener:(type,fn)=>handlers[type]=fn};window.top=main?window:{};
  const document={activeElement:null,querySelectorAll:()=>children};
  const context=vm.createContext({window,parent,document});vm.runInContext(source,context);return{context,window,handlers,messages,children};
}
function event(control=null,patch={}){return{key:' ',repeat:false,preventDefault(){this.prevented=true;},stopPropagation(){this.stopped=true;},target:{closest:()=>control},...patch};}
const root=setup(true);
assert(root.window.switchboardViewerSpaceEligible(event()));
for(const tag of ['input','textarea','select','button','a','video'])assert(!root.window.switchboardViewerSpaceEligible(event({matches:()=>false,tagName:tag})));
assert(root.window.switchboardViewerSpaceEligible(event({matches:selector=>selector==='[data-entry]'})),'library cards can open viewers despite their button role');
for(const patch of [{key:'Enter'},{ctrlKey:true},{metaKey:true},{shiftKey:true},{altKey:true},{isComposing:true},{target:{isContentEditable:true}}])assert(!root.window.switchboardViewerSpaceEligible(event(null,patch)));
const child=setup();
child.handlers.keydown(event());assert.equal(child.messages.at(-1).type,'switchboard:viewer-space');
const before=child.messages.length;const repeat=event(null,{repeat:true});child.handlers.keydown(repeat);assert.equal(child.messages.length,before);assert(repeat.prevented);
child.handlers.keydown(event({matches:()=>true}));assert.equal(child.messages.length,before,'a document cannot spoof a library card to suppress its button behavior');
child.handlers.message({source:{},data:{type:'switchboard:viewer-space',apiVersion:1}});assert.equal(child.messages.length,before);
const nested={};child.children.push({contentWindow:nested});child.handlers.message({source:nested,data:{type:'switchboard:viewer-space',apiVersion:1}});assert.equal(child.messages.length,before+1,'HTML document keys relay through their current parent frame');
let opened=0,closed=0;const handlers={};
Object.assign(root.context,{data:{},dialogKind:'',send(){},$:()=>null});
root.context.window.addEventListener=(type,fn)=>handlers[type]=fn;
vm.runInContext(fs.readFileSync(path.join(__dirname,'../src/viewer.js'),'utf8'),root.context);
root.context.viewDocument=()=>opened++;
root.context.closeDialog=()=>{closed++;vm.runInContext("documentViewer=null;dialogKind=''",root.context);};
root.context.handleViewerSpace(event());assert.equal(opened,1);
vm.runInContext("documentViewer={request:1};dialogKind='Content viewer'",root.context);
root.context.handleViewerSpace(event(null,{repeat:true}));assert.equal(closed,0);
root.context.handleViewerSpace(event());assert.equal(closed,1);
root.context.dialogKind='Settings';assert.equal(root.context.handleViewerSpace(event()),false);assert.equal(opened,1);
const currentFrame={};root.context.currentFrame=currentFrame;vm.runInContext("documentViewer={frame:{contentWindow:currentFrame}};dialogKind='Content viewer'",root.context);
handlers.message({source:{},data:{type:'switchboard:viewer-space',apiVersion:1}});assert.equal(closed,1);
handlers.message({source:currentFrame,data:{type:'switchboard:viewer-space',apiVersion:1}});assert.equal(closed,2);
console.log('Viewer keyboard checks passed: Space toggle, repeat and modifier guards, text/buttons/media, library cards, frame identity, nested HTML relay, and modal isolation.');
