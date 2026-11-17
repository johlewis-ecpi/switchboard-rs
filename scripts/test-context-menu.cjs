const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const html = fs.readFileSync(path.join(__dirname, '../src/ui.html'), 'utf8');
const functions = ['contextMenu', 'invokeContextAction'].map(name => html.split('\n').find(line => line.startsWith(`function ${name}(`))).join('\n');
const sent = [], viewed = [], removed = [], moved = [], shown = [], contextElement = {innerHTML:''};
const entries = [{id:'a',kind:'file',favorite:false},{id:'b',kind:'folder',favorite:true},{id:'web',kind:'url',favorite:false}];
let selection = ['a','b'], hidden = 0, organized = 0;
const context = vm.createContext({
  window:{switchboardNativeContextMenu:true}, state:{minimal:true,scope:'all'},
  contextIds:[], selected:()=>selection, entity:id=>entries.find(e=>e.id===id),
  $:()=>contextElement, innerWidth:240, innerHeight:120,
  hideHover:()=>hidden++, send:(action,args)=>sent.push({action,...args}),
  viewDocument:id=>viewed.push(id), removeEntries:ids=>removed.push([...ids]),
  moveMinimal:(id,direction)=>moved.push({id,direction}),
  updateState:state=>shown.push(state), bulk:()=>organized++,
});
vm.runInContext(functions, context);
context.contextMenu('a',230,110);
assert.equal(contextElement.innerHTML,'', 'native menus are never constrained by the embedded context element');
assert.equal(sent.at(-1).action,'contextMenu');
assert.equal(sent.at(-1).x,230); assert.equal(sent.at(-1).y,110);
assert.equal(hidden,1,'hover popup dismisses before the menu opens');
for(const action of ['open','copy','reveal']) {
  context.invokeContextAction(action);
  assert.equal(sent.at(-1).action,action);
  assert.deepEqual([...sent.at(-1).ids],['a','b'],'selected groups are preserved');
}
context.invokeContextAction('viewDocument'); assert.deepEqual(viewed,['a']);
context.invokeContextAction('favorite'); assert.equal(sent.at(-1).value,'true');
context.invokeContextAction('clearCollection'); assert.equal(sent.at(-1).operation,'collection');
context.invokeContextAction('organize'); assert.equal(organized,1); assert.deepEqual([...shown.at(-1).selected],['a','b']);
context.invokeContextAction('show'); assert.equal(sent.at(-1).action,'showInLibrary'); assert.equal(sent.at(-1).id,'a');
context.invokeContextAction('earlier'); context.invokeContextAction('later');
assert.deepEqual(moved,[{id:'a',direction:-1},{id:'a',direction:1}]);
context.invokeContextAction('remove'); assert.deepEqual(removed,[['a','b']]);
context.contextMenu('web',20,20);
context.invokeContextAction('open'); assert.deepEqual([...sent.at(-1).ids],['web'],'right-clicking an unselected entry targets only that entry');
const oldMoves=moved.length; context.invokeContextAction('unknown'); assert.equal(moved.length,oldMoves);
entries.pop(); const oldSent=sent.length;
context.invokeContextAction('open'); assert.equal(sent.length,oldSent,'removed entries cannot be acted on by a stale menu');
// The standalone browser fallback still exposes the same menu options.
context.window.switchboardNativeContextMenu=false;
context.contextMenu('a',20,20);
for(const action of ['open','viewDocument','reveal','copy','favorite','organize','clearCollection','earlier','later','show','remove']) {
  assert(contextElement.innerHTML.includes(`data-context="${action}"`),action);
}
context.contextMenu('b',20,20); assert(contextElement.innerHTML.includes('Remove Favorite')); assert(!contextElement.innerHTML.includes('data-context="viewDocument"'));
console.log('Context menu checks passed: native delegation, all actions, selected groups, unselected entries, stale references, and browser fallback.');
