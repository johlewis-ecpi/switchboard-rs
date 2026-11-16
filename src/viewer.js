// Viewer host only. Format-specific code belongs in independently built reader packages.
let documentViewer = null, viewerSequence = 0;
const VIEWER_MAX_RANGE = 8 * 1024 * 1024, VIEWER_MAX_PENDING = 8;
function viewDocument(id = selected()[0]) {
  const entry = entity(id);
  if (!entry || entry.kind !== 'file') return toast('Select one local file to view');
  if (!window.switchboardPreviewWindow) {
    hideHover();
    send('openViewerWindow', {id});
    return;
  }
  createDocumentViewer(entry.name, id);
  send('viewDocument', {id, request: documentViewer.request});
}
function createDocumentViewer(name, id = null, request = ++viewerSequence) {
  hideHover();
  modal('Content viewer', '<p role="status">Loading document…</p>', '', true);
  documentViewer = {id, request, name};
  const modalElement = $('#dialogs .modal');
  modalElement.classList.add('document-viewer');
  modalElement.querySelector('header').innerHTML = `<h2>${esc(name)}</h2><small id="readerName">Loading…</small>`;
  modalElement.querySelector('footer').innerHTML = `${id?`<button data-open="${attr(id)}">Open Externally</button>`:''}<button data-ui="closeDialog">Close</button>`;
}

function handleViewerSpace(event) {
  if (!window.switchboardViewerSpaceEligible?.(event) || data?.launcher || (dialogKind && dialogKind !== 'Content viewer')) return false;
  event.preventDefault();
  if (event.repeat) return true;
  if (documentViewer) closeDialog();
  else if (!dialogKind) viewDocument();
  return true;
}

function disposeDocumentViewer() {
  if (!documentViewer) return;
  documentViewer.observer?.disconnect();
  clearTimeout(documentViewer.timer);
  const request = documentViewer.request;
  documentViewer = null;
  send('closeViewer', {request});
}
function layoutDocumentViewer() {
  if (!documentViewer?.native) return;
  const host = $('#readerHost');
  if (!host) return;
  const r = host.getBoundingClientRect();
  send('layoutViewer', {request: documentViewer.request, rect: [r.x, r.y, r.width, r.height]});
}
function readerFailure(message) {
  if (!documentViewer) return;
  clearTimeout(documentViewer.timer);
  documentViewer.observer?.disconnect();
  documentViewer.native = false;
  documentViewer.frame = null;
  documentViewer.failed = true;
  if (documentViewer.testKey) send('viewerTestEvent', {request:documentViewer.request,event:'error',message});
  $('#dialogs .body').innerHTML = `<p role="alert">${esc(message)}</p><p class="hint">${documentViewer.testKey?'Return to Viewers to review the result or try another sample.':'Use Open Externally, or close and retry.'}</p>`;
}
function receiveDocument({request, document: doc, error}) {
  if (!documentViewer || request !== documentViewer.request || dialogKind !== 'Content viewer') return;
  if (error) return readerFailure(error);
  documentViewer.doc = doc;
  $('#readerName').textContent = doc.readerName;
  const body = $('#dialogs .body');
  body.innerHTML = `${doc.notice ? `<p class="reader-notice" role="status">${esc(doc.notice)}</p>` : ''}<div id="readerHost" tabindex="0" aria-label="Document content"></div>`;
  const host = $('#readerHost');
  if (doc.renderer === 'native') {
    documentViewer.native = true;
    documentViewer.observer = new ResizeObserver(layoutDocumentViewer);
    documentViewer.observer.observe(host);
    layoutDocumentViewer();
  } else if (doc.renderer === 'package' || doc.renderer === 'html') {
    // Opaque-origin iframe: package code gets document text, never paths or host IPC.
    const frame = document.createElement('iframe');
    frame.title = doc.readerName;
    frame.setAttribute('sandbox', 'allow-scripts');
    frame.referrerPolicy = 'no-referrer';
    documentViewer.frame = frame;
    const policy = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'";
    // Only the host-selected bundled HTML adapter receives a child-frame
    // capability. Its document frame keeps a separate origin and stricter CSP.
    const readerPolicy = doc.renderer === 'html' ? policy.replace("frame-src 'none'", "frame-src about:") : policy;
    frame.srcdoc = `<!doctype html><meta http-equiv="Content-Security-Policy" content="${readerPolicy}">` + doc.readerHtml;
    documentViewer.timer = setTimeout(() => {
      if (documentViewer?.request === request) readerFailure('Reader did not signal readiness within 15 seconds');
    }, 15000);
    host.append(frame);
  } else {
    readerFailure('No compatible reader package is available');
    return;
  }
  host.focus();
}
// API v2 (binary range protocol): the reader never receives file contents up front. It requests byte ranges,
// which the host validates against the document size and serves from the file (see readViewerRange in Rust).
function viewerBytes(base64) {
  if (Uint8Array.fromBase64) return Uint8Array.fromBase64(base64);
  const binary = atob(base64), bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
function receiveViewerRange({request, id, data, error}) {
  const viewer = documentViewer;
  if (!viewer?.frame || request !== viewer.request || !viewer.pending?.has(id)) return;
  const length = viewer.pending.get(id);
  viewer.pending.delete(id);
  const target = viewer.frame.contentWindow;
  if (error || typeof data !== 'string') {
    target.postMessage({type:'switchboard:range-error', apiVersion:2, request, id, message:String(error || 'The host could not read the file').slice(0, 500)}, '*');
    return;
  }
  try {
    const bytes = viewerBytes(data);
    if (bytes.length !== length) throw new Error('The host returned an unexpected range length');
    target.postMessage({type:'switchboard:range', apiVersion:2, request, id, data:bytes.buffer}, '*', [bytes.buffer]);
  } catch (failure) {
    target.postMessage({type:'switchboard:range-error', apiVersion:2, request, id, message:String(failure.message).slice(0, 500)}, '*');
  }
}
function viewerReadRange(viewer, message) {
  const {id, offset, length} = message, size = viewer.doc.size;
  const reject = text => viewer.frame.contentWindow.postMessage({type:'switchboard:range-error', apiVersion:2, request:viewer.request, id, message:text}, '*');
  if (!Number.isSafeInteger(id) || id < 0 || viewer.pending.has(id)) return;
  if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(length) || offset < 0 || length < 0 || length > VIEWER_MAX_RANGE || offset + length > size) return reject('Invalid range');
  if (viewer.pending.size >= VIEWER_MAX_PENDING) return reject('Too many outstanding reads');
  if (length === 0) return viewer.frame.contentWindow.postMessage({type:'switchboard:range', apiVersion:2, request:viewer.request, id, data:new ArrayBuffer(0)}, '*');
  viewer.pending.set(id, length);
  send('readViewerRange', {request:viewer.request, id, offset, length});
}
window.addEventListener('message', event => {
  const viewer = documentViewer;
  if (!viewer?.frame || event.source !== viewer.frame.contentWindow) return;
  const message = event.data;
  const version = viewer.doc?.apiVersion === 2 ? 2 : 1;
  if (!message || (message.apiVersion !== version && message.type !== 'switchboard:viewer-space')) return;
  if (message.type === 'switchboard:viewer-space') { if (message.apiVersion === 1) closeDialog(); return; }
  if (message.type === 'switchboard:ready' && !viewer.ready) {
    viewer.ready = true;
    clearTimeout(viewer.timer);
    if (viewer.testKey) send('viewerTestEvent', {request:viewer.request,event:'ready'});
    const doc = version === 2
      ? {name:viewer.name, extension:viewer.doc.extension, size:viewer.doc.size, readOnly:true}
      : {name:viewer.name, extension:viewer.doc.extension, text:viewer.doc.text, readOnly:true};
    if (version === 2) viewer.pending = new Map();
    viewer.frame.contentWindow.postMessage({type:'switchboard:open', apiVersion:version, request:viewer.request, document:doc}, '*');
  } else if (message.type === 'switchboard:read' && version === 2 && viewer.ready && message.request === viewer.request) {
    viewerReadRange(viewer, message);
  } else if (message.type === 'switchboard:rendered' && viewer.ready && message.request === viewer.request) {
    if (viewer.testKey) send('viewerTestEvent', {request:viewer.request,event:'rendered'});
  } else if (message.type === 'switchboard:error' && message.request === viewer.request) {
    readerFailure(String(message.message || 'Reader could not display this document').slice(0, 2000));
  }
});
window.addEventListener('resize', layoutDocumentViewer);
/* END_VIEWER_MODULE */
