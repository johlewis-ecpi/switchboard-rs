// Settings UI for the format-independent registry. No reader implementation lives here.
let viewerInventory = [], pendingViewerTest = null;
const viewerSettingsTitle = 'Settings → Viewers';
function viewerSettings(requestInventory = true) {
  modal(viewerSettingsTitle, '<p role="status">Checking installed viewers…</p>',
    '<button data-action="installViewer">Install…</button><button data-action="listViewers">Refresh</button>', true);
  if (requestInventory) send('listViewers');
}
function viewerManagerError(message) {
  if (dialogKind !== viewerSettingsTitle) return toast(message);
  let error = $('#viewerManagerError');
  if (!error) {
    error = document.createElement('p'); error.id = 'viewerManagerError'; error.className = 'reader-notice';
    error.setAttribute('role', 'alert'); $('#dialogs .body').prepend(error);
  }
  error.textContent = message;
}
function startupLabel(value) {
  return ({waiting:'Waiting for reader',passed:'Startup passed',opened:'Native view opened',failed:'Startup failed',notCompleted:'Not completed'})[value] || 'Not tested';
}
function renderingLabel(value) {
  return ({reported:'Reader reports success — visual check needed',confirmed:'Visually confirmed',failed:'Rendering failed'})[value] || 'Not verified';
}
function receiveViewerInventory({readers, directory}) {
  if (dialogKind !== viewerSettingsTitle) return;
  viewerInventory = readers;
  const rows = readers.map(reader => {
    const test = reader.lastTest;
    const controls = `<button data-viewer-op="test" data-viewer-key="${attr(reader.key)}" ${reader.validationError?'disabled':''}>Test with Sample…</button>
      <button data-viewer-op="toggle" data-viewer-key="${attr(reader.key)}" ${!reader.enabled&&reader.validationError?'disabled':''}>${reader.enabled?'Disable':'Enable'}</button>
      ${reader.source==='installed'?`<button class="danger" data-viewer-op="remove" data-viewer-key="${attr(reader.key)}">Remove…</button>`:''}`;
    return `<section class="viewer-row"><div class="row"><h3>${esc(reader.name)}</h3><small>${esc(reader.version || 'Version not declared')}</small><span class="spacer"></span><span class="chip">${reader.enabled?'Enabled':'Disabled'}</span></div>
      <p class="hint">${esc(({system:'System adapter',bundled:'Bundled reader',installed:'Installed reader'})[reader.source])}${reader.id?' · '+esc(reader.id):''}</p>
      <p class="viewer-formats">${reader.extensions.length?reader.extensions.map(ext=>'<span class="chip">.'+esc(ext)+'</span>').join(''):'Text/code and source fallback'}</p>
      <dl class="viewer-checks"><dt>Validation</dt><dd>${reader.validationError?`<span class="viewer-failure">${esc(reader.validationError)}</span>`:'Valid'}</dd>
      <dt>Startup</dt><dd>${esc(test?.startup==='waiting'?'Not completed':startupLabel(test?.startup))}</dd><dt>Rendering</dt><dd>${esc(renderingLabel(test?.rendering))}</dd></dl>
      ${reader.conflicts.length?`<p class="viewer-failure">Conflicts with ${esc(reader.conflicts.join(', '))}. Disable one to resolve.</p>`:''}
      ${test?`<p class="hint">Last test: ${esc(new Date(test.checkedAt).toLocaleString())} · ${esc(test.sampleName)}${test.sampleExtension?' (.'+esc(test.sampleExtension)+')':''}</p><p class="hint">${esc(test.detail)}</p>`:'<p class="hint">No sample test recorded.</p>'}
      ${reader.testStale?'<p class="viewer-failure">Viewer changed since this test. Test again; these results apply to the previous version.</p>':''}
      ${reader.path?`<p class="readonly viewer-path">${esc(reader.path)}</p>`:''}<div class="row viewer-controls">${controls}</div></section>`;
  }).join('');
  $('#dialogs .body').innerHTML = `<nav class="row settings-tabs" aria-label="Settings"><button data-ui="settings">Quick Launcher</button><button aria-current="page" disabled>Viewers</button></nav>
    <p class="hint">Validation checks the package. Startup checks that it responds. Rendering requires checking a sample visually; results apply only to that sample.</p>
    <div class="viewer-list">${rows}</div><p class="hint viewer-path">Install location: ${esc(directory)}</p>`;
}
function requestViewerTest(key) {
  pendingViewerTest = ++viewerSequence;
  send('testViewer', {key, request:pendingViewerTest});
}
function receiveViewerSample({request, document:doc, key, name, result}) {
  if (request !== pendingViewerTest || dialogKind !== viewerSettingsTitle) {
    send('closeViewer', {request}); return;
  }
  pendingViewerTest = null;
  createDocumentViewer(name, null, request);
  documentViewer.testKey = key;
  $('#dialogs .modal footer').innerHTML = `<span id="viewerTestStatus" class="hint" role="status" aria-live="polite"></span><span class="spacer"></span>
    <button id="confirmViewerRendering" data-viewer-verdict="pass" disabled>Confirm Rendering</button>
    <button data-viewer-verdict="fail">Report Problem</button><button data-ui="viewers">Back to Viewers</button>`;
  receiveDocument({request, document:doc});
  receiveViewerTestStatus({request, result});
}
function receiveViewerTestStatus({request, result}) {
  if (!documentViewer?.testKey || documentViewer.request !== request) return;
  documentViewer.testResult = result;
  const label = $('#viewerTestStatus');
  if (label) label.textContent = `${startupLabel(result.startup)} · ${renderingLabel(result.rendering)}. ${result.detail}`;
  const button = $('#confirmViewerRendering');
  if (button) button.disabled = documentViewer.failed || !['passed','opened'].includes(result.startup);
}
function removeViewerPrompt(key) {
  const reader = viewerInventory.find(reader=>reader.key===key);
  if (!reader || reader.source !== 'installed') return;
  modal('Remove viewer?', `<p>Remove ${esc(reader.name)}?</p><p class="hint">This removes its installed package and test history. Source documents and the original package are preserved.</p><p class="readonly">${esc(reader.path)}</p>`,
    '<button data-ui="viewers">Cancel</button><button class="danger" id="confirmRemoveViewer">Remove Viewer</button>');
  $('#confirmRemoveViewer').onclick = () => { viewerSettings(false); send('removeViewer', {key}); };
}
document.addEventListener('click', event => {
  const button = event.target.closest('button');
  if (!button) return;
  const key = button.dataset.viewerKey;
  if (button.dataset.viewerOp === 'test') requestViewerTest(key);
  if (button.dataset.viewerOp === 'toggle') {
    const reader = viewerInventory.find(reader=>reader.key===key);
    if (reader) send('setViewerEnabled', {key, enabled:!reader.enabled});
  }
  if (button.dataset.viewerOp === 'remove') removeViewerPrompt(key);
  if (button.dataset.viewerVerdict && documentViewer?.testKey) {
    send('viewerTestVerdict', {request:documentViewer.request, passed:button.dataset.viewerVerdict==='pass'});
  }
});

// Native smoke-test hooks exercise the real WKWebView and sandboxed reader frame.
async function waitForViewerInventory() {
  for (let attempt = 0; attempt < 100; attempt++) {
    if ($('#dialogs .viewer-row') && viewerInventory.some(reader=>reader.key==='builtin:text')) return;
    await new Promise(resolve=>setTimeout(resolve,50));
  }
  throw Error('Viewer inventory did not load');
}
async function smokeViewerProtocol(doc) {
  createDocumentViewer('Viewer smoke sample');
  const request = documentViewer.request;
  try {
    await new Promise((resolve,reject)=>{
      const timer = setTimeout(()=>{window.removeEventListener('message',check);reject(Error('Sandboxed reader did not render the sample'));},16000);
      function check(event) {
        if (event.source===documentViewer?.frame?.contentWindow && event.data?.type==='switchboard:rendered' && event.data.request===request) {
          clearTimeout(timer); window.removeEventListener('message',check); resolve();
        }
      }
      window.addEventListener('message',check);
      receiveDocument({request, document:doc});
    });
  } finally { closeDialog(); }
}
