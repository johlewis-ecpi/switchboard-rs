/* Composition root. Replace the parser/renderer, keep the host adapter. */
(function () {
  'use strict';
  const content = document.getElementById('content');
  const status = document.getElementById('status');
  const error = document.getElementById('error');
  SwitchboardViewer.createReader({
    extensions: /* SUPPORTED_EXTENSIONS */,
    parse: SampleReportFormat.parseReport,
    render: SampleReportRenderer.renderReport,
    onLoading() { error.hidden = true; content.replaceChildren(); status.textContent = 'Reading document…'; },
    mount(view) { content.replaceChildren(view); status.textContent = ''; },
    onError(message) { content.replaceChildren(); status.textContent = ''; error.textContent = message; error.hidden = false; }
  });
})();
