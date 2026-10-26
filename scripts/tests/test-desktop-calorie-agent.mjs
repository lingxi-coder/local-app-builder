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

test('the calorie page keeps its ordinary UI boot path without document.modelContext', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  let source = await readFile(new URL('app.mjs', base), 'utf8');
  source = source
    .replace(/^import .* from '\.\/nutrition\.mjs';\s*$/m, '')
    .replace(/^import .* from '\.\/webmcp\.mjs';\s*$/m, '');
  class Element {
    constructor(id = '') {
      this.id = id;
      this.value = '';
      this.hidden = false;
      this.disabled = false;
      this.textContent = '';
      this.listeners = new Map();
      this.children = [];
    }
    addEventListener(type, listener) { this.listeners.set(type, listener); }
    replaceChildren(...children) { this.children = children; }
    append(...children) { this.children.push(...children); }
  }
  const elements = new Map();
  const document = {
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, new Element(id));
      return elements.get(id);
    },
  };
  const localStorage = {length: 0, getItem: () => '[]', setItem() {}, removeItem() {}, key: () => null};
  const sessionStorage = {length: 0, getItem: () => null, setItem() {}, removeItem() {}, key: () => null};
  const window = {localApp: {}, listeners: new Map(), addEventListener(type, listener) { this.listeners.set(type, listener); }};
  const context = {localApp: {}, localStorage, sessionStorage};
  const nutrition = await import('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs');
  const webmcp = await import('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/webmcp.mjs');
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const runApp = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'sessionStorage', 'nutrition', 'webmcp',
    `const {nutritionSchema, recognitionPrompt, checkedResult, calorieRange, mealRecord} = nutrition;\n`
      + `const {applyUnsavedMealPortion, registerMealWebMcp, summarizeMealDraft, summarizeSavedMealRecords} = webmcp;\n`
      + `${source}\nreturn {document, window};`);

  const app = await runApp(context, document, window, localStorage, sessionStorage, nutrition, webmcp);
  for (const id of ['photo-choose', 'camera-open', 'analyze', 'cancel', 'save']) {
    assert.equal(typeof app.document.getElementById(id)?.listeners.get('click'), 'function', `${id} UI remains wired`);
  }
  assert.match(app.document.getElementById('status').textContent, /普通浏览器无法调用宿主 Agent/);
  assert.equal(document.modelContext, undefined);
  assert.deepEqual([...window.listeners.keys()], ['pagehide'], 'only ordinary camera cleanup was installed');
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
