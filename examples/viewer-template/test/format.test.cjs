const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const path=require('node:path');
const {parseReport,limits}=require('../src/format.js');
const sample=name=>fs.readFileSync(path.join(__dirname,'../samples',name+'.sbview'),'utf8');
const minimal=()=>({schemaVersion:1,title:'Report',columns:[{key:'name',label:'Name'}],rows:[]});
test('normalizes samples, Unicode, nulls, missing values and literal markup',()=>{
  const report=parseReport(sample('welcome'));assert.equal(report.rows.length,3);assert.equal(report.rows[0][3],'65');assert(Object.isFrozen(report));
  const edge=parseReport(sample('edge-cases'));assert(edge.rows.some(row=>row[1].includes('<img')));assert(edge.rows.some(row=>row[1].includes('</script>')));
  assert(edge.rows.some(row=>row[1]==='true'));assert.equal(edge.rows.filter(row=>row[1]==='').length,2);
  assert.equal(parseReport(sample('empty')).rows.length,0);assert.throws(()=>parseReport(sample('invalid')),/schemaVersion/);
  assert.equal(parseReport('\uFEFF'+JSON.stringify(minimal())).title,'Report');
});
test('rejects malformed input, unsupported schemas, unsafe keys and invalid cells',()=>{
  for(const text of ['','{','[]','null'])assert.throws(()=>parseReport(text));
  const cases=[{...minimal(),schemaVersion:2},{...minimal(),unknown:true},{...minimal(),description:null},{...minimal(),title:''},{...minimal(),columns:[{key:'constructor',label:'Unsafe'}]},
    {...minimal(),columns:[{key:'a',label:'A'},{key:'a',label:'B'}]}, {...minimal(),rows:[{other:'field'}]}, {...minimal(),rows:[{name:{nested:true}}]}];
  for(const value of cases)assert.throws(()=>parseReport(JSON.stringify(value)));
  assert.throws(()=>parseReport('{"schemaVersion":1,"title":"X","columns":[{"key":"n","label":"N"}],"rows":[{"n":1e999}]}'),/finite/);
});
test('enforces resource limits and permits Unicode at the character boundary',()=>{
  assert.throws(()=>parseReport('x'.repeat(limits.characters+1)),/limit/);
  assert.throws(()=>parseReport(JSON.stringify({...minimal(),rows:Array(limits.rows+1).fill({})})),/2000/);
  assert.throws(()=>parseReport(JSON.stringify({...minimal(),columns:Array.from({length:25},(_,i)=>({key:'c'+i,label:'C'}))})),/24/);
  assert.throws(()=>parseReport(JSON.stringify({...minimal(),rows:[{name:'x'.repeat(limits.cell+1)}]})),/4096/);
  assert.equal(parseReport(JSON.stringify({...minimal(),title:'✓'.repeat(200)})).title.length,200);
});
