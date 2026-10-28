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

class MemoryStorage {
  constructor(entries = []) { this.records = new Map(entries); }
  getItem(key) { return this.records.get(key) ?? null; }
  setItem(key, value) { this.records.set(key, String(value)); }
  removeItem(key) { this.records.delete(key); }
}

async function boot({localApp, indexedDB, storage = new MemoryStorage(), randomUUID, initialize = true}) {
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
    crypto: {randomUUID: randomUUID || (() => { let id = 0; return () => `00000000-0000-4000-8000-${String(++id).padStart(12, '0')}`; })()},
  };
  const Option = class { constructor(label, value) { this.label = label; this.value = value; } };
  const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
  const run = new AsyncFunction('globalThis', 'document', 'window', 'localStorage', 'indexedDB', 'File', 'confirm', 'setInterval', 'Option',
    `${source}\n${initialize ? 'await boot();' : ''}\nreturn {initialize: boot, sendTurn, createChat, checkChatCreation, checkSavedChats, updateControls, setFiles: (files) => { selectedFiles = files; }, pending: () => pendingDraft, pendingCreation: () => pendingCreation};`);
  const fixture = await run(context, document, {addEventListener() {}}, storage, indexedDB, TestFile, () => false, () => 0, Option);
  return {fixture, elements, storage};
}

function host({platform = 'desktop', getTask, file, runAgent, connectAgent}) {
  const chat = {id: 'chat-1', async history() { return {messages: []}; }};
  return {
    async info() { return {id: 'app-1', revision: 'rev-1', platform}; },
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

const creationKey = 'attachment-review:app-1:revision-rev-1:chat-creation';
const creationId = 'a2a2a2a2-1212-4b4b-8c8c-121212121212';
const creationFailure = (code) => Object.assign(new Error(code), {code});

function creationHost({create, creationStatus, id = 'app-1', revision = 'rev-1'}) {
  return {
    async info() { return {id, revision, platform: 'desktop'}; },
    agent: {
      chats: {
        create, creationStatus,
        async list() { return {chats: []}; }, // Simulate a delayed list projection.
        async open(id) { return {id, async history() { return {messages: []}; }}; },
      },
      createClient(options) { return {messages: options.initialMessages || []}; },
    },
  };
}

test('ambiguous Chat creation survives refresh and only explicit retry submits the exact original request', async () => {
  const storage = new MemoryStorage();
  const posts = [];
  const reads = [];
  let release;
  let entered;
  const gate = new Promise((resolve) => { release = resolve; });
  const started = new Promise((resolve) => { entered = resolve; });
  let idsGenerated = 0;
  const localApp = creationHost({
    async create(options) {
      assert.deepEqual(JSON.parse(storage.getItem(creationKey)), options, 'intent must be durable before POST');
      posts.push(structuredClone(options));
      if (posts.length === 1) {
        entered();
        await gate;
        throw creationFailure('host_unavailable');
      }
      return {id: options.requestId};
    },
    async creationStatus(options) { reads.push(structuredClone(options)); return null; },
  });
  const first = await boot({localApp, storage, randomUUID() { idsGenerated++; return creationId; }});
  const initial = first.fixture.createChat();
  await started;
  await first.fixture.createChat();
  release();
  await initial;
  assert.equal(posts.length, 1, 'duplicate taps do not generate a second request');
  assert.equal(idsGenerated, 1);
  assert.deepEqual(reads, [posts[0]], 'failed creation triggers only a same-input read');
  assert.match(first.elements.get('status').textContent, /may still be in flight/);
  const original = storage.getItem(creationKey);

  const refreshed = await boot({localApp, storage, randomUUID() { throw new Error('refresh/retry must not replace the ID'); }});
  assert.equal(posts.length, 1, 'refresh must not POST');
  assert.equal(storage.getItem(creationKey), original);
  assert.deepEqual(reads, [posts[0], posts[0]], 'refresh is read-only');
  await refreshed.fixture.checkChatCreation();
  assert.equal(posts.length, 1, 'the Check action never POSTs');
  await refreshed.fixture.createChat({retry: true});
  assert.equal(posts.length, 2);
  assert.deepEqual(posts[1], posts[0]);
  assert.equal(storage.getItem(creationKey), null);
  assert.equal(storage.getItem('attachment-review:revision-rev-1:chatId'), creationId);
  assert.equal(refreshed.elements.get('chat-list').value, creationId, 'a delayed list cannot select another Chat');
});

test('refresh recovers a confirmed creation without POST and preserves an omitted title', async () => {
  const options = {requestId: creationId};
  const storage = new MemoryStorage([[creationKey, JSON.stringify(options)]]);
  const reads = [];
  const localApp = creationHost({
    async create() { assert.fail('confirmed creation must not POST again'); },
    async creationStatus(value) { reads.push(structuredClone(value)); return {id: value.requestId}; },
  });
  const {fixture, elements} = await boot({localApp, storage});
  assert.deepEqual(reads, [options]);
  assert.equal(Object.hasOwn(reads[0], 'title'), false);
  assert.equal(fixture.pendingCreation(), null);
  assert.equal(storage.getItem(creationKey), null);
  assert.match(elements.get('status').textContent, /creation is confirmed/);
});

test('explicit retry retains title whitespace and does not normalize saved options', async () => {
  const options = {requestId: creationId, title: '  Original title\n'};
  const storage = new MemoryStorage([[creationKey, JSON.stringify(options)]]);
  const posts = [];
  const localApp = creationHost({
    async create(value) { posts.push(structuredClone(value)); return {id: value.requestId}; },
    async creationStatus(value) { assert.deepEqual(value, options); return null; },
  });
  const {fixture} = await boot({localApp, storage});
  assert.equal(posts.length, 0);
  await fixture.createChat({retry: true});
  assert.deepEqual(posts, [options]);
});

test('missing IDs, invalid UUIDs or original input never cause replacement creation on refresh or retry', async (t) => {
  for (const options of [{title: 'lost ID'}, {requestId: creationId.toUpperCase()},
    {requestId: creationId, title: null}, {requestId: creationId, title: 'saved', unexpected: true}]) {
    await t.test(JSON.stringify(options), async () => {
      const original = JSON.stringify(options);
      const storage = new MemoryStorage([[creationKey, original]]);
      const localApp = creationHost({
        async create() { assert.fail('invalid record cannot POST'); },
        async creationStatus() { assert.fail('invalid input cannot query status'); },
      });
      const {fixture, elements} = await boot({localApp, storage, randomUUID() { assert.fail('cannot generate a replacement ID'); }});
      assert.equal(elements.get('new-chat').disabled, true);
      assert.equal(elements.get('retry-creation').hidden, true);
      await fixture.createChat({retry: true});
      await fixture.createChat();
      assert.equal(storage.getItem(creationKey), original);
      assert.match(elements.get('status').textContent, /missing or invalid/);
    });
  }
});

test('a failed durable save prevents the first creation POST', async () => {
  const storage = new MemoryStorage();
  storage.setItem = () => { throw new Error('Browser storage is full'); };
  const localApp = creationHost({
    async create() { assert.fail('storage must be saved before POST'); },
    async creationStatus() { assert.fail('no request exists to query'); },
  });
  const {fixture, elements} = await boot({localApp, storage});
  await fixture.createChat();
  assert.equal(fixture.pendingCreation(), null);
  assert.match(elements.get('status').textContent, /Browser storage is full/);
});

test('initialization and a newly saved intent from another tab cannot be overwritten by New chat', async () => {
  const options = {requestId: creationId, title: 'Other tab original'};
  const storage = new MemoryStorage();
  const reads = [];
  const localApp = creationHost({
    async create() { assert.fail('New chat cannot overwrite an unresolved intent'); },
    async creationStatus(value) { reads.push(structuredClone(value)); return null; },
  });
  const {fixture, elements} = await boot({localApp, storage, initialize: false,
    randomUUID() { assert.fail('cannot allocate an ID before restoring the pending creation'); }});
  fixture.updateControls();
  assert.equal(elements.get('new-chat').disabled, true);
  await fixture.createChat();
  assert.equal(reads.length, 0, 'initializing UI submits and queries nothing');
  await fixture.initialize();
  storage.setItem(creationKey, JSON.stringify(options));
  await fixture.createChat();
  assert.deepEqual(reads, [options], 'newly observed pending intent only receives a read-only check');
  assert.deepEqual(JSON.parse(storage.getItem(creationKey)), options);
});

test('conflict, deleted and unavailable status checks retain the intent and prevent a retry POST', async (t) => {
  for (const code of ['chat_create_conflict', 'chat_create_deleted', 'chat_storage_uncertain', 'host_unavailable']) {
    await t.test(code, async () => {
      const options = {requestId: creationId, title: 'Original'};
      const original = JSON.stringify(options);
      const storage = new MemoryStorage([[creationKey, original]]);
      let reads = 0;
      const localApp = creationHost({
        async create() { assert.fail('failed status check cannot authorize a POST'); },
        async creationStatus(value) { reads++; assert.deepEqual(value, options); throw creationFailure(code); },
      });
      const {fixture, elements} = await boot({localApp, storage});
      await fixture.createChat({retry: true});
      assert.ok(reads >= 2);
      assert.equal(storage.getItem(creationKey), original);
      assert.match(elements.get('status').textContent, new RegExp(code));
      assert.equal(fixture.pendingCreation().requestId, creationId);
    });
  }
});

test('pending Chat creation is partitioned by App ID and revision', async (t) => {
  for (const scope of [{id: 'app-2'}, {revision: 'rev-2'}]) {
    await t.test(JSON.stringify(scope), async () => {
      const original = JSON.stringify({requestId: creationId, title: 'Other scope'});
      const storage = new MemoryStorage([[creationKey, original]]);
      const localApp = creationHost({...scope,
        async create() { assert.fail('startup must not POST'); },
        async creationStatus() { assert.fail('another App/revision intent must not be queried'); },
      });
      const {fixture} = await boot({localApp, storage});
      assert.equal(fixture.pendingCreation(), null);
      assert.equal(storage.getItem(creationKey), original);
    });
  }
});

test('a confirmed creation does not remove a newer intent saved while history was loading', async () => {
  const original = {requestId: creationId};
  const newer = {requestId: 'b3b3b3b3-1212-4b4b-8c8c-121212121212', title: 'Newer intent'};
  const storage = new MemoryStorage([[creationKey, JSON.stringify(original)]]);
  const localApp = creationHost({
    async create() { assert.fail('recovery must not POST'); },
    async creationStatus(value) { assert.deepEqual(value, original); return {id: value.requestId}; },
  });
  localApp.agent.chats.open = async (id) => {
    storage.setItem(creationKey, JSON.stringify(newer));
    return {id, async history() { return {messages: []}; }};
  };
  await boot({localApp, storage});
  assert.deepEqual(JSON.parse(storage.getItem(creationKey)), newer);
});

test('a mismatched host Chat ID leaves the original intent intact', async () => {
  const original = JSON.stringify({requestId: creationId});
  const storage = new MemoryStorage([[creationKey, original]]);
  const localApp = creationHost({
    async create() { assert.fail('recovery must not POST'); },
    async creationStatus() { return {id: 'b3b3b3b3-1212-4b4b-8c8c-121212121212'}; },
  });
  const {elements} = await boot({localApp, storage});
  assert.equal(storage.getItem(creationKey), original);
  assert.match(elements.get('status').textContent, /different Chat creation identity/);
});

function chatListHost(pages) {
  const calls = {lists: [], opens: []};
  const localApp = creationHost({
    async create() { assert.fail('listing must never create a replacement'); },
    async creationStatus() { assert.fail('listing must not invent creation options'); },
  });
  localApp.agent.chats.list = async (options) => {
    calls.lists.push(structuredClone(options));
    return pages[options.cursor ? Number(options.cursor) : 0];
  };
  localApp.agent.chats.open = async (id) => {
    calls.opens.push(id);
    return {id, async history() { return {messages: []}; }};
  };
  return {localApp, calls};
}

test('page-local unavailable IDs do not erase an App-wide signal or replace the saved Chat', async () => {
  const savedKey = 'attachment-review:revision-rev-1:chatId';
  const storage = new MemoryStorage([[savedKey, 'unavailable-chat']]);
  const {localApp, calls} = chatListHost([
    {chats: [{chatId: 'healthy-chat', title: 'Available'}], hasUncertainChats: true, nextCursor: '1'},
    {chats: [], unavailableChats: [{chatId: 'unavailable-chat', code: 'chat_storage_uncertain'}]},
  ]);
  const {fixture, elements} = await boot({localApp, storage});
  assert.deepEqual(calls.lists.map((x) => x.cursor), [undefined, '1']);
  assert.deepEqual(calls.opens, [], 'a healthy Chat is not silently selected instead of the saved unavailable one');
  assert.equal(storage.getItem(savedKey), 'unavailable-chat');
  assert.equal(elements.get('chat-list').value, '');
  assert.equal(elements.get('chat-list-warning').hidden, false);
  assert.match(elements.get('chat-list-warning').textContent, /Some Chats in this App/);
  assert.match(elements.get('chat-list-warning').textContent, /unavailable-chat/);
  assert.match(elements.get('chat-list-warning').textContent, /no replacement Chat was created/);
  await fixture.checkSavedChats();
  assert.equal(calls.lists.length, 4, 'explicit Check only repeats paged reads');
  assert.deepEqual(calls.opens, []);
  assert.equal(storage.getItem(savedKey), 'unavailable-chat');
});

test('pagination continues through a page with only unavailable Chats', async () => {
  const {localApp, calls} = chatListHost([
    {chats: [], unavailableChats: [{chatId: 'unavailable-1', code: 'chat_storage_uncertain'}], hasUncertainChats: true, nextCursor: '1'},
    {chats: [{chatId: 'healthy-chat'}], unavailableChats: [], hasUncertainChats: true},
  ]);
  const {elements} = await boot({localApp});
  assert.equal(calls.lists.length, 2);
  assert.deepEqual(calls.opens, ['healthy-chat']);
  assert.match(elements.get('chat-list-warning').textContent, /unavailable-1/);
  assert.equal(elements.get('chat-list-warning').hidden, false);
});

test('missing uncertainty fields never mean healthy storage, and true is retained across pages', async (t) => {
  for (const [name, pages, expected] of [
    ['omitted fields', [{chats: []}], /did not confirm/],
    ['empty unavailable array', [{chats: [], unavailableChats: []}], /did not confirm/],
    ['App-wide true without page IDs', [{chats: [], unavailableChats: [], hasUncertainChats: true}], /Some Chats in this App/],
    ['later omitted flag', [{chats: [], hasUncertainChats: true, nextCursor: '1'}, {chats: []}], /Some Chats in this App/],
    ['later false flag', [{chats: [], hasUncertainChats: true, nextCursor: '1'}, {chats: [], hasUncertainChats: false}], /Some Chats in this App/],
    ['page IDs with omitted flag', [{chats: [], unavailableChats: [{chatId: 'unavailable-1', code: 'chat_storage_uncertain'}]}], /Some Chats in this App/],
  ]) {
    await t.test(name, async () => {
      const {localApp, calls} = chatListHost(pages);
      const {elements} = await boot({localApp});
      assert.match(elements.get('chat-list-warning').textContent, expected);
      assert.equal(elements.get('chat-list-warning').hidden, false);
      assert.deepEqual(calls.opens, []);
    });
  }
});

test('an explicitly clear list still cannot declare an absent saved Chat deleted or choose its replacement', async () => {
  const savedKey = 'attachment-review:revision-rev-1:chatId';
  const storage = new MemoryStorage([[savedKey, 'missing-chat']]);
  const {localApp, calls} = chatListHost([{chats: [{chatId: 'other-chat'}], hasUncertainChats: false}]);
  const {elements} = await boot({localApp, storage});
  assert.deepEqual(calls.opens, []);
  assert.equal(storage.getItem(savedKey), 'missing-chat');
  assert.match(elements.get('chat-list-warning').textContent, /Saved Chat missing-chat is not currently available/);
  assert.match(elements.get('chat-list-warning').textContent, /Check saved Chats again/);
});

test('all pages explicitly clear without a missing saved Chat need no uncertainty warning', async () => {
  const {localApp} = chatListHost([
    {chats: [], hasUncertainChats: false, nextCursor: '1'},
    {chats: [{chatId: 'healthy-chat'}], hasUncertainChats: false},
  ]);
  const {elements} = await boot({localApp});
  assert.equal(elements.get('chat-list-warning').hidden, true);
});

test('malformed list uncertainty is rejected rather than converted to a clean result', async () => {
  for (const status of [{hasUncertainChats: 'false'}, {unavailableChats: [{chatId: '', code: 'chat_storage_uncertain'}]}]) {
    const {localApp, calls} = chatListHost([{chats: [], ...status}]);
    await assert.rejects(boot({localApp}), /invalid unavailable Chat status/);
    assert.deepEqual(calls.opens, []);
  }
});

test('unavailable whole-store reads retain data and identity without guessing a repair from error codes', async (t) => {
  for (const message of ['Existing Agent storage format is unsupported', 'Existing Agent storage format is missing', 'Existing Chat storage format cannot be verified']) {
    await t.test(message, async () => {
      const original = JSON.stringify({requestId: creationId, title: 'Keep original'});
      const entries = [[creationKey, original], ['attachment-review:revision-rev-1:chatId', 'saved-chat'], ['business-data', 'preserved']];
      const storage = new MemoryStorage(entries);
      const localApp = creationHost({
        async create() { assert.fail('an unreadable store is not an empty store to recreate'); },
        async creationStatus() { assert.fail('whole-store failure cannot trigger another operation'); },
      });
      localApp.agent.chats.list = async () => { throw new Error(message); }; // No platform error-code matching.
      localApp.agent.chats.delete = async () => { assert.fail('failure cannot delete host data'); };
      const {fixture, elements} = await boot({localApp, storage, initialize: false});
      await assert.rejects(fixture.initialize(), new RegExp(message));
      assert.deepEqual([...storage.records], entries);
      assert.equal(elements.get('chat-list-warning').hidden, false);
      assert.match(elements.get('chat-list-warning').textContent, /data is unavailable or could not be verified/);
      assert.match(elements.get('chat-list-warning').textContent, /not automatically delete, reset or rebuild/);
      assert.equal(fixture.pendingCreation().requestId, creationId);
      await fixture.createChat();
      assert.deepEqual([...storage.records], entries, 'failed initialization never permits replacement creation');
    });
  }
});

test('a history-read failure leaves the original saved Chat and creation intent intact', async () => {
  const entries = [[creationKey, JSON.stringify({requestId: creationId})], ['attachment-review:revision-rev-1:chatId', 'saved-chat']];
  const storage = new MemoryStorage(entries);
  const {localApp} = chatListHost([{chats: [{chatId: 'saved-chat'}], hasUncertainChats: false}]);
  localApp.agent.chats.open = async (id) => ({id, async history() { throw new Error('Cannot verify existing storage'); }});
  const {fixture, elements} = await boot({localApp, storage, initialize: false});
  await assert.rejects(fixture.initialize(), /Cannot verify existing storage/);
  assert.deepEqual([...storage.records], entries);
  assert.match(elements.get('chat-list-warning').textContent, /Existing data and saved identities were left untouched/);
});
