const assert=require('node:assert/strict'),fs=require('node:fs'),vm=require('node:vm'),path=require('node:path');
const html=fs.readFileSync(path.join(__dirname,'../readers/html/index.html'),'utf8');
class Element {
  constructor(tag='div'){this.tagName=tag;this.children=[];this.attributes={};this.listeners={};this.hidden=false;}
  append(...nodes){this.children.push(...nodes);}
  replaceChildren(...nodes){this.children=[...nodes];}
  setAttribute(name,value){this.attributes[name]=value;}
  addEventListener(name,fn){this.listeners[name]=fn;}
}
const elements=Object.fromEntries(['content','code','page','source'].map(id=>[id,new Element()]));
const messages=[],parent={postMessage:message=>messages.push(message)};
let listener;
const context=vm.createContext({document:{getElementById:id=>elements[id],createElement:tag=>new Element(tag)},parent,window:{addEventListener:(name,fn)=>{if(name==='message')listener=fn;}}});
vm.runInContext(html.match(/<script>([\s\S]*?)<\/script>/)[1],context);
assert.equal(messages[0].type,'switchboard:ready');
const documentText='<!doctype html><style>h1{color:red}</style><h1>Visual page</h1><script>window.example=true;</script>';
const open=(request,extension='html')=>({type:'switchboard:open',apiVersion:1,request,document:{name:'page.html',extension,text:documentText,readOnly:true}});
listener({source:{},data:open(1)});assert.equal(elements.content.children.length,0);
listener({source:parent,data:open(1)});
const first=elements.content.children[0];
assert.equal(first.tagName,'iframe');assert.equal(first.attributes.sandbox,'allow-scripts');
assert(first.srcdoc.includes("frame-src 'none'"));assert(first.srcdoc.includes("connect-src 'none'"));
assert(first.srcdoc.endsWith(documentText));assert.equal(elements.code.textContent,documentText);
assert.equal(elements.content.hidden,false);first.listeners.load();assert.equal(messages.at(-1).type,'switchboard:rendered');assert.equal(messages.at(-1).request,1);
elements.source.listeners.click();assert.equal(elements.content.hidden,true);assert.equal(elements.code.hidden,false);
elements.page.listeners.click();assert.equal(elements.content.hidden,false);assert.equal(elements.code.hidden,true);
listener({source:parent,data:open(2,'HTM')});const second=elements.content.children[0];assert.notEqual(first,second);
const count=messages.length;first.listeners.load();assert.equal(messages.length,count,'old document frames cannot acknowledge the new request');
second.listeners.load();assert.equal(messages.at(-1).request,2);
listener({source:parent,data:open(3,'txt')});assert.equal(messages.at(-1).type,'switchboard:error');assert.equal(messages.at(-1).request,3);
assert.equal(elements.code.textContent,'');assert.equal(elements.content.children[0].attributes.role,'alert');
const unsupported=open(4);unsupported.document.readOnly=false;listener({source:parent,data:unsupported});assert.equal(messages.at(-1).type,'switchboard:error');
console.log('HTML reader checks passed: separate document sandbox, visual/source modes, scripts and styles preserved, source validation, stale frames, errors, and lifecycle reports.');
