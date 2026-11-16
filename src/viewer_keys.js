// Injected by the host into every frame. Reader packages need no keyboard code.
(() => {
  'use strict';
  const eligible = event => {
    if (event.key !== ' ' || event.isComposing || event.metaKey || event.ctrlKey || event.altKey || event.shiftKey) return false;
    const target = event.composedPath?.()[0] || event.target || document.activeElement;
    if (target?.isContentEditable) return false;
    const control = target?.closest?.('input,textarea,select,button,a[href],summary,video,audio,[role="button"],[role="link"],[role="checkbox"],[role="switch"],[role="slider"],[role="spinbutton"],[contenteditable]:not([contenteditable="false"])');
    return !control || (window === window.top && control.matches?.('[data-entry]'));
  };
  window.switchboardViewerSpaceEligible = eligible;
  if (window === window.top) return;
  const forward = () => parent.postMessage({type:'switchboard:viewer-space',apiVersion:1},'*');
  window.addEventListener('keydown', event => {
    if (!eligible(event)) return;
    event.preventDefault();event.stopPropagation();
    if (!event.repeat) forward();
  }, true);
  // Relay from direct child frames only (the bundled HTML reader has one).
  window.addEventListener('message', event => {
    if (event.data?.type !== 'switchboard:viewer-space' || event.data.apiVersion !== 1) return;
    if ([...document.querySelectorAll('iframe')].some(frame => frame.contentWindow === event.source)) forward();
  });
})();
