// Run await verifyAuditUi() in the generated fixture's DevTools console.
// Real keyboard behavior is checked separately with Chrome DevTools input.
export async function verifyAuditUi() {
  const frame = () => new Promise(resolve => requestAnimationFrame(resolve));
  const assert = (condition, message) => { if (!condition) throw new Error(message); };
  const wait = async condition => {
    for (let attempt = 0; attempt < 240; attempt++) { if (condition()) return; await frame(); }
    throw new Error('Audit UI condition timed out');
  };
  const button = (text, root = document) => [...root.querySelectorAll('button')].find(element => element.textContent.trim() === text);
  const click = async text => { const element = button(text); assert(element && !element.disabled, `Unavailable button: ${text}`); element.focus(); element.click(); await frame(); };
  const field = async (element, value) => { element.value = value; element.dispatchEvent(new Event(element.tagName === 'SELECT' ? 'change' : 'input', { bubbles: true })); await frame(); };
  const save = async () => { document.querySelector('dialog form').requestSubmit(); await wait(() => !document.querySelector('dialog')); };
  const lastConfig = () => audit.requests.findLast(item => item.args?.request?.op === 'save_config').args.request.config;

  await wait(() => button('Edit'));
  await click('Edit');
  await field(document.querySelector('dialog input'), '  Audit documents  ');
  await save();
  const profile = lastConfig().profiles[0];
  assert(profile.sources[0] === '/audit/Documents ' && profile.excludes[0] === ' **/draft ' && profile.label === 'Audit documents', 'Profile literals or label changed');
  await click('Repositories'); await click('Edit'); await save();
  assert(lastConfig().repositories[0].location === '/audit/repository ', 'Existing repository location changed');
  await click('Add repository');
  const inputs = document.querySelectorAll('dialog input');
  await field(inputs[0], '  New repository  '); await field(inputs[1], ' /audit/new repository '); await save();
  const repository = lastConfig().repositories.at(-1);
  assert(repository.label === 'New repository' && repository.location === ' /audit/new repository ', 'Repository request whitespace changed');

  await click('Settings');
  const theme = document.querySelector('.short-field select');
  const luminance = rgb => {
    const channels = rgb.match(/[\d.]+/g).slice(0, 3).map(Number).map(value => { value /= 255; return value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4; });
    return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722;
  };
  const contrast = (a, b) => (Math.max(luminance(a), luminance(b)) + .05) / (Math.min(luminance(a), luminance(b)) + .05);
  const contrasts = [];
  for (const name of ['light', 'dark', 'system']) {
    await field(theme, name);
    const style = getComputedStyle(theme);
    const fill = contrast(style.borderTopColor, style.backgroundColor);
    const page = contrast(style.borderTopColor, getComputedStyle(document.querySelector('.app')).backgroundColor);
    assert(fill >= 3 && page >= 3, `Control contrast failed: ${name}`);
    contrasts.push({ theme: name, fill, page });
  }

  await click('Snapshots');
  assert(document.body.innerText.includes('No repository selected'), 'Missing empty selection state');
  await field(document.querySelector('select'), 'local');
  audit.holdNextStatus(); await wait(() => audit.statusHeld);
  await click('Load snapshots');
  audit.holdNextStatus(); audit.releaseStatus(); await wait(() => audit.statusHeld);
  assert(button('Loading…')?.disabled && !document.querySelector('.snapshot-row'), 'Pre-admission status settled the query');
  audit.releaseStatus(); await wait(() => document.querySelector('.snapshot-row'));
  document.querySelector('.snapshot-row').click(); await frame();

  const startRestore = async target => {
    await click('Restore snapshot…');
    await field(document.querySelector('dialog input'), target);
    document.querySelector('dialog form').requestSubmit();
    await wait(() => document.querySelector('.jobbar') && document.querySelector('dialog input').disabled);
    assert(audit.requests.findLast(item => item.args?.request?.op === 'restore').args.request.target === target, 'Restore target whitespace changed');
  };
  await startRestore('/audit/restore ');
  await click('Close dialog');
  assert(!document.querySelector('dialog') && audit.status.active?.kind === 'restore', 'Dismissal stopped or trapped the restore');
  assert(audit.requests.every(item => item.args?.request?.op !== 'cancel'), 'Dismissal sent cancellation');
  assert(document.activeElement.textContent === 'Cancel operation', 'Dismissal lost keyboard focus');
  const outputView = () => document.querySelector('.jobbar .output-scroll');
  const atBottom = element => element.scrollHeight - element.clientHeight - element.scrollTop <= 2;
  assert(outputView().textContent === 'Waiting for restic output…', 'Missing honest waiting state');
  assert(!document.querySelector('.jobbar progress'), 'Unreliable percent bar is still primary activity');
  const payload = '<img src=x onerror="globalThis.auditInjected=true">';
  audit.appendOutput([...Array.from({ length: 198 }, (_, i) => `restoring file ${i}`), payload, `long path /${'directory/'.repeat(100)}`], true);
  await wait(() => outputView().textContent.includes(payload));
  await frame();
  assert(atBottom(outputView()), 'Initial output did not follow the tail');
  assert(!outputView().querySelector('img') && !globalThis.auditInjected, 'Output markup was interpreted as HTML');
  assert(outputView().scrollWidth <= outputView().clientWidth, 'Long output line did not wrap');
  assert(outputView().tabIndex === 0 && outputView().getAttribute('aria-label') === 'Command output', 'Output is not keyboard accessible');
  assert(document.querySelector('.output-retention')?.textContent.includes('200 lines / 64 KiB'), 'Missing truncation limits');
  audit.appendOutput(['appended while following']);
  await wait(() => outputView().textContent.includes('appended while following')); await frame();
  assert(atBottom(outputView()), 'Full output ring stopped following on append');
  outputView().scrollTop = 0; outputView().dispatchEvent(new Event('scroll')); await frame();
  await wait(() => button('Follow output'));
  audit.appendOutput(['appended while reading earlier output']);
  await wait(() => outputView().textContent.includes('appended while reading earlier output')); await frame();
  assert(outputView().scrollTop === 0 && button('Follow output'), 'Output stole scroll while reading');
  await click('Follow output'); await frame();
  assert(atBottom(outputView()) && !button('Follow output'), 'Follow output did not resume');
  assert(document.activeElement === outputView(), 'Follow output lost keyboard focus');
  outputView().scrollTop = 0; outputView().dispatchEvent(new Event('scroll')); await frame();
  await click('Cancel operation');
  await wait(() => !document.querySelector('.jobbar') && !button('Load snapshots').disabled);
  assert(audit.requests.filter(item => item.args?.request?.op === 'cancel').length === 1, 'Cancellation was not sent once');
  assert(document.querySelector('[role=alert]')?.textContent.includes('incomplete output'), 'Cancellation omitted incomplete-output warning');

  await click('Profiles');
  const historyDetails = document.querySelector('.history-output');
  assert(historyDetails && !historyDetails.open, 'Completed output was not collapsed');
  historyDetails.open = true; await frame();
  assert(historyDetails.querySelector('.output-scroll').textContent.includes(payload), 'Cancelled output was not retained');
  assert(document.querySelector('.activity-row').textContent.includes('cancelled'), 'Cancelled history state missing');
  // Failed jobs use the same history presentation, independent of cancellation.
  audit.status.history[0].state = 'failed';
  await wait(() => document.querySelector('.activity-row .badge')?.textContent === 'failed');
  assert(historyDetails.querySelector('.output-scroll').textContent.includes(payload), 'Failed output was not retained');
  await click('Snapshots');
  await field(document.querySelector('select'), 'local'); await click('Load snapshots');
  await wait(() => document.querySelector('.snapshot-row') && !button('Load snapshots').disabled);
  document.querySelector('.snapshot-row').click(); await frame();

  await startRestore('/audit/evicted ');
  document.querySelector('dialog .dialog-heading button').click(); await frame();
  assert(!document.querySelector('dialog') && audit.status.active?.kind === 'restore', 'Header dismissal failed');
  assert(!button('Follow output') && outputView().textContent === 'Waiting for restic output…', 'New job did not reset follow and output');
  audit.appendOutput(Array.from({ length: 200 }, (_, i) => `new job file ${i}`));
  await wait(() => outputView().textContent.includes('new job file 199')); await frame();
  assert(atBottom(outputView()), 'New job did not reset tail following');
  audit.evict();
  await wait(() => document.querySelector('[role=alert]')?.textContent.includes('no longer available'));
  assert(!button('Load snapshots').disabled, 'Evicted completion left the interface busy');
  await click('Load snapshots');
  await wait(() => document.querySelector('.snapshot-row') && !button('Load snapshots').disabled);
  assert(document.documentElement.scrollWidth <= innerWidth, 'Horizontal overflow');
  return { literalRequests: true, stalePollIgnored: true, dismissalAndCancellation: true, historyEvictionRecovery: true, commandOutput: { follow: true, pausedAppend: true, fullRingAppend: true, jobReset: true, literalHtml: true, wrapping: true, truncation: true, cancelledAndFailedHistory: true }, contrasts, viewport: [innerWidth, innerHeight] };
}
