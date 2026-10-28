import assert from 'node:assert/strict';
import test from 'node:test';
import {readFile} from 'node:fs/promises';

class Element {
  constructor(id = '') {
    Object.assign(this, {id, value: '', checked: false, hidden: false, disabled: false, textContent: '', children: [], dataset: {}});
  }
  addEventListener() {}
  replaceChildren(...children) { this.children = children; }
  append(...children) { this.children.push(...children); }
  add(child) { this.children.push(child); }
}

class MemoryIndexedDB {
  constructor() { this.records = new Map(); }
  open() {
    const request = {};
    const records = this.records;
    const db = {
      close() {},
      transaction() {
        const tx = {};
        tx.objectStore = () => ({
          get(key) {
            const result = {};
            queueMicrotask(() => {
              result.result = structuredClone(records.get(key));
              result.onsuccess?.();
              queueMicrotask(() => tx.oncomplete?.());
            });
            return result;
          },
          put(value) {
            queueMicrotask(() => { records.set(value.key, structuredClone(value)); tx.oncomplete?.(); });
          },
          delete(key) {
            queueMicrotask(() => { records.delete(key); tx.oncomplete?.(); });
          },
        });
        return tx;
      },
    };
    queueMicrotask(() => { request.result = db; request.onsuccess?.(); });
    return request;
  }
}

class TestFile {
  constructor(name, size = 4) { this.name = name; this.size = size; }
}

async function boot({localApp, indexedDB}) {
  const path = new URL('../../plugins/local-app-builder/assets/examples/attachment-chat-skill/app.mjs', import.meta.url);
  const original = await readFile(path, 'utf8');
  const source = original.replace(/\nvoid boot\(\)\.catch\(\(failure\) => setStatus\(failure\.message\)\);\s*$/, '');
  assert.notEqual(source, original);
  const elements = new Map();
  const document = {
    visibilityState: 'visible',
    getElementById(id) { if (!elements.has(id)) elements.set(id, new Element(id)); return elements.get(id); },
    createElement(tag) { return new Element(tag); },
    addEventListener() {},
  };
  const context = {
    localApp, indexedDB,
    crypto: {randomUUID: (() => { let id = 0; return () => `generated-${++id}`; })()},
  };
  const storage = {getItem() { return 'chat-1'; }, setItem() {}, removeItem() {}};
  const Option = class { constructor(label, value) { this.label = label; this.value = value; } };
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const run = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'indexedDB', 'File', 'confirm', 'setInterval', 'Option',
    `${source}\nawait boot();\nreturn {sendTurn, updateControls, setFiles: (files) => { selectedFiles = files; }, pending: () => pendingDraft};`);
  const fixture = await run(context, document, {addEventListener() {}}, storage, indexedDB, TestFile, () => false, () => 0, Option);
  return {fixture, elements};
}

function host({platform = 'desktop', getTask, file, runAgent, connectAgent}) {
  const chat = {id: 'chat-1', async history() { return {messages: []}; }};
  return {
    async info() { return {revision: 'rev-1', platform}; },
    agent: {
      chats: {
        async list() { return {chats: [{chatId: chat.id, title: 'Saved chat'}]}; },
        async open(id) { assert.equal(id, chat.id); return chat; },
      },
      tasks: {get: getTask},
      file,
      createClient(options) {
        return {
          messages: [...(options.initialMessages || [])],
          setMessages(messages) { this.messages = [...messages]; },
          runAgent(parameters, subscriber) { return runAgent.call(this, options, parameters, subscriber); },
          connectAgent(parameters, subscriber) { return connectAgent(options, parameters, subscriber); },
          abortRun() {},
        };
      },
    },
  };
}

test('an ambiguous attachment retry after reload reuses its original message and concurrent clicks start once', async () => {
  const indexedDB = new MemoryIndexedDB();
  let enteredFile;
  let releaseFile;
  const entered = new Promise((resolve) => { enteredFile = resolve; });
  const gate = new Promise((resolve) => { releaseFile = resolve; });
  let fileCalls = 0;
  const inputs = [];
  const lookups = [];
  const localApp = host({
    platform: 'mobile-browser',
    async file(file) {
      fileCalls++;
      enteredFile();
      await gate;
      return {type: 'document', id: `part-${fileCalls}`, source: {type: 'data', mimeType: 'text/plain', value: 'ZGF0YQ=='},
        metadata: {'com.lingxi.localApp': {version: 1, filename: file.name}}};
    },
    async getTask(runId) {
      lookups.push(runId);
      const failure = new Error('Not visible yet');
      failure.code = 'task_not_found';
      throw failure;
    },
    async runAgent(options, parameters, subscriber) {
      inputs.push(structuredClone({threadId: options.threadId, messages: this.messages, ...parameters}));
      if (inputs.length === 1) throw new Error('Delivery is uncertain');
      subscriber.onRunFinishedEvent({outcome: 'success'});
      return {newMessages: [], result: undefined};
    },
    async connectAgent() { throw new Error('No task was found'); },
  });
  const first = await boot({localApp, indexedDB});
  first.elements.get('prompt').value = 'Review the file';
  first.fixture.setFiles([new TestFile('private.txt')]);
  const send = first.fixture.sendTurn();
  await entered;
  assert.equal(first.elements.get('cancel').hidden, true, 'preparing a new input cannot cancel an older run');
  const duplicateClick = first.fixture.sendTurn();
  releaseFile();
  await Promise.all([send, duplicateClick]);
  assert.equal(inputs.length, 1, 'a second click while preparing the first request starts no second run');
  assert.equal(fileCalls, 1);
  const saved = indexedDB.records.get('attachment-review:revision-rev-1').draft;
  assert.equal(saved.message.content[1].id, 'part-1');
  assert.equal(saved.files, undefined, 'the generated part is saved without a second copy of the File');

  const reopened = await boot({localApp, indexedDB});
  assert.equal(inputs.length, 1, 'reload restores the draft without sending it');
  assert.equal(reopened.elements.get('file-names').textContent, 'private.txt');
  await reopened.fixture.sendTurn();
  assert.deepEqual(lookups, [saved.runId, saved.runId, saved.runId]);
  assert.equal(inputs.length, 2, 'only the explicit retry resubmits');
  assert.deepEqual(inputs[1], inputs[0], 'the retry preserves run, thread, message, part IDs, file bytes and options');
  assert.equal(fileCalls, 1, 'the retry must not regenerate attachment part IDs');
  assert.equal(indexedDB.records.size, 0, 'the application draft is removed after successful completion');
});

test('a saved run is reconnected even before its message appears in Chat history', async () => {
  const indexedDB = new MemoryIndexedDB();
  const draft = {chatId: 'chat-1', runId: 'run-1', messageId: 'msg-1', prompt: 'hello', skills: [],
    message: {id: 'msg-1', role: 'user', content: [{type: 'text', text: 'hello'}]}};
  indexedDB.records.set('attachment-review:revision-rev-1', {key: 'attachment-review:revision-rev-1', draft, createdAt: Date.now()});
  const calls = {starts: 0, connections: [], lookups: []};
  const localApp = host({
    async getTask(runId) { calls.lookups.push(runId); return {runId, threadId: 'chat-1', state: 'running'}; },
    async file() { throw new Error('Recovery must not re-encode files'); },
    async runAgent() { calls.starts++; throw new Error('Recovery must not start a run'); },
    async connectAgent(options, parameters, subscriber) {
      calls.connections.push({options, parameters});
      subscriber.onRunFinishedEvent({outcome: 'success'});
      return {newMessages: [], result: undefined};
    },
  });
  const {fixture} = await boot({localApp, indexedDB});
  await fixture.sendTurn();
  assert.deepEqual(calls.lookups, ['run-1']);
  assert.deepEqual(calls.connections, [{options: {threadId: 'chat-1'}, parameters: {runId: 'run-1'}}]);
  assert.equal(calls.starts, 0);
});

test('an incomplete saved input can be checked but cannot be resubmitted', async () => {
  const indexedDB = new MemoryIndexedDB();
  const draft = {chatId: 'chat-1', runId: 'run-1', messageId: 'msg-1', prompt: 'hello', skills: [],
    message: {id: 'different-message', role: 'user', content: [{type: 'text', text: 'hello'}]}};
  indexedDB.records.set('attachment-review:revision-rev-1', {key: 'attachment-review:revision-rev-1', draft, createdAt: Date.now()});
  const calls = {starts: 0, lookups: []};
  const localApp = host({
    async getTask(runId) {
      calls.lookups.push(runId);
      const failure = new Error('Not visible yet');
      failure.code = 'task_not_found';
      throw failure;
    },
    async file() { throw new Error('unexpected file'); },
    async runAgent() { calls.starts++; throw new Error('unexpected submission'); },
    async connectAgent() { throw new Error('unexpected reconnect'); },
  });
  const {fixture, elements} = await boot({localApp, indexedDB});
  assert.equal(elements.get('send').textContent, 'Check saved run');
  await fixture.sendTurn();
  assert.deepEqual(calls.lookups, ['run-1', 'run-1', 'run-1']);
  assert.equal(calls.starts, 0);
  assert.match(elements.get('status').textContent, /exact input is unavailable or incomplete/);
  assert.equal(indexedDB.records.size, 1);
});

test('missing IndexedDB blocks mobile Agent submission without claiming Desktop is blocked', async () => {
  let starts = 0;
  const options = {
    async getTask() { throw new Error('unexpected lookup'); },
    async file() { throw new Error('unexpected file'); },
    async runAgent(_options, _parameters, subscriber) {
      starts++;
      subscriber.onRunFinishedEvent({outcome: 'success'});
      return {newMessages: [], result: undefined};
    },
    async connectAgent() { throw new Error('unexpected reconnect'); },
  };
  const mobile = await boot({localApp: host({...options, platform: 'mobile-browser'})});
  mobile.elements.get('prompt').value = 'Hello';
  mobile.fixture.updateControls();
  assert.equal(mobile.elements.get('send').disabled, true);
  assert.match(mobile.elements.get('status').textContent, /mobile host cannot submit Agent runs/);
  await mobile.fixture.sendTurn();
  assert.equal(starts, 0);

  const desktop = await boot({localApp: host({...options, platform: 'desktop'})});
  desktop.elements.get('prompt').value = 'Hello';
  desktop.fixture.updateControls();
  assert.equal(desktop.elements.get('send').disabled, false);
  assert.match(desktop.elements.get('status').textContent, /Agent availability depends on this host/);
  await desktop.fixture.sendTurn();
  assert.equal(starts, 1);
});
