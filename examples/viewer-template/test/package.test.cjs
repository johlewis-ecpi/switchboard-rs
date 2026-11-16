const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const path=require('node:path');
const {dom,find,environment,open,settle,executeBundle}=require('./helpers.cjs');
test('standalone build boots, safely renders, filters, wraps, and reports parse failures',async()=>{
  const {build}=await import('../scripts/build.mjs');const directory=await build();const html=fs.readFileSync(path.join(directory,'index.html'),'utf8');
  const document=dom(),env=environment();executeBundle(html,env,document);
  assert.equal(env.messages[0].type,'switchboard:ready');
  env.dispatch(open(1,fs.readFileSync(path.join(__dirname,'../samples/edge-cases.sbview'),'utf8')));await settle();
  assert.equal(env.messages.at(-1).type,'switchboard:rendered');assert(document.nodes.content.textContent.includes('<img src=x'));
  assert(document.nodes.content.textContent.includes('</script>'));assert.equal(document.nodes.error.hidden,true);
  const article=document.nodes.content;const search=find(article,node=>node.type==='search');
  search.value='Unicode';search.dispatch('input');assert(document.nodes.content.textContent.includes('1 of 7 rows'));
  search.value='no matching row';search.dispatch('input');assert(document.nodes.content.textContent.includes('No rows match'));
  const wrap=find(article,node=>node.type==='checkbox');wrap.checked=false;wrap.dispatch('change');assert(find(article,node=>node.tagName==='TABLE').classList.contains('no-wrap'));
  env.dispatch(open(2,'not JSON'));await settle();assert.equal(env.messages.at(-1).type,'switchboard:error');assert.equal(document.nodes.error.hidden,false);assert.equal(document.nodes.content.children.length,0);
  env.dispatch(open(3,fs.readFileSync(path.join(__dirname,'../samples/empty.sbview'),'utf8')));await settle();assert(document.nodes.content.textContent.includes('no rows'));assert.equal(document.nodes.error.hidden,true);
});
test('manifest and package validation enforce installer constraints and reproducible builds',async()=>{
  const {build}=await import('../scripts/build.mjs');const {validateManifest,validatePackage}=await import('../scripts/manifest.mjs');
  const manifest=JSON.parse(fs.readFileSync(path.join(__dirname,'../reader.json'),'utf8'));
  for(const patch of [{apiVersion:2},{id:'../escape'},{extensions:['.txt']},{extensions:['txt','TXT']},{extensions:[]},{entry:'../outside.html'},{unknown:true}])assert.throws(()=>validateManifest({...manifest,...patch}));
  const directory=await build();const before=fs.readFileSync(path.join(directory,'index.html'),'utf8');await build();assert.equal(fs.readFileSync(path.join(directory,'index.html'),'utf8'),before);
  const checked=await validatePackage(directory);assert.equal(checked.manifest.id,manifest.id);assert(checked.html.includes('MIT License'));assert(checked.bytes<16*1024*1024);
});
test('validator permits embedded images but rejects external assets and escaping entries',async()=>{
  const os=require('node:os');const {validatePackage}=await import('../scripts/manifest.mjs');
  const temporary=fs.mkdtempSync(path.join(os.tmpdir(),'viewer-package-'));
  try {
    const manifest=JSON.parse(fs.readFileSync(path.join(__dirname,'../reader.json'),'utf8'));
    fs.writeFileSync(path.join(temporary,'reader.json'),JSON.stringify(manifest));
    fs.writeFileSync(path.join(temporary,'index.html'),'<html><img src="data:image/png;base64,AA=="></html>');
    await validatePackage(temporary);
    fs.writeFileSync(path.join(temporary,'index.html'),'<html><img src="https://example.invalid/image.png"></html>');
    await assert.rejects(validatePackage(temporary),/Image assets/);
    fs.writeFileSync(path.join(temporary,'index.html'),'<script src="reader.js"></script>');
    await assert.rejects(validatePackage(temporary),/self-contained/);
    if(process.platform!=='win32'){
      const alias=temporary+'-link';fs.symlinkSync(temporary,alias);
      try{await assert.rejects(validatePackage(alias),/symlink/);}finally{fs.unlinkSync(alias);}
    }
  } finally {fs.rmSync(temporary,{recursive:true,force:true});}
});
