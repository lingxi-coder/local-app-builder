import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { checkedResult, calorieRange, mealRecord, recognitionPrompt } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs';

const food = { name: 'Rice', grams: 150, kcalPer100gMin: 120, kcalPer100gMax: 150, basis: 'model_estimate', sources: [] };
const result = () => ({ items: [{...food}], assumptions: ['Cooking oil is unknown'], questions: ['How much rice?'] });

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
  assert.deepEqual(manifest.package.files, ['index.html', 'styles.css', 'app.mjs', 'nutrition.mjs']);

  const source = await readFile(new URL('app.mjs', base), 'utf8');
  for (const api of ['agent.createClient({threadId: crypto.randomUUID()})', 'await agent.file(image)',
    "role: 'user'", "type: 'text'", 'runClient.addMessage', 'runClient.runAgent',
    "'com.lingxi.localApp'", 'outputSchema: nutritionSchema', 'onRunErrorEvent',
    'onRunFinishedEvent({outcome: terminalOutcome})', "outcome === 'cancelled'", 'runClient.abortRun()', 'mealRecord']) {
    assert.ok(source.includes(api), `calorie example should use ${api}`);
  }
  assert.doesNotMatch(source, /fetch\(|api[_-]?key|OPENAI_API_KEY|window\.lingxi|innerHTML/i);
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
  assert.equal(JSON.parse(plugin).version, '6.0.0');
  for (const document of [readme, skill, reference]) {
    assert.match(document, /AG-UI 1\.0/);
    assert.match(document, /RunAgentInput/);
  }
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
