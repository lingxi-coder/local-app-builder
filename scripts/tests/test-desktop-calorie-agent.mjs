import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { checkedResult, calorieRange, mealRecord, recognitionPrompt } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs';
import {
  applyUnsavedMealPortion,
  registerMealWebMcp,
  summarizeMealDraft,
  summarizeSavedMealRecords,
} from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/webmcp.mjs';

const food = { name: 'Rice', grams: 150, kcalPer100gMin: 120, kcalPer100gMax: 150, basis: 'model_estimate', sources: [] };
const result = () => ({ items: [{...food}], assumptions: ['Cooking oil is unknown'], questions: ['How much rice?'] });

class FakeModelContext {
  constructor() {
    this.origin = 'https://meal.example';
    this.window = {};
    this.registerCalls = [];
    this.registered = new Map();
  }
  async registerTool(tool, {signal} = {}) {
    assert.ok(signal instanceof AbortSignal);
    signal.throwIfAborted();
    assert.equal(this.registered.has(tool.name), false, `duplicate registration: ${tool.name}`);
    const entry = {tool, origin: this.origin, window: this.window, signal};
    this.registerCalls.push(tool.name);
    this.registered.set(tool.name, entry);
    signal.addEventListener('abort', () => {
      if (this.registered.get(tool.name) === entry) this.registered.delete(tool.name);
    }, {once: true});
  }
  async getTools() {
    return [...this.registered.values()].map(({tool, origin, window}) => ({
      name: tool.name,
      title: tool.title,
      description: tool.description,
      annotations: tool.annotations,
      inputSchema: tool.inputSchema,
      origin,
      window,
    }));
  }
  async executeTool(descriptor, input = {}, {signal} = {}) {
    signal?.throwIfAborted();
    const entry = this.registered.get(descriptor.name);
    assert.ok(entry, `tool is registered: ${descriptor.name}`);
    assert.equal(descriptor.origin, this.origin);
    const value = entry.tool.execute(input, {signal});
    return JSON.stringify(value);
  }
}

class FakeWindow {
  constructor() { this.listeners = new Map(); }
  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) ?? new Set();
    listeners.add(listener);
    this.listeners.set(type, listeners);
  }
  removeEventListener(type, listener) { this.listeners.get(type)?.delete(listener); }
  dispatch(type, event) { for (const listener of this.listeners.get(type) ?? []) listener(event); }
  listenerCount(type) { return this.listeners.get(type)?.size ?? 0; }
}

async function bootCalorieApp({agent, modelContext, storage, indexedDB, indexedDBGetter, warnings = []} = {}) {
  const base = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  let source = await readFile(new URL('app.mjs', base), 'utf8');
  source = source
    .replace(/^import .* from '\.\/nutrition\.mjs';\s*$/m, '')
    .replace(/^import .* from '\.\/webmcp\.mjs';\s*$/m, '');

  function findField(node, field) {
    if (node?.dataset?.field === field) return node;
    for (const child of node?.children ?? []) {
      const found = findField(child, field);
      if (found) return found;
    }
    return null;
  }
  class Element {
    constructor(id = '') {
      this.id = id;
      this.value = '';
      this.hidden = false;
      this.disabled = false;
      this.textContent = '';
      this.listeners = new Map();
      this.children = [];
      this.dataset = {};
    }
    addEventListener(type, listener) {
      const handlers = this.listeners.get(type) ?? new Set();
      handlers.add(listener);
      this.listeners.set(type, handlers);
    }
    dispatchEvent(event) {
      let result;
      for (const listener of this.listeners.get(event.type) ?? []) result = listener(event);
      return result ?? true;
    }
    click() { return this.dispatchEvent({type: 'click', target: this}); }
    replaceChildren(...children) { this.children = children; this.textContent = ''; }
    append(...children) { this.children.push(...children); }
    querySelectorAll(selector) {
      if (selector === '.food') return this.children.filter((child) => child.className === 'food');
      return [];
    }
    querySelector(selector) {
      const match = selector.match(/^\[data-field="([^"]+)"\]$/);
      return match ? findField(this, match[1]) : null;
    }
  }
  const elements = new Map();
  const document = {
    modelContext,
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
    createElement(tag) { return new Element(tag); },
  };
  const localStorage = storage ?? {
    value: '[]',
    length: 0,
    key: () => null,
    getItem() { return this.value; },
    setItem(_key, value) { this.value = value; },
    removeItem() {},
  };
  const sessionStorage = {length: 0, key: () => null, getItem: () => null, setItem() {}, removeItem() {}};
  const localApp = {
    info: async () => ({hostPlatform: 'macos'}),
    ...(agent ? {agent} : {}),
  };
  const window = new FakeWindow();
  window.localApp = localApp;
  const context = {localApp, localStorage, sessionStorage};
  if (indexedDBGetter) Object.defineProperty(context, 'indexedDB', {get: indexedDBGetter});
  else if (indexedDB !== undefined) context.indexedDB = indexedDB;
  const nutrition = await import('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs');
  const webmcp = await import('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/webmcp.mjs');
  const crypto = {randomUUID: (() => { let id = 0; return () => `meal-run-${++id}`; })()};
  const console = {warn: (...args) => warnings.push(args)};
  class File { constructor() { this.name = 'meal-photo.jpg'; this.type = 'image/jpeg'; this.size = 1; } }
  const url = {createObjectURL: () => 'blob:meal-photo', revokeObjectURL() {}};
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const runApp = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'sessionStorage', 'nutrition', 'webmcp', 'crypto', 'console', 'File', 'Event', 'URL',
    `const {nutritionSchema, recognitionPrompt, checkedResult, calorieRange, mealRecord} = nutrition;\n`
      + `const {applyUnsavedMealPortion, registerMealWebMcp, summarizeMealDraft, summarizeSavedMealRecords} = webmcp;\n`
      + `${source}\nreturn {document, window, click: (id) => document.getElementById(id).click(), prepareImage: () => showPhoto(new File()), state: () => ({result, completedRunId, busy})};`);
  const app = await runApp(context, document, window, localStorage, sessionStorage, nutrition, webmcp, crypto, console, File, Event, url);
  return {...app, storage: localStorage, modelContext, warnings};
}

async function waitFor(predicate, message) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail(message);
}

function successfulMealAgent() {
  return {
    async file() { return {type: 'image', source: {type: 'data', value: 'private-photo-bytes', mimeType: 'image/jpeg'}}; },
    createClient() {
      return {
        addMessage() {},
        abortRun() {},
        async runAgent(_input, subscriber) {
          subscriber.onRunFinishedEvent({outcome: 'success'});
          return {result: result(), newMessages: []};
        },
      };
    },
  };
}

test('edits scale estimates without silently replacing model assumptions or provenance', () => {
  const checked = checkedResult(result());
  assert.deepEqual(calorieRange(checked.items), {min: 180, max: 225});
  const record = mealRecord(checked, [{name: 'Cooked rice', grams: '200'}], 'run-1', new Date('2026-10-23T12:00:00Z'));
  assert.deepEqual(record.range, {min: 240, max: 300});
  assert.equal(record.items[0].estimatedGrams, 150);
  assert.equal(record.items[0].estimatedName, 'Rice');
  assert.equal(record.items[0].basis, 'model_estimate');
  assert.equal(record.reviewedByUser, true);
  assert.equal(checked.items[0].grams, 150);
  assert.equal('image' in record, false);
});

test('inverted ranges, invalid edited portions, and unearned database provenance are rejected', () => {
  assert.throws(() => checkedResult({...result(), items: [{...food, kcalPer100gMin: 200}]}));
  assert.throws(() => checkedResult({...result(), items: [{...food, name: 'x'.repeat(121)}]}));
  assert.throws(() => checkedResult({...result(), items: [{...food, sources: Array(9).fill('source')}]}));
  assert.throws(() => checkedResult({...result(), items: [{...food, sources: ['x'.repeat(501)]}]}));
  assert.throws(() => checkedResult({...result(), assumptions: Array(13).fill('assumption')}));
  assert.throws(() => checkedResult({...result(), assumptions: ['x'.repeat(501)]}));
  assert.throws(() => checkedResult({...result(), questions: Array(9).fill('question')}));
  assert.throws(() => checkedResult({...result(), questions: ['x'.repeat(501)]}));
  for (const grams of ['', 0, -1, Infinity, 'unknown', 10001]) {
    assert.throws(() => mealRecord(result(), [{name: 'Rice', grams}], 'run-1'));
  }
  assert.throws(() => checkedResult({...result(), items: [{...food, basis: 'nutrition_source', sources: ['invented database']}]}));
  assert.throws(() => checkedResult({...result(), items: [{...food, sources: ['invented database']}]}));
  assert.match(recognitionPrompt(), /no tools/);
  assert.match(recognitionPrompt(), /do not invent/);
});

test('empty food result is explainable but cannot be saved as a meal', () => {
  const empty = checkedResult({items: [], assumptions: [], questions: ['Please photograph food.']});
  assert.throws(() => mealRecord(empty, [], 'run-1'));
});

test('calorie example uses official AG-UI messages, ContentParts, results and lifecycle events', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('lingxi-app.json', base), 'utf8'));
  assert.equal(manifest.schema, 2);
  assert.deepEqual(manifest.capabilities, ['agent', 'camera']);
  assert.deepEqual(manifest.agent.tools, []);
  assert.ok(manifest.targets.some((target) => target.platform === 'android'));
  assert.ok(manifest.targets.some((target) => target.platform === 'ios'));
  assert.deepEqual(manifest.package.files, ['index.html', 'styles.css', 'app.mjs', 'nutrition.mjs', 'webmcp.mjs']);

  const source = await readFile(new URL('app.mjs', base), 'utf8');
  for (const api of ['agent.createClient({threadId: crypto.randomUUID()})', 'await agent.file(image)',
    "role: 'user'", "type: 'text'", 'runClient.addMessage', 'runClient.runAgent',
    "'com.lingxi.localApp'", 'outputSchema: nutritionSchema', 'onRunErrorEvent',
    'onRunFinishedEvent({outcome: terminalOutcome})', "outcome === 'cancelled'", 'runClient.abortRun()', 'mealRecord']) {
    assert.ok(source.includes(api), `calorie example should use ${api}`);
  }
  assert.doesNotMatch(source, /fetch\(|api[_-]?key|OPENAI_API_KEY|window\.lingxi|innerHTML/i);
});

test('WebMCP discovery and calls reuse meal logic, expose only bounded summaries, and leave Save to the user', async () => {
  const modelContext = new FakeModelContext();
  const windowRef = new FakeWindow();
  const checked = checkedResult(result());
  let draft = {result: checked, edits: [{name: 'Rice', grams: '150'}], canSave: true, busy: false};
  const savedRecord = {...mealRecord(checked, [{name: 'Rice', grams: '150'}], 'saved-run', new Date('2026-10-23T12:00:00Z')), photoBytes: 'must-not-leave-storage'};
  const savedRecords = [savedRecord];
  const calls = {historyReads: 0};
  const binding = registerMealWebMcp({
    documentRef: {modelContext},
    windowRef,
    getDraftSummary: () => summarizeMealDraft(draft),
    updateDraftPortion(input) {
      const change = applyUnsavedMealPortion(draft, input);
      draft = {...draft, edits: change.edits};
      return {updated: true, draft: change.summary};
    },
    listSavedRecords(limit) {
      calls.historyReads += 1;
      return summarizeSavedMealRecords(savedRecords, limit);
    },
  });
  assert.equal(binding.supported, true);
  assert.equal(await binding.ready, true);

  const tools = await modelContext.getTools();
  assert.deepEqual(tools.map((tool) => tool.name), [
    'meal_get_draft_summary', 'meal_list_saved_records', 'meal_update_unsaved_portion',
  ]);
  const byName = Object.fromEntries(tools.map((tool) => [tool.name, tool]));

  const draftSummary = JSON.parse(await modelContext.executeTool(byName.meal_get_draft_summary, {}));
  assert.equal(draftSummary.available, true);
  assert.deepEqual(draftSummary.calorieRange, {min: 180, max: 225});
  assert.equal(draftSummary.items[0].estimatedName, 'Rice');
  assert.equal(draftSummary.canSave, true);
  assert.equal('photoBytes' in draftSummary, false);

  const history = JSON.parse(await modelContext.executeTool(byName.meal_list_saved_records, {limit: 1}));
  assert.equal(calls.historyReads, 1);
  assert.equal(history.count, 1);
  assert.equal(history.records[0].id, 'saved-run');
  assert.equal('photoBytes' in history.records[0], false);
  assert.equal(JSON.stringify(history).includes('must-not-leave-storage'), false);

  const update = JSON.parse(await modelContext.executeTool(byName.meal_update_unsaved_portion, {
    itemIndex: 0, grams: 200, name: 'Cooked rice',
  }));
  assert.equal(update.updated, true);
  assert.deepEqual(update.draft.calorieRange, {min: 240, max: 300});
  assert.deepEqual(draft.edits, [{name: 'Cooked rice', grams: '200'}]);
  assert.equal(draft.result.items[0].grams, 150, 'the model estimate remains unchanged');
  assert.equal(draft.canSave, true, 'the draft remains eligible for the existing manual Save button');
  assert.equal(savedRecords.length, 1, 'the WebMCP edit never writes the saved history');
  assert.equal(windowRef.listenerCount('pagehide'), 1);
  binding.dispose();
});

test('WebMCP rejects unknown or invalid inputs before draft or history state changes', async () => {
  const modelContext = new FakeModelContext();
  const draft = {result: checkedResult(result()), edits: [{name: 'Rice', grams: '150'}], canSave: true, busy: false};
  const before = structuredClone(draft);
  let historyReads = 0;
  const binding = registerMealWebMcp({
    documentRef: {modelContext},
    windowRef: new FakeWindow(),
    getDraftSummary: () => summarizeMealDraft(draft),
    updateDraftPortion: (input) => {
      const change = applyUnsavedMealPortion(draft, input);
      draft.edits = change.edits;
      return change;
    },
    listSavedRecords: () => { historyReads += 1; return {count: 0, records: []}; },
  });
  await binding.ready;
  const tools = Object.fromEntries((await modelContext.getTools()).map((tool) => [tool.name, tool]));

  for (const invalid of [
    {itemIndex: 0, grams: 200, source: 'unexpected'},
    {itemIndex: 0, grams: '200'},
    {itemIndex: 0, grams: 10001},
    {itemIndex: 20, grams: 200},
    {itemIndex: 0, grams: 200, name: '   '},
  ]) {
    await assert.rejects(modelContext.executeTool(tools.meal_update_unsaved_portion, invalid));
    assert.deepEqual(draft, before);
  }
  await assert.rejects(modelContext.executeTool(tools.meal_list_saved_records, {limit: 21}));
  await assert.rejects(modelContext.executeTool(tools.meal_get_draft_summary, {includePhoto: true}));
  assert.equal(historyReads, 0);
  binding.dispose();
});

test('WebMCP is optional and its registrations survive BFCache but clear on real page exit', async () => {
  let callbackCalls = 0;
  const callbacks = {
    getDraftSummary() { callbackCalls += 1; return {}; },
    updateDraftPortion() { callbackCalls += 1; return {}; },
    listSavedRecords() { callbackCalls += 1; return {}; },
  };
  const unsupportedWindow = new FakeWindow();
  const unsupported = registerMealWebMcp({documentRef: {}, windowRef: unsupportedWindow, ...callbacks});
  assert.equal(unsupported.supported, false);
  assert.equal(await unsupported.ready, false);
  assert.equal(unsupportedWindow.listenerCount('pagehide'), 0);
  assert.equal(unsupportedWindow.listenerCount('pageshow'), 0);
  assert.equal(callbackCalls, 0, 'missing document.modelContext does not call app handlers');

  const modelContext = new FakeModelContext();
  const page = new FakeWindow();
  const binding = registerMealWebMcp({documentRef: {modelContext}, windowRef: page, ...callbacks});
  await binding.ready;
  assert.equal(callbackCalls, 0, 'registering business tools does not run or save an operation');
  assert.equal((await modelContext.getTools()).length, 3);
  assert.equal(modelContext.registerCalls.length, 3);

  page.dispatch('pagehide', {persisted: true});
  assert.equal((await modelContext.getTools()).length, 3, 'a BFCache entry retains its live document registration');
  page.dispatch('pageshow', {persisted: true});
  await binding.ready;
  assert.equal((await modelContext.getTools()).length, 3);
  assert.equal(modelContext.registerCalls.length, 3, 'restore does not duplicate the registered tools');

  page.dispatch('pagehide', {persisted: false});
  await Promise.resolve();
  assert.equal((await modelContext.getTools()).length, 0, 'real navigation aborts all three registrations');
  binding.dispose();

  const reloaded = registerMealWebMcp({documentRef: {modelContext}, windowRef: new FakeWindow(), ...callbacks});
  await reloaded.ready;
  assert.equal((await modelContext.getTools()).length, 3, 'a new document can register one fresh copy');
  assert.equal(modelContext.registerCalls.length, 6);
  reloaded.dispose();
});

test('native registration does not require page-side discovery, and failed batches remove earlier tools', async () => {
  const callbacks = {
    getDraftSummary: () => ({available: false}),
    updateDraftPortion: () => ({}),
    listSavedRecords: () => ({records: []}),
  };
  const backing = new FakeModelContext();
  const native = {registerTool: backing.registerTool.bind(backing)};
  const binding = registerMealWebMcp({documentRef: {modelContext: native}, windowRef: new FakeWindow(), ...callbacks});
  assert.equal(await binding.ready, true);
  assert.equal((await backing.getTools()).length, 3);
  assert.equal(native.getTools, undefined, 'registration preserves the partial native implementation');
  binding.dispose();
  assert.equal((await backing.getTools()).length, 0);

  let attempts = 0;
  const failing = {async registerTool(tool, options) {
    if (++attempts === 2) throw new Error('Browser registration denied');
    await backing.registerTool(tool, options);
  }};
  const denied = registerMealWebMcp({documentRef: {modelContext: failing}, windowRef: new FakeWindow(), ...callbacks});
  assert.equal(await denied.ready, false);
  assert.equal((await backing.getTools()).length, 0, 'a failure removes the partially registered batch');
  denied.dispose();
});

test('saving finalizes the draft and its WebMCP edit tool cannot alter saved history', async () => {
  const modelContext = new FakeModelContext();
  const app = await bootCalorieApp({agent: successfulMealAgent(), modelContext});
  await waitFor(() => modelContext.registerCalls.length === 3, 'app should register its three WebMCP tools');
  app.prepareImage();
  await app.click('analyze');

  assert.ok(app.state().completedRunId);
  assert.equal(app.document.getElementById('save').disabled, false);
  const oldGramsInput = app.document.getElementById('food-items').querySelectorAll('.food')[0].querySelector('[data-field="grams"]');
  app.click('save');
  const savedStorage = app.storage.value;
  const savedRecords = JSON.parse(savedStorage);
  assert.equal(savedRecords.length, 1);
  assert.equal(app.state().completedRunId, null);
  assert.equal(app.state().result, null);
  assert.equal(app.document.getElementById('result').hidden, true);
  assert.equal(app.document.getElementById('save').disabled, true);

  // A late event from a detached input cannot re-enable Save after finalization.
  oldGramsInput.dispatchEvent(new Event('input'));
  assert.equal(app.document.getElementById('save').disabled, true);

  const tools = Object.fromEntries((await modelContext.getTools()).map((tool) => [tool.name, tool]));
  const summary = JSON.parse(await modelContext.executeTool(tools.meal_get_draft_summary, {}));
  assert.equal(summary.available, false);
  await assert.rejects(modelContext.executeTool(tools.meal_update_unsaved_portion, {itemIndex: 0, grams: 200}));
  assert.equal(app.storage.value, savedStorage, 'WebMCP cannot rewrite the saved run');
  assert.deepEqual(JSON.parse(app.storage.value), savedRecords);
});

test('a failed storage write keeps the completed draft available for correction and retry', async () => {
  const modelContext = new FakeModelContext();
  const storage = {
    value: '[]', length: 0, key: () => null, getItem() { return this.value; }, removeItem() {},
    setItem() { throw new Error('storage quota denied'); },
  };
  const app = await bootCalorieApp({agent: successfulMealAgent(), modelContext, storage});
  await waitFor(() => modelContext.registerCalls.length === 3, 'app should register its three WebMCP tools');
  app.prepareImage();
  await app.click('analyze');
  app.click('save');

  assert.ok(app.state().completedRunId);
  assert.ok(app.state().result);
  assert.equal(app.document.getElementById('result').hidden, false);
  assert.equal(app.document.getElementById('save').disabled, false);
  assert.match(app.document.getElementById('error').textContent, /storage quota denied/);
  assert.equal(storage.value, '[]');

  const tools = Object.fromEntries((await modelContext.getTools()).map((tool) => [tool.name, tool]));
  const update = JSON.parse(await modelContext.executeTool(tools.meal_update_unsaved_portion, {itemIndex: 0, grams: 220}));
  assert.equal(update.updated, true);
  assert.deepEqual(update.calorieRange, {min: 264, max: 330});
  assert.equal(app.document.getElementById('save').disabled, false);
  assert.equal(storage.value, '[]', 'editing stays in the visible draft until a later successful Save');
});

test('WebMCP missing or denied is reported without disabling the manual meal UI', async () => {
  const exampleRoot = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  const html = await readFile(new URL('index.html', exampleRoot), 'utf8');
  assert.match(html, /id="webmcp-status"[^>]*role="status"[^>]*hidden/);

  const missing = await bootCalorieApp({agent: successfulMealAgent()});
  const missingStatus = missing.document.getElementById('webmcp-status');
  assert.equal(missingStatus.hidden, false);
  assert.match(missingStatus.textContent, /未提供/);
  assert.ok(missing.document.getElementById('analyze').listeners.get('click')?.size);
  assert.ok(missing.document.getElementById('photo-choose').listeners.get('click')?.size);

  const denied = await bootCalorieApp({
    agent: successfulMealAgent(),
    modelContext: {async registerTool() { throw new Error('browser denied registration'); }},
  });
  await waitFor(
    () => denied.document.getElementById('webmcp-status').textContent.includes('注册失败'),
    'registration failure should reach the visible status row',
  );
  assert.ok(denied.document.getElementById('analyze').listeners.get('click')?.size);
  assert.ok(denied.document.getElementById('save').listeners.get('click')?.size);

  const registeredContext = new FakeModelContext();
  const registered = await bootCalorieApp({agent: successfulMealAgent(), modelContext: registeredContext});
  await waitFor(() => registeredContext.registerCalls.length === 3, 'app should register all tools');
  await waitFor(
    () => registered.document.getElementById('webmcp-status').textContent.includes('已注册'),
    'successful registration should be visible without claiming external Agent acceptance',
  );
});

test('legacy IndexedDB cleanup failure cannot abort ordinary App startup', async () => {
  for (const config of [
    {indexedDBGetter: () => { throw new Error('IndexedDB getter denied'); }},
    {indexedDB: {deleteDatabase() { throw new Error('IndexedDB delete denied'); }}},
  ]) {
    const warnings = [];
    const app = await bootCalorieApp({...config, warnings});
    assert.ok(warnings.length > 0, 'cleanup failure is recorded for diagnostics');
    for (const id of ['photo-choose', 'camera-open', 'analyze', 'save']) {
    assert.ok(app.document.getElementById(id).listeners.get('click')?.size);
    }
  }
});

test('oversized locally edited history is rejected by the app tool without a storage write', async () => {
  const modelContext = new FakeModelContext();
  const malformed = mealRecord(checkedResult(result()), [{name: 'Rice', grams: 150}], 'oversized-record');
  malformed.items[0].name = 'Rice'.repeat(40);
  const storage = {
    value: JSON.stringify([malformed]), length: 0, key: () => null,
    getItem() { return this.value; }, setItem() { throw new Error('must not write while listing'); }, removeItem() {},
  };
  const app = await bootCalorieApp({agent: successfulMealAgent(), modelContext, storage});
  await waitFor(() => modelContext.registerCalls.length === 3, 'app should register its three WebMCP tools');
  const before = storage.value;
  const tools = Object.fromEntries((await modelContext.getTools()).map((tool) => [tool.name, tool]));
  await assert.rejects(modelContext.executeTool(tools.meal_list_saved_records, {}));
  assert.equal(storage.value, before);
  assert.equal(app.state().result, null);
});

test('persistent chat restores the same thread read-only and starts runs only for new user turns', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/attachment-chat-skill/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('lingxi-app.json', base), 'utf8'));
  assert.deepEqual(manifest.capabilities, ['agent']);
  assert.deepEqual(manifest.agent.tools, []);
  assert.deepEqual(manifest.agent.skills, [{id: 'review', path: 'skills/review/SKILL.md'}]);
  const [source, skill] = await Promise.all([
    readFile(new URL('app.mjs', base), 'utf8'),
    readFile(new URL(manifest.agent.skills[0].path, base), 'utf8'),
  ]);
  for (const api of ['agent.chats.create', 'agent.chats.open', 'agent.createClient({threadId: chat.id, initialMessages: messages})',
    'Array.isArray(page.chats)', 'page.nextCursor',
    'chatHandle.history(options)', 'page.chats', 'client.setMessages(messages)', 'app.agent.tasks.get(draft.runId)',
    'app.agent.createClient({threadId: task.threadId})', 'recoveryClient.connectAgent({runId: draft.runId}, recovery.subscriber)',
    'client.setMessages([message])', 'client.runAgent({', 'app.agent.file(file)',
    'runId: draft.runId', "'com.lingxi.localApp'", 'onRunErrorEvent', 'onRunFinishedEvent', 'metadata[\'com.lingxi.localApp\']',
    "run.getOutcome() === 'cancelled'", 'client.abortRun()']) {
    assert.ok(source.includes(api), `attachment-chat example should use ${api}`);
  }
  const boot = source.slice(source.indexOf('async function boot()'), source.indexOf("$('attach').addEventListener"));
  assert.doesNotMatch(boot, /runAgent\(/, 'startup restores a thread without starting a model request');
  assert.match(source, /message\.content/);
  assert.match(source, /refresh will not send it automatically/i);
  assert.match(source, /draftTtlMs = 24 \* 60 \* 60 \* 1000/);
  assert.match(skill, /untrusted data/);
  assert.doesNotMatch(source, /api[_-]?key|OPENAI_API_KEY|window\.lingxi|innerHTML/i);
});

test('a refreshed chat reconnects a saved run by task identity and preserves RUN_ERROR drafts', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/attachment-chat-skill/', import.meta.url);
  const original = await readFile(new URL('app.mjs', base), 'utf8');
  const source = original.replace(/\nvoid boot\(\)\.catch\(\(failure\) => setStatus\(failure\.message\)\);\s*$/, '');
  assert.notEqual(source, original, 'test harness should replace the browser boot call');

  class Element {
    constructor(id = '') {
      this.id = id;
      this.value = '';
      this.checked = false;
      this.hidden = false;
      this.disabled = false;
      this.textContent = '';
      this.children = [];
      this.dataset = {};
      this.listeners = new Map();
    }
    addEventListener(type, listener) { this.listeners.set(type, listener); }
    replaceChildren(...children) { this.children = children; }
    append(...children) { this.children.push(...children); }
    add(child) { this.children.push(child); }
    click() {}
  }

  const elements = new Map();
  const document = {
    visibilityState: 'visible',
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
    createElement(tag) { return new Element(tag); },
    addEventListener() {},
  };
  const historyMessage = {id: 'message-1', role: 'user', content: [{type: 'text', text: 'Keep this exact request'}]};
  const calls = {createdClients: [], connects: [], starts: 0, taskLookups: [], chatLists: []};
  const chat = {
    id: 'chat-1',
    async history() { return {messages: [historyMessage]}; },
  };
  const localApp = {
    async info() { return {revision: 'rev-1'}; },
    agent: {
      chats: {
        async list(options) {
          calls.chatLists.push(options);
          return calls.chatLists.length === 1
            ? {chats: [{chatId: 'chat-1', title: 'Saved chat'}], nextCursor: 'cursor-1'}
            : {chats: []};
        },
        async open(id) { assert.equal(id, 'chat-1'); return chat; },
      },
      tasks: {
        async get(runId) {
          calls.taskLookups.push(runId);
          return {runId, threadId: 'chat-1', state: 'failed'};
        },
      },
      createClient(options) {
        calls.createdClients.push(options);
        return {
          messages: [...(options.initialMessages || [])],
          setMessages(messages) { this.messages = [...messages]; },
          async connectAgent(parameters, subscriber) {
            calls.connects.push({options, parameters});
            await subscriber.onRunErrorEvent({event: {
              type: 'RUN_ERROR',
              message: 'Provider disconnected',
              code: 'provider_unavailable',
            }});
            return {result: undefined, newMessages: []};
          },
          async runAgent() { calls.starts += 1; return {result: {}, newMessages: []}; },
          abortRun() {},
        };
      },
    },
  };
  const context = {localApp};
  const localStorage = {getItem() { return 'chat-1'; }, setItem() {}, removeItem() {}};
  const Option = class { constructor(label, value) { this.label = label; this.value = value; } };
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const runFixture = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'File', 'confirm', 'setInterval', 'Option',
    `${source}\nawait boot();\nreturn {sendTurn, pending: () => pendingDraft, setPending: (draft) => { pendingDraft = draft; }};`);
  const fixture = await runFixture(context, document, {addEventListener() {}}, localStorage, class File {}, () => false, () => 0, Option);
  const draft = {chatId: 'chat-1', runId: 'run-1', messageId: 'message-1', prompt: 'Keep this exact request', files: [], skills: []};
  fixture.setPending(draft);

  await fixture.sendTurn();

  assert.deepEqual(calls.chatLists, [
    {limit: 50, includeArchived: false},
    {limit: 50, includeArchived: false, cursor: 'cursor-1'},
  ]);
  assert.deepEqual(calls.taskLookups, ['run-1']);
  assert.deepEqual(calls.createdClients[1], {threadId: 'chat-1'}, 'recovery client has no initial messages or remembered run ID');
  assert.deepEqual(calls.connects, [{options: {threadId: 'chat-1'}, parameters: {runId: 'run-1'}}]);
  assert.equal(calls.starts, 0, 'RUN_ERROR recovery must not start a model request');
  assert.equal(fixture.pending(), draft, 'the exact retry input and run ID remain saved after RUN_ERROR');
  assert.match(elements.get('status').textContent, /Provider disconnected/);
  assert.match(elements.get('status').textContent, /remain saved/);
});

test('a persistent Chat run sends only its new user Message, not historical File bytes', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/attachment-chat-skill/', import.meta.url);
  const original = await readFile(new URL('app.mjs', base), 'utf8');
  const source = original.replace(/\nvoid boot\(\)\.catch\(\(failure\) => setStatus\(failure\.message\)\);\s*$/, '');
  assert.notEqual(source, original, 'test harness should replace the browser boot call');

  class Element {
    constructor(id = '') {
      this.id = id;
      this.value = '';
      this.checked = false;
      this.hidden = false;
      this.disabled = false;
      this.textContent = '';
      this.children = [];
      this.dataset = {};
      this.listeners = new Map();
    }
    addEventListener(type, listener) { this.listeners.set(type, listener); }
    replaceChildren(...children) { this.children = children; }
    append(...children) { this.children.push(...children); }
    add(child) { this.children.push(child); }
    click() {}
  }

  const elements = new Map();
  const document = {
    visibilityState: 'visible',
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
    createElement(tag) { return new Element(tag); },
    addEventListener() {},
  };
  const oldFileBytes = Buffer.from('HISTORICAL_FILE_BYTES_MUST_NOT_BE_RESENT').toString('base64');
  let history = [
    {id: 'old-user', role: 'user', content: [
      {type: 'text', text: 'Review this earlier file.'},
      {type: 'document', source: {type: 'data', value: oldFileBytes, mimeType: 'text/plain'}, metadata: {filename: 'old.txt'}},
    ]},
    {id: 'old-assistant', role: 'assistant', content: 'The earlier file was reviewed.'},
  ];
  const calls = {runInputs: [], clients: []};
  const chat = {
    id: 'chat-1',
    async history() { return {messages: structuredClone(history)}; },
  };
  const localApp = {
    async info() { return {revision: 'rev-1'}; },
    agent: {
      chats: {
        async list() { return {chats: [{chatId: 'chat-1', title: 'Saved chat'}]}; },
        async open(id) { assert.equal(id, 'chat-1'); return chat; },
      },
      createClient(options) {
        calls.clients.push(options);
        return {
          messages: [...(options.initialMessages || [])],
          setMessages(messages) { this.messages = [...messages]; },
          async runAgent(parameters, subscriber) {
            const input = {
              threadId: options.threadId,
              runId: parameters.runId,
              messages: structuredClone(this.messages),
              forwardedProps: parameters.forwardedProps,
            };
            calls.runInputs.push(input);
            const newAssistant = {id: 'new-assistant', role: 'assistant', content: 'The new turn was reviewed.'};
            history = [...history, ...structuredClone(this.messages), newAssistant];
            await subscriber.onRunFinishedEvent({outcome: 'success'});
            return {result: undefined, newMessages: [newAssistant]};
          },
          abortRun() {},
        };
      },
    },
  };
  const context = {localApp, crypto: {randomUUID: (() => { let id = 0; return () => `generated-${++id}`; })()}};
  const localStorage = {getItem() { return 'chat-1'; }, setItem() {}, removeItem() {}};
  const Option = class { constructor(label, value) { this.label = label; this.value = value; } };
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const runFixture = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'File', 'confirm', 'setInterval', 'Option',
    `${source}\nawait boot();\nreturn {sendTurn};`);
  const fixture = await runFixture(context, document, {addEventListener() {}}, localStorage, class File {}, () => false, () => 0, Option);
  elements.get('prompt').value = 'Review only the new turn.';

  await fixture.sendTurn();

  assert.equal(calls.runInputs.length, 1);
  assert.equal(calls.runInputs[0].threadId, 'chat-1');
  assert.equal(calls.runInputs[0].messages.length, 1, 'the host owns historical context for this persistent thread');
  assert.equal(calls.runInputs[0].messages[0].role, 'user');
  assert.equal(calls.runInputs[0].messages[0].content[0].text, 'Review only the new turn.');
  assert.equal(JSON.stringify(calls.runInputs[0]).includes(oldFileBytes), false, 'historical file bytes must not be sent again');
  assert.equal(history.length, 4, 'history remains canonical and includes both completed turns');
  assert.equal(calls.clients.length, 1, 'the same persistent thread client is reused for the new turn');
});

test('plugin metadata and generator guidance name AG-UI 1.0 as the only Agent contract', async () => {
  const root = new URL('../../', import.meta.url);
  const [plugin, readme, skill, reference] = await Promise.all([
    readFile(new URL('plugins/local-app-builder/.lingxi-plugin/plugin.json', root), 'utf8'),
    readFile(new URL('README.md', root), 'utf8'),
    readFile(new URL('plugins/local-app-builder/skills/create-local-app/SKILL.md', root), 'utf8'),
    readFile(new URL('plugins/local-app-builder/skills/create-local-app/references/host-agent.md', root), 'utf8'),
  ]);
  assert.equal(JSON.parse(plugin).version, '6.1.0');
  for (const document of [readme, reference]) {
    assert.match(document, /AG-UI 1\.0/);
    assert.match(document, /RunAgentInput/);
  }
  assert.match(skill, /AG-UI 1\.0/);
  assert.match(skill, /references\/host-agent\.md/);
  assert.match(reference, /Message\[\]/);
  assert.match(reference, /ContentPart/);
  assert.match(reference, /BaseEvent/);
  assert.match(reference, /forwardedProps[\s\S]{0,100}com\.lingxi\.localApp/);
  assert.match(reference, /exactly one user `Message`/);
  assert.match(reference, /audio and video parts are unsupported/);
  assert.match(reference, /nextCursor/);
  assert.match(reference, /onRunFinishedEvent\(\{outcome: terminalOutcome\}\)/);
  assert.match(reference, /connectAgent\(\{runId\},/);
  assert.match(reference, /read-only/);
  assert.match(reference, /finite OS execution lease/);
});
