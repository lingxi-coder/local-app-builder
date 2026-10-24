const $ = (id) => document.getElementById(id);
const app = await loadHostSdk();
const draftTtlMs = 24 * 60 * 60 * 1000;
function showRecoveryWarning() {
  $('recovery-warning').textContent = 'Offline recovery is unavailable in this browser. Keep this page open while the host is paused.';
  $('recovery-warning').hidden = false;
}
window.addEventListener('localApp:recovery-unavailable', showRecoveryWarning);
let info = null;
let chat = null;
let unsubscribe = null;
let selectedFiles = [];
let pendingDraft = null;
let busy = false;
let drafts = null;

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
      const request = indexedDB.open('attachment-review-drafts', 1);
      request.onupgradeneeded = () => {
        if (!request.result.objectStoreNames.contains('drafts')) request.result.createObjectStore('drafts', {keyPath: 'key'});
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
        } else resolve(record?.request || null);
      };
      tx.oncomplete = () => db.close();
      tx.onerror = () => { db.close(); reject(tx.error || new Error('Could not read the saved request.')); };
    });
  }
  async put(request) {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      tx.objectStore('drafts').put({key: this.key, request, createdAt: Date.now()});
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
function createRequestId() {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID();
  if (typeof globalThis.crypto?.getRandomValues === 'function') {
    const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
    return [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
  }
  return `attachment-review-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
function updateControls() {
  const hasChat = Boolean(chat);
  $('send').disabled = !app || !hasChat || busy || (!pendingDraft && !($('prompt').value.trim() || selectedFiles.length));
  $('attach').disabled = busy || Boolean(pendingDraft);
  $('prompt').disabled = busy || Boolean(pendingDraft);
  $('use-skill').disabled = busy || Boolean(pendingDraft);
  $('chat-list').disabled = busy || Boolean(pendingDraft);
  $('new-chat').disabled = busy || Boolean(pendingDraft);
  $('cancel').hidden = !busy || !chat;
  $('clear-draft').hidden = !pendingDraft || busy;
  $('archive-chat').disabled = busy || !hasChat || Boolean(pendingDraft);
  $('delete-chat').disabled = busy || !hasChat || Boolean(pendingDraft);
  $('draft').hidden = !pendingDraft;
  if (pendingDraft) $('draft').textContent = `A send is saved for explicit retry with request ID ${pendingDraft.requestId}. Refresh will not send it automatically.`;
}

function renderMessages(messages) {
  const container = $('messages');
  container.replaceChildren();
  for (const message of messages) {
    const row = document.createElement('article');
    row.className = 'message';
    row.dataset.role = message.role;
    const heading = document.createElement('h2');
    heading.textContent = message.role === 'assistant' ? 'Assistant' : 'You';
    const body = document.createElement('p');
    body.textContent = message.text;
    row.append(heading, body);
    if (message.attachments?.length) {
      const files = document.createElement('p');
      files.className = 'attachments';
      files.textContent = message.attachments.map((item) => `${item.name} · ${item.mediaType} · ${item.size} bytes`).join('\n');
      row.append(files);
    }
    container.append(row);
  }
}

async function refreshHistory() {
  if (!chat) { renderMessages([]); return; }
  const page = await chat.history({limit: 50});
  renderMessages(page.messages || []);
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
  } catch (error) { setStatus(error.message); }
  updateControls();
}

async function loadChatList(preferredId) {
  const chats = await app.agent.chats.list({limit: 50, includeArchived: false});
  const select = $('chat-list');
  select.replaceChildren(new Option('Choose a saved chat', ''));
  for (const summary of chats) select.add(new Option(summary.title || summary.chatId, summary.chatId));
  const savedId = preferredId || localStorage.getItem(`attachment-review:revision-${info.revision}:chatId`);
  const selected = chats.find((item) => item.chatId === savedId) || chats[0];
  if (selected) {
    select.value = selected.chatId;
    await openChat(selected.chatId);
  } else {
    chat = null;
    renderMessages([]);
  }
  updateControls();
}

async function openChat(chatId) {
  unsubscribe?.();
  chat = await app.agent.chats.open(chatId);
  localStorage.setItem(`attachment-review:revision-${info.revision}:chatId`, chat.id);
  unsubscribe = chat.subscribe((event) => {
    if (event.type === 'messages') void refreshHistory().catch((error) => setStatus(error.message));
    if (event.type === 'connection') setStatus(event.error?.message || 'The host is unavailable.');
  });
  await refreshHistory();
  setStatus('Chat history is connected.');
  updateControls();
}

async function createChat() {
  if (pendingDraft) throw new Error('Retry or finish the saved send before changing chats.');
  unsubscribe?.();
  chat = await app.agent.chats.create({title: 'Attachment review'});
  localStorage.setItem(`attachment-review:revision-${info.revision}:chatId`, chat.id);
  await loadChatList(chat.id);
  setStatus('New durable chat created.');
}

async function sendDraft() {
  if (!chat || busy) return;
  if (await clearExpiredDraft()) return;
  const request = pendingDraft || {
    requestId: createRequestId(),
    prompt: $('prompt').value.trim() || 'Review the attached files and list concrete questions.',
    attachments: selectedFiles,
    skills: $('use-skill').checked ? ['review'] : [],
    tools: []
  };
  try {
    const files = request.attachments || [];
    if (files.length > 8 || files.some((file) => file.size > 25 * 1024 * 1024)
        || files.reduce((size, file) => size + file.size, 0) > 50 * 1024 * 1024) {
      throw new Error('Choose up to eight files, 25 MiB each and 50 MiB total.');
    }
    if (files.some((file) => typeof File !== 'function' || !(file instanceof File))) {
      throw new Error('Attachments must be browser File objects.');
    }
    if (!pendingDraft) await drafts.put({chatId: chat.id, ...request});
    pendingDraft = {chatId: pendingDraft?.chatId || chat.id, ...request};
    busy = true;
    updateControls();
    setStatus('Sending one bounded request to the Local App host…');
    const {requestId, prompt, attachments, skills, tools} = pendingDraft;
    const input = {requestId, prompt, attachments: attachments || [], skills: skills || [], tools: tools || []};
    const handle = await app.agent.chats.open(pendingDraft.chatId);
    await handle.send(input);
    await drafts.delete();
    pendingDraft = null;
    selectedFiles = [];
    $('file-names').textContent = 'No files selected';
    $('prompt').value = '';
    await loadChatList(handle.id);
    setStatus('Reply saved in chat history.');
  } catch (error) {
    if (error.definitive && pendingDraft) {
      try {
        await drafts.delete();
        pendingDraft = null;
        selectedFiles = [];
        $('file-input').value = '';
        $('file-names').textContent = 'No files selected';
        $('prompt').value = '';
        setStatus(`${error.message} LingXi did not accept the request; the saved prompt and files were cleared.`);
      } catch (cleanupError) {
        setStatus(`${error.message} The host rejected the request, but its local draft could not be cleared: ${cleanupError.message}`);
      }
    } else {
      setStatus(`${error.message}${pendingDraft ? ' The unchanged request remains saved for an explicit retry.' : ''}`);
    }
  } finally {
    busy = false;
    updateControls();
  }
}

async function boot() {
  if (!app?.agent?.chats || typeof app.agent.chats.open !== 'function') {
    setStatus('Open this page in LingXi’s system-selected external browser to use persistent Agent Chats.');
    updateControls();
    return;
  }
  info = await app.info();
  if (info.recovery?.status === 'unavailable') showRecoveryWarning();
  drafts = new DraftStore(info.revision);
  const draft = await drafts.get();
  if (draft?.chatId && typeof draft.requestId === 'string') {
    pendingDraft = draft;
    selectedFiles = draft.attachments || [];
  }
  await loadChatList(pendingDraft?.chatId);
  if (!pendingDraft && drafts.expired) setStatus('The saved retry draft expired after 24 hours and was deleted. It was not sent again automatically.');
  if (pendingDraft) {
    $('prompt').value = pendingDraft.prompt;
    $('file-names').textContent = pendingDraft.attachments?.map((file) => file.name).join(', ') || 'No files selected';
    setStatus('A saved request is ready. Choose Retry to reuse its request ID and files.');
  }
  updateControls();
}

$('attach').addEventListener('click', () => void attachFiles());
$('file-input').addEventListener('change', () => {
  selectedFiles = Array.from($('file-input').files || []);
  $('file-names').textContent = selectedFiles.length ? selectedFiles.map((file) => file.name).join(', ') : 'No files selected';
  updateControls();
});
$('send').addEventListener('click', () => void sendDraft());
$('prompt').addEventListener('input', updateControls);
$('new-chat').addEventListener('click', () => void createChat().catch((error) => setStatus(error.message)));
$('chat-list').addEventListener('change', () => {
  if ($('chat-list').value) void openChat($('chat-list').value).catch((error) => setStatus(error.message));
});
$('cancel').addEventListener('click', () => {
  if (chat && busy) void chat.cancel().then(() => setStatus('Cancel requested.')).catch((error) => setStatus(error.message));
});
$('clear-draft').addEventListener('click', async () => {
  if (!pendingDraft || busy || !confirm('Forget this saved retry draft? Any admitted task remains in chat history and will not be cancelled.')) return;
  await drafts.delete();
  pendingDraft = null;
  selectedFiles = [];
  $('file-names').textContent = 'No files selected';
  $('prompt').value = '';
  setStatus('Saved retry draft removed. Any admitted task remains in durable chat history.');
  updateControls();
});
$('archive-chat').addEventListener('click', async () => {
  if (!chat || pendingDraft) return;
  await app.agent.chats.archive(chat.id);
  unsubscribe?.(); chat = null;
  await loadChatList();
});
$('delete-chat').addEventListener('click', async () => {
  if (!chat || pendingDraft || !confirm('Delete this chat and its saved history?')) return;
  const id = chat.id;
  await app.agent.chats.delete(id);
  unsubscribe?.(); chat = null;
  localStorage.removeItem(`attachment-review:revision-${info.revision}:chatId`);
  await loadChatList();
});
const checkDraftExpiry = () => {
  if (document.visibilityState === 'visible') void clearExpiredDraft().catch((error) => setStatus(error.message));
};
document.addEventListener('visibilitychange', checkDraftExpiry);
window.addEventListener('pageshow', checkDraftExpiry);
setInterval(checkDraftExpiry, 60 * 60 * 1000);
void boot().catch((error) => setStatus(error.message));
