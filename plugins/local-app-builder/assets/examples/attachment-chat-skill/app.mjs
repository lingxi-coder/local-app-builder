const $ = (id) => document.getElementById(id);
const app = await loadHostSdk();
const draftTtlMs = 24 * 60 * 60 * 1000;
let info = null;
let chat = null;
let client = null;
let selectedFiles = [];
let pendingDraft = null;
let busy = false;
let drafts = null;

function showRecoveryWarning() {
  $('recovery-warning').textContent = 'Offline recovery is unavailable in this browser. Keep this page open while the host is paused.';
  $('recovery-warning').hidden = false;
}
window.addEventListener('localApp:recovery-unavailable', showRecoveryWarning);

async function loadHostSdk() {
  if (!globalThis.localApp) {
    try { await import('/__local_app_sdk.js'); } catch {}
  }
  return globalThis.localApp || null;
}

class DraftStore {
  constructor(revision) { this.key = `attachment-review:revision-${revision}`; this.expired = false; }
  open() {
    if (!globalThis.indexedDB) return Promise.reject(new Error('This browser cannot persist a retry draft.'));
    return new Promise((resolve, reject) => {
      const request = indexedDB.open('attachment-review-drafts', 2);
      request.onupgradeneeded = (event) => {
        if (!request.result.objectStoreNames.contains('drafts')) request.result.createObjectStore('drafts', {keyPath: 'key'});
        else if (event.oldVersion < 2) request.transaction.objectStore('drafts').clear();
      };
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error || new Error('Could not open saved drafts.'));
      request.onblocked = () => reject(new Error('Saved drafts are busy in another tab.'));
    });
  }
  async get() {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      const store = tx.objectStore('drafts');
      const request = store.get(this.key);
      request.onsuccess = () => {
        const record = request.result;
        this.expired = false;
        if (record && (!Number.isFinite(record.createdAt) || record.createdAt > Date.now() + 5 * 60 * 1000
            || Date.now() - record.createdAt >= draftTtlMs)) {
          store.delete(this.key);
          this.expired = true;
          resolve(null);
        } else resolve(record?.draft || null);
      };
      tx.oncomplete = () => db.close();
      tx.onerror = () => { db.close(); reject(tx.error || new Error('Could not read the saved draft.')); };
    });
  }
  async put(draft) {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      tx.objectStore('drafts').put({key: this.key, draft, createdAt: Date.now()});
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error('Could not save the request.')); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error('Could not save the request.')); };
    });
  }
  async delete() {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      tx.objectStore('drafts').delete(this.key);
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error('Could not remove the saved request.')); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error('Could not remove the saved request.')); };
    });
  }
}

function setStatus(message) { $('status').textContent = message; }
function createRunId() {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID();
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  return [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
}

async function clearExpiredDraft() {
  if (!drafts || !pendingDraft || busy) return false;
  const saved = await drafts.get();
  if (saved || !drafts.expired) return false;
  pendingDraft = null;
  selectedFiles = [];
  $('file-input').value = '';
  $('file-names').textContent = 'No files selected';
  $('prompt').value = '';
  setStatus('The saved retry draft expired after 24 hours and was deleted. It was not sent again automatically.');
  updateControls();
  return true;
}

function updateControls() {
  const hasChat = Boolean(chat && client);
  $('send').disabled = !app || !hasChat || busy || (!pendingDraft && !($('prompt').value.trim() || selectedFiles.length));
  $('send').textContent = pendingDraft ? 'Retry saved turn' : 'Send';
  $('attach').disabled = busy || Boolean(pendingDraft);
  $('prompt').disabled = busy || Boolean(pendingDraft);
  $('use-skill').disabled = busy || Boolean(pendingDraft);
  $('chat-list').disabled = busy || Boolean(pendingDraft);
  $('new-chat').disabled = busy || Boolean(pendingDraft);
  $('cancel').hidden = !busy || !client;
  $('clear-draft').hidden = !pendingDraft || busy;
  $('archive-chat').disabled = busy || !chat || Boolean(pendingDraft);
  $('delete-chat').disabled = busy || !chat || Boolean(pendingDraft);
  $('draft').hidden = !pendingDraft;
  if (pendingDraft) $('draft').textContent = `A turn is saved for explicit retry with run ID ${pendingDraft.runId}. Refresh will not send it automatically.`;
}

function messageParts(message) {
  if (typeof message.content === 'string') return [{type: 'text', text: message.content}];
  return Array.isArray(message.content) ? message.content : [];
}

function renderMessages(messages) {
  const container = $('messages');
  container.replaceChildren();
  for (const message of messages) {
    const row = document.createElement('article');
    row.className = 'message';
    row.dataset.role = message.role;
    const heading = document.createElement('h2');
    heading.textContent = message.role === 'assistant' ? 'Assistant' : message.role === 'user' ? 'You' : message.role;
    row.append(heading);
    const parts = messageParts(message);
    const text = parts.filter((part) => part?.type === 'text').map((part) => part.text).join('\n');
    if (text) {
      const body = document.createElement('p');
      body.textContent = text;
      row.append(body);
    }
    const media = parts.filter((part) => part && part.type !== 'text');
    if (media.length) {
      const files = document.createElement('p');
      files.className = 'attachments';
      files.textContent = media.map((part) => {
        const metadata = part.metadata && typeof part.metadata === 'object' ? part.metadata : {};
        const localFile = metadata['com.lingxi.localApp'];
        return localFile?.filename || `${part.type} attachment`;
      }).join('\n');
      row.append(files);
    }
    container.append(row);
  }
}

async function fetchHistory(chatHandle) {
  if (!chatHandle) return [];
  const messages = [];
  let cursor;
  do {
    const options = {limit: 50};
    if (cursor) options.cursor = cursor;
    const page = await chatHandle.history(options);
    if (!page || !Array.isArray(page.messages)) throw new Error('The host returned invalid AG-UI history.');
    messages.push(...page.messages);
    cursor = page.nextCursor;
  } while (cursor);
  return messages;
}

async function readHistory() {
  if (!chat) return [];
  const messages = await fetchHistory(chat);
  client.setMessages(messages);
  return messages;
}

function subscriberForRun(agent = client, priorMessages = []) {
  let failure = null;
  let outcome;
  const subscriber = {
    onEvent({event}) {
      if (event.type === 'RUN_STARTED') setStatus('AG-UI run started.');
      if (event.type === 'STEP_STARTED') setStatus('The host is processing this turn…');
      if (event.type === 'TEXT_MESSAGE_CONTENT') {
        const knownIds = new Set(priorMessages.map((message) => message.id));
        renderMessages([...priorMessages, ...agent.messages.filter((message) => !knownIds.has(message.id))]);
      }
    },
    onRunErrorEvent({event}) { failure = event; },
    onRunFinishedEvent({outcome: terminalOutcome}) { outcome = terminalOutcome; },
  };
  return {subscriber, getFailure: () => failure, getOutcome: () => outcome};
}

async function refreshHistory() {
  const messages = await readHistory();
  renderMessages(messages);
}

async function attachFiles() {
  if (!app) { $('file-input').click(); return; }
  try {
    selectedFiles = typeof app.files?.pick === 'function'
      ? await app.files.pick({multiple: true, accept: '.pdf,.txt,.md,.csv,image/*'})
      : await new Promise((resolve) => {
        const input = $('file-input');
        input.value = '';
        input.onchange = () => resolve(Array.from(input.files || []));
        input.click();
      });
    $('file-names').textContent = selectedFiles.length ? selectedFiles.map((file) => file.name).join(', ') : 'No files selected';
  } catch (failure) { setStatus(failure.message); }
  updateControls();
}

function chatIdentity(summary) { return summary.chatId; }
async function loadChatList(preferredId) {
  const chats = [];
  let cursor;
  do {
    const options = {limit: 50, includeArchived: false};
    if (cursor) options.cursor = cursor;
    const page = await app.agent.chats.list(options);
    if (!page || !Array.isArray(page.chats)) throw new Error('The host returned invalid AG-UI chat summaries.');
    chats.push(...page.chats);
    if (page.nextCursor !== undefined && (typeof page.nextCursor !== 'string' || !page.nextCursor || page.nextCursor === cursor)) {
      throw new Error('The host returned an invalid AG-UI chat cursor.');
    }
    cursor = page.nextCursor;
  } while (cursor);
  const select = $('chat-list');
  select.replaceChildren(new Option('Choose a saved chat', ''));
  for (const summary of chats) {
    const id = chatIdentity(summary);
    select.add(new Option(summary.title || id, id));
  }
  const savedId = preferredId || localStorage.getItem(`attachment-review:revision-${info.revision}:chatId`);
  const selected = chats.find((item) => chatIdentity(item) === savedId) || chats[0];
  if (selected) {
    select.value = chatIdentity(selected);
    await openChat(chatIdentity(selected));
  } else {
    chat = null;
    client = null;
    renderMessages([]);
  }
  updateControls();
}

async function openChat(chatId) {
  chat = await app.agent.chats.open(chatId);
  if (chat.id !== chatId) throw new Error('The host returned a different Chat identity.');
  const messages = await fetchHistory(chat);
  client = app.agent.createClient({threadId: chat.id, initialMessages: messages});
  localStorage.setItem(`attachment-review:revision-${info.revision}:chatId`, chat.id);
  renderMessages(messages);
  setStatus('Chat history restored. No model request was started.');
  updateControls();
}

async function recoverSavedRun(draft, history) {
  const task = await app.agent.tasks.get(draft.runId);
  if (!task || task.runId !== draft.runId || task.threadId !== chat.id) {
    throw new Error('The host could not confirm the saved run belongs to this Chat. The draft remains saved.');
  }

  // A refreshed page has no lastRunId. Use a clean client and reconnect by the durable task identity.
  const recoveryClient = app.agent.createClient({threadId: task.threadId});
  const recovery = subscriberForRun(recoveryClient, history);
  await recoveryClient.connectAgent({runId: draft.runId}, recovery.subscriber);
  await refreshHistory();

  if (recovery.getFailure()) {
    const failure = recovery.getFailure();
    setStatus(`The saved AG-UI run failed: ${failure.message || 'The host reported RUN_ERROR.'}${failure.code ? ` (${failure.code})` : ''} The original input and run ID remain saved; no new model request was started.`);
    return false;
  }
  if (recovery.getOutcome() !== 'success') {
    const outcome = recovery.getOutcome();
    const reason = outcome === 'cancelled' ? 'The saved run was cancelled.'
      : outcome === 'interrupt' ? 'The saved run paused for an interrupt.'
        : 'The host did not report a successful terminal event.';
    setStatus(`${reason} The original input and run ID remain saved; no new model request was started.`);
    return false;
  }

  if (drafts) await drafts.delete();
  pendingDraft = null;
  selectedFiles = [];
  $('file-names').textContent = 'No files selected';
  $('prompt').value = '';
  setStatus('The saved turn finished successfully and was restored from Chat history. No new model request was started.');
  return true;
}

async function createChat() {
  if (pendingDraft) throw new Error('Retry or forget the saved turn before changing Chats.');
  const created = await app.agent.chats.create({title: 'Attachment review'});
  await loadChatList(created.id);
  setStatus('New persistent AG-UI thread created.');
}

async function messageForDraft(draft) {
  const parts = await Promise.all(draft.files.map((file) => app.agent.file(file)));
  return {
    id: draft.messageId,
    role: 'user',
    content: [
      {type: 'text', text: draft.prompt},
      ...parts,
    ],
  };
}

async function sendTurn() {
  if (!client || busy) return;
  if (await clearExpiredDraft()) return;
  const draft = pendingDraft || {
    chatId: chat.id,
    runId: createRunId(),
    messageId: createRunId(),
    prompt: $('prompt').value.trim() || 'Review the attached files and list concrete questions.',
    files: selectedFiles,
    skills: $('use-skill').checked ? ['review'] : [],
  };
  if (draft.chatId !== chat.id) throw new Error('Open the saved Chat that owns this turn before retrying.');
  if (draft.files.length > 8 || draft.files.some((file) => file.size > 25 * 1024 * 1024)
      || draft.files.reduce((size, file) => size + file.size, 0) > 50 * 1024 * 1024) {
    throw new Error('Choose up to eight files, 25 MiB each and 50 MiB total.');
  }
  if (draft.files.some((file) => typeof File !== 'function' || !(file instanceof File))) {
    throw new Error('Attachments must be browser File objects.');
  }

  try {
    if (!pendingDraft && drafts) await drafts.put(draft);
    pendingDraft = draft;
    busy = true;
    updateControls();
    setStatus('Preparing official AG-UI user message…');

    const history = await readHistory();
    const alreadyInHistory = history.some((message) => message.id === draft.messageId);
    if (alreadyInHistory) {
      // The prior request may already be admitted. Reconnect and observe it; never submit again on refresh.
      await recoverSavedRun(draft, history);
      return;
    }

    const message = await messageForDraft(draft);
    // The Chat's Harness session already owns canonical history. Keep the full
    // transcript for display, but send only this new user message to the run.
    client.setMessages([message]);
    const run = subscriberForRun(client, history);
    setStatus('Sending the new turn in this persistent thread…');
    const response = await client.runAgent({
      runId: draft.runId,
      forwardedProps: {
        'com.lingxi.localApp': {
          version: 1,
          tools: [],
          skills: draft.skills,
          maxTurns: 6,
          timeoutMs: 120000,
        },
      },
    }, run.subscriber);
    if (run.getFailure()) {
      const failure = new Error(`${run.getFailure().message}${run.getFailure().code ? ` (${run.getFailure().code})` : ''}`);
      failure.runFailed = true;
      throw failure;
    }
    if (run.getOutcome() === 'cancelled') throw new Error('The host confirmed that this turn was cancelled.');
    else if (run.getOutcome() === 'interrupt') throw new Error('This turn is paused. Return to LingXi to handle its approval or question.');
    else if (run.getOutcome() === 'success') setStatus(`Turn finished with ${response.newMessages.length} new AG-UI message(s).`);
    else throw new Error('The host did not report a successful AG-UI terminal outcome.');

    await refreshHistory();
    if (drafts) await drafts.delete();
    pendingDraft = null;
    selectedFiles = [];
    $('file-names').textContent = 'No files selected';
    $('prompt').value = '';
  } catch (failure) {
    if (pendingDraft) await refreshHistory().catch(() => {});
    setStatus(`${failure.message}${pendingDraft ? ' The exact input and run ID remain saved. No new model request was started automatically.' : ''}`);
  } finally {
    busy = false;
    updateControls();
  }
}

async function boot() {
  if (!app?.agent?.chats || typeof app.agent.createClient !== 'function') {
    setStatus('Open this page in LingXi’s system-selected external browser to use persistent AG-UI threads.');
    updateControls();
    return;
  }
  info = await app.info();
  if (info.recovery?.status === 'unavailable') showRecoveryWarning();
  drafts = globalThis.indexedDB ? new DraftStore(info.revision) : null;
  pendingDraft = drafts ? await drafts.get() : null;
  if (pendingDraft?.files?.length) selectedFiles = pendingDraft.files;
  await loadChatList(pendingDraft?.chatId);
  if (!pendingDraft && drafts?.expired) setStatus('The saved retry draft expired after 24 hours and was deleted. It was not sent again automatically.');
  else if (!drafts) setStatus('IndexedDB retry storage is unavailable. Online Agent use still works; a page refresh cannot restore a local draft.');
  if (pendingDraft) {
    $('prompt').value = pendingDraft.prompt;
    $('file-names').textContent = pendingDraft.files?.map((file) => file.name).join(', ') || 'No files selected';
    setStatus('A saved turn is ready. Choose Retry to reuse its run ID and original files.');
  }
  updateControls();
}

$('attach').addEventListener('click', () => void attachFiles());
$('file-input').addEventListener('change', () => {
  selectedFiles = Array.from($('file-input').files || []);
  $('file-names').textContent = selectedFiles.length ? selectedFiles.map((file) => file.name).join(', ') : 'No files selected';
  updateControls();
});
$('send').addEventListener('click', () => void sendTurn());
$('prompt').addEventListener('input', updateControls);
$('new-chat').addEventListener('click', () => void createChat().catch((failure) => setStatus(failure.message)));
$('chat-list').addEventListener('change', () => {
  if ($('chat-list').value) void openChat($('chat-list').value).catch((failure) => setStatus(failure.message));
});
$('cancel').addEventListener('click', () => {
  if (!client || !busy) return;
  client.abortRun();
  setStatus('Cancellation requested; waiting for the host terminal outcome.');
});
$('clear-draft').addEventListener('click', async () => {
  if (!pendingDraft || busy || !confirm('Forget this saved turn? Any admitted run remains in Chat history and will not be cancelled.')) return;
  if (drafts) await drafts.delete();
  pendingDraft = null;
  selectedFiles = [];
  $('file-names').textContent = 'No files selected';
  $('prompt').value = '';
  setStatus('Saved retry draft removed. Any admitted run remains in durable Chat history.');
  updateControls();
});
$('archive-chat').addEventListener('click', async () => {
  if (!chat || pendingDraft) return;
  await app.agent.chats.archive(chat.id);
  chat = null;
  client = null;
  await loadChatList();
});
$('delete-chat').addEventListener('click', async () => {
  if (!chat || pendingDraft || !confirm('Delete this Chat and its saved history?')) return;
  const id = chat.id;
  await app.agent.chats.delete(id);
  chat = null;
  client = null;
  localStorage.removeItem(`attachment-review:revision-${info.revision}:chatId`);
  await loadChatList();
});
const checkDraftExpiry = () => {
  if (document.visibilityState === 'visible') void clearExpiredDraft().catch((failure) => setStatus(failure.message));
};
document.addEventListener('visibilitychange', checkDraftExpiry);
window.addEventListener('pageshow', checkDraftExpiry);
setInterval(checkDraftExpiry, 60 * 60 * 1000);
void boot().catch((failure) => setStatus(failure.message));
