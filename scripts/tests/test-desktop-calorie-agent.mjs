import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
import { webcrypto } from 'node:crypto';
import { MealAgentClient, revisionStorage } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/agent-client.mjs';
import { checkedResult, calorieRange, mealRecord, recognitionPrompt } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs';

const food = { name: 'Rice', grams: 150, kcalPer100gMin: 120, kcalPer100gMax: 150, basis: 'model_estimate', sources: [] };
const result = () => ({ items: [{ ...food }], assumptions: ['Cooking oil is unknown'], questions: ['How much rice?'] });
function memoryStorage() {
  const values = new Map();
  return { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: (key) => values.delete(key) };
}
function memoryDrafts() {
  let request = null;
  return { async get() { return request; }, async put(value) { request = value; }, async delete() { request = null; } };
}
const state = (sequence, status = 'running') => ({ runId: 'run-1', state: status, sequence, updatedAt: 5, result: { text: 'done', output: result(), attachments: [] } });

test('edits scale estimates without silently replacing model assumptions or provenance', () => {
  const checked = checkedResult(result());
  assert.deepEqual(calorieRange(checked.items), { min: 180, max: 225 });
  const record = mealRecord(checked, [{ name: 'Cooked rice', grams: '200' }], 'run-1', new Date('2026-10-23T12:00:00Z'));
  assert.deepEqual(record.range, { min: 240, max: 300 });
  assert.equal(record.items[0].estimatedGrams, 150);
  assert.equal(record.items[0].estimatedName, 'Rice');
  assert.equal(record.items[0].basis, 'model_estimate');
  assert.equal(record.reviewedByUser, true);
  assert.equal(checked.items[0].grams, 150);
  assert.equal('image' in record, false);
});

test('inverted ranges, invalid edited portions, and unearned database provenance are rejected', () => {
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, kcalPer100gMin: 200 }] }));
  for (const grams of ['', 0, -1, Infinity, 'unknown', 10001]) {
    assert.throws(() => mealRecord(result(), [{ name: 'Rice', grams }], 'run-1'));
  }
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, basis: 'nutrition_source', sources: ['invented database'] }] }));
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, sources: ['invented database'] }] }));
  assert.match(recognitionPrompt(), /no tools/);
  assert.match(recognitionPrompt(), /do not invent/);
});

test('empty food result is explainable but cannot be saved as a meal', () => {
  const empty = checkedResult({ items: [], assumptions: [], questions: ['Please photograph food.'] });
  assert.throws(() => mealRecord(empty, [], 'run-1'));
});

test('a lost task admission retries the same durable File request after page reload', async () => {
  const storage = memoryStorage();
  const drafts = memoryDrafts();
  const inputs = [];
  const request = { requestId: 'request-1', prompt: 'meal', attachments: [{ name: 'meal.jpg', type: 'image/jpeg', size: 5 }] };
  let admitted = false;
  const agent = {
    tasks: {
      async start(input) { inputs.push(input); if (inputs.length === 1) throw new Error('IPC disconnected'); admitted = true; return { runId: 'run-1' }; },
      subscribe() { return () => {}; }, async get() { return state(1, 'succeeded'); },
      async list() { return admitted ? [state(1, 'succeeded')] : []; },
    },
  };
  const first = new MealAgentClient(agent, storage, () => {}, drafts);
  await assert.rejects(first.submit(request));
  const reloaded = new MealAgentClient(agent, storage, () => {}, drafts);
  assert.equal(await reloaded.resume(), null);
  assert.equal(inputs.length, 1, 'refresh never silently retries an unconfirmed request');
  await assert.rejects(reloaded.submit({ requestId: 'must-not-replace', prompt: 'new photo must not silently replace the original' }), /待恢复/);
  await reloaded.submit(request);
  assert.deepEqual(inputs, [request, request]);
  assert.equal(reloaded.pending, null, 'temporary file payload is removed after host acceptance');
  const third = new MealAgentClient(agent, storage, () => {}, drafts);
  await third.resume();
  assert.equal(inputs.length, 2, 'accepted runs reconnect through get/subscribe, never run again');
});

test('a definitive host rejection clears the app-owned prompt and File draft', async () => {
  const storage = memoryStorage();
  const drafts = memoryDrafts();
  const agent = {tasks: {async start() {
    const error = new Error('Agent access is denied.');
    error.definitive = true;
    throw error;
  }}};
  const client = new MealAgentClient(agent, storage, () => {}, drafts);
  await assert.rejects(client.submit({requestId: 'rejected-1', prompt: 'meal', attachments: [{name: 'meal.jpg', size: 5}]}), /denied/);
  assert.equal(await drafts.get(), null);
  assert.equal(client.pending, null);
  assert.equal(storage.getItem('meal-notes.agent.pending'), null);
});

test('expired photo drafts clear in memory and cannot be resubmitted after the retention window', async () => {
  const storage = memoryStorage();
  storage.setItem('meal-notes.agent.pending', 'expired-1');
  const drafts = {expired: true, async get() { return null; }, async put() {}, async delete() {}};
  const received = [];
  let starts = 0;
  const client = new MealAgentClient({tasks: {async start() { starts++; return {runId: 'run-1'}; }}}, storage,
    (event) => received.push(event), drafts);
  client.pending = {requestId: 'expired-1', prompt: 'private', attachments: []};
  assert.equal(await client.checkDraftExpiry(), true);
  assert.equal(client.pending, null);
  assert.equal(storage.getItem('meal-notes.agent.pending'), null);
  assert.deepEqual(received, [{type: 'draft-expired'}]);

  await assert.rejects(client.submit({requestId: 'expired-1', prompt: 'private', attachments: []}),
    (error) => error.code === 'draft_expired');
  assert.equal(starts, 0, 'expired bytes and request IDs are never replayed');
});

test('a crash after task admission removes the matching local File draft on resume', async () => {
  const storage = revisionStorage(memoryStorage(), 1);
  storage.setItem('meal-notes.agent.run', 'run-1');
  const drafts = memoryDrafts();
  await drafts.put({requestId: 'request-1', prompt: 'meal', attachments: []});
  const task = {...state(1, 'succeeded'), requestId: 'request-1'};
  let starts = 0;
  const agent = {tasks: {
    async start() { starts++; },
    async get() { return task; },
    subscribe() { return () => {}; },
  }};
  const client = new MealAgentClient(agent, storage, () => {}, drafts);
  assert.equal(await client.resume(), 'run-1');
  assert.equal(await drafts.get(), null);
  assert.equal(starts, 0, 'resume observes the admitted task without resubmitting');
});

test('an unaccepted draft is preserved for explicit same-request retry and cannot be cancelled as a task', async () => {
  const storage = memoryStorage();
  const drafts = memoryDrafts();
  const inputs = [];
  const firstPhoto = { requestId: 'photo-1', prompt: 'meal', attachments: [{ name: 'first.jpg', type: 'image/jpeg', size: 5 }] };
  let granted = false;
  const agent = {
    tasks: {
      async start(input) { inputs.push(input); if (!granted) throw new Error('Agent access has not been granted for this application.'); return { runId: 'run-1' }; },
      subscribe() { return () => {}; }, async get() { return state(1, 'succeeded'); }, async list() { return []; },
      async cancel() { assert.fail('cannot cancel before admission'); },
    },
  };
  const client = new MealAgentClient(agent, storage, () => {}, drafts);
  await assert.rejects(client.submit(firstPhoto));
  granted = true;
  await client.resume();
  assert.equal(inputs.length, 1, 'granting permission then refreshing must not run the rejected photo');
  await client.submit(firstPhoto);
  assert.deepEqual(inputs, [firstPhoto, firstPhoto]);
  assert.equal(client.pending, null);
});

test('replay/get race and duplicate events cannot overwrite a terminal state', async () => {
  const received = [];
  let listener;
  let unsubscribed = 0;
  const agent = {
    tasks: {
      subscribe(_id, callback) { listener = callback; callback({ type: 'state', runId: 'run-1', sequence: 4, task: state(4, 'succeeded') }); return () => { unsubscribed++; }; },
      async get() { return state(2); },
      async cancel(runId) { assert.equal(runId, 'run-1'); },
    },
  };
  const client = new MealAgentClient(agent, memoryStorage(), (event) => received.push(event), memoryDrafts());
  await client.watch('run-1');
  listener({ type: 'state', runId: 'other-app-run', sequence: 5, task: state(5) });
  listener({ type: 'state', runId: 'run-1', sequence: 4, task: state(4, 'succeeded') });
  assert.deepEqual(received.map((event) => event.task.state), ['succeeded']);
  await client.cancel();
  client.dispose();
  assert.equal(unsubscribed, 1);
});

test('storage failure prevents starting a task that cannot recover its request identity', async () => {
  let ran = false;
  const storage = { getItem() { return null; }, setItem() { throw new Error('QuotaExceededError'); } };
  const client = new MealAgentClient({ tasks: { async start() { ran = true; } } }, storage, () => {}, memoryDrafts());
  await assert.rejects(client.submit({ requestId: 'request-1', prompt: 'meal' }), /QuotaExceededError/);
  assert.equal(ran, false);
});

test('replacing a photo forgets only a completed run and refresh never restores its old result', async () => {
  const storage = memoryStorage();
  const drafts = memoryDrafts();
  let listener, reads = 0, unsubscribed = 0;
  const agent = {
    tasks: {
      async start() { return { runId: 'run-1' }; },
      subscribe(_id, callback) { listener = callback; return () => { unsubscribed++; }; },
      async get() { reads++; return state(1); },
      async list() { return []; },
    },
  };
  const seen = [];
  const client = new MealAgentClient(agent, storage, event => seen.push(event), drafts);
  await client.submit({ requestId: 'photo-1', prompt: 'meal' });
  assert.throws(() => client.forgetCompleted(), /恢复或等待/);
  listener({ type: 'state', runId: 'run-1', sequence: 2, task: state(2, 'succeeded') });
  client.forgetCompleted();
  assert.equal(client.runId, null);
  assert.equal(unsubscribed, 1);
  listener({ type: 'state', runId: 'run-1', sequence: 3, task: state(3, 'succeeded') });
  assert.equal(seen.length, 2, 'late events for the previous photo cannot update the UI');
  const refreshed = new MealAgentClient(agent, storage, () => assert.fail('old result restored'), drafts);
  assert.equal(await refreshed.resume(), null);
  assert.equal(reads, 1, 'choosing a new photo removes the old reconnect identity');
  await drafts.put({ requestId: 'unconfirmed', prompt: 'meal' });
  refreshed.pending = { requestId: 'unconfirmed' };
  assert.throws(() => refreshed.forgetCompleted(), /恢复或等待/);
});

test('version changes never restore or replay the previous version task or unconfirmed photo', async () => {
  const storage = memoryStorage();
  const previous = revisionStorage(storage, 1);
  previous.setItem('meal-notes.agent.run', 'previous-version-run');
  const agent = { tasks: { start() { assert.fail('previous photo replayed'); }, get() { assert.fail('previous version accessed'); }, list() { assert.fail('previous version accessed'); } } };
  const next = new MealAgentClient(agent, revisionStorage(storage, 2), () => assert.fail('previous result restored'), memoryDrafts());
  assert.equal(await next.resume(), null);
  assert.equal(next.pending, null);
  next.forgetCompleted();
  assert.equal(previous.getItem('meal-notes.agent.run'), 'previous-version-run');
  assert.throws(() => revisionStorage(storage, 0), /运行版本/);
});

test('schema-2 example uses the external-browser Agent API and keeps provider access in the host', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('lingxi-app.json', base), 'utf8'));
  assert.equal(manifest.schema, 2);
  assert.deepEqual(manifest.capabilities, ['agent', 'camera']);
  assert.ok(manifest.targets.some(target => target.platform === 'android'));
  assert.ok(manifest.targets.some(target => target.platform === 'ios'));
  assert.deepEqual(manifest.agent.tools, []);
  const source = await readFile(new URL('app.mjs', base), 'utf8');
  assert.match(source, /window\.localApp\?\.agent/);
  assert.match(source, /getUserMedia/);
  assert.match(source, /attachments: \[image\]/);
  assert.doesNotMatch(source, /fetch\(|api[_-]?key|window\.lingxi|innerHTML/i);
  assert.doesNotMatch(source, /images:|agent\.run\(/);
  const client = await readFile(new URL('agent-client.mjs', base), 'utf8');
  assert.match(client, /agent\.tasks\.start/);
  assert.match(client, /event\.task/);
  assert.match(client, /draftTtlMs = 24 \* 60 \* 60 \* 1000/);
  assert.match(client, /error\.definitive/);
});

test('attachment chat example keeps a retryable File draft and uses the packaged Skill and durable Chat APIs', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/attachment-chat-skill/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('lingxi-app.json', base), 'utf8'));
  assert.deepEqual(manifest.capabilities, ['agent']);
  assert.deepEqual(manifest.agent.tools, []);
  assert.deepEqual(manifest.agent.skills, [{ id: 'review', path: 'skills/review/SKILL.md' }]);
  const [source, skill] = await Promise.all([
    readFile(new URL('app.mjs', base), 'utf8'),
    readFile(new URL(manifest.agent.skills[0].path, base), 'utf8'),
  ]);
  for (const api of ['agent.chats.create', 'agent.chats.list', 'agent.chats.open',
    'agent.chats.archive', 'agent.chats.delete', 'handle.send', 'chat.history',
    'chat.subscribe', 'chat.cancel']) {
    assert.ok(source.includes(api), `example should demonstrate ${api}`);
  }
  assert.match(source, /attachments: attachments/);
  assert.match(source, /\? \['review'\] : \[\]/);
  assert.match(source, /pendingDraft\.requestId/);
  assert.match(source, /refresh will not send it automatically/i);
  assert.match(source, /localApp:recovery-unavailable/);
  assert.match(source, /draftTtlMs = 24 \* 60 \* 60 \* 1000/);
  assert.match(source, /error\.definitive/);
  assert.match(skill, /untrusted data/);
  assert.doesNotMatch(source, /fetch\(|api[_-]?key|OPENAI_API_KEY|window\.lingxi|innerHTML/i);
});

test('browser disconnection does not corrupt task sequence or replay an Agent request', async () => {
  let receive; let runs = 0; const events=[];
  const agent = {
    tasks: {
      start: async () => { runs++; return {runId:'run-1'}; },
      subscribe: (_id, listener) => { receive=listener; return () => {}; },
      get: async () => state(3),
    },
  };
  const client = new MealAgentClient(agent, memoryStorage(), event=>events.push(event), memoryDrafts());
  await client.watch('run-1');
  receive({type:'connection',runId:'run-1',error:{code:'host_unavailable',message:'paused'}});
  receive({type:'state',runId:'run-1',sequence:2,task:state(2)});
  receive({type:'state',runId:'run-1',sequence:4,task:state(4,'succeeded')});
  assert.equal(events.length,3);
  assert.equal(events[1].type,'connection'); assert.equal(events[2].task.state,'succeeded');
  assert.equal(client.lastSequence,4); assert.equal(runs,0);
});

test('browser SDK fails closed when its durable request ledger is unavailable', async (t) => {
  const sdkPath = new URL('../../../harness-runtime/crates/runtime/src/mobile/unified_local_apps/browser_sdk.js', import.meta.url);
  let source;
  try { source = await readFile(sdkPath, 'utf8'); }
  catch (error) {
    if (error.code === 'ENOENT') { t.skip('requires the sibling harness-runtime checkout'); return; }
    throw error;
  }
  source = source.replace('__LINGXI_APP_CONFIGURATION__', JSON.stringify({
    appId: 'app-test', name: 'Test', revision: 3, hostPlatform: 'android', runId: 'run-test', capabilities: ['agent'], services: []
  }));
  const events = [];
  const window = {
    top: null,
    isSecureContext: false,
    addEventListener() {},
    dispatchEvent(event) { events.push(event); return true; }
  };
  window.top = window;
  const document = {visibilityState: 'visible', addEventListener() {}};
  let fetches = 0;
  const context = {
    window, document, navigator: {},
    location: {origin: 'http://127.0.0.1:32123', href: 'http://127.0.0.1:32123/__run/run-test/index.html'},
    history: {state: null, replaceState() {}},
    crypto: webcrypto,
    TextEncoder,
    CustomEvent: class { constructor(type, init = {}) { this.type = type; this.detail = init.detail; } },
    fetch: async () => { fetches++; throw new Error('fetch should not happen'); },
    setInterval: () => 1,
    clearTimeout,
  };
  runInNewContext(source, context);
  const sdk = window.localApp;
  assert.equal(typeof sdk.agent.run, 'function');
  assert.equal(typeof sdk.agent.tasks.start, 'function');
  assert.equal(typeof sdk.agent.chats.open, 'function');
  assert.equal(typeof sdk.files.pick, 'function');
  assert.equal(sdk.agent.images, undefined);
  await assert.rejects(
    runInNewContext("window.localApp.agent.tasks.start({prompt:'must be persisted first'})", context),
    (error) => error.code === 'storage_unavailable'
  );
  assert.equal(fetches, 0, 'the host receives nothing when request identity and input cannot be durably saved');
  assert.ok(events.some((event) => event.type === 'localApp:recovery-unavailable'));
});
