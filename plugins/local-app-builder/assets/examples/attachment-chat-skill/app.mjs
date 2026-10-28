const $ = (id) => document.getElementById(id);
const app = await loadHostSdk();
const draftTtlMs = 24 * 60 * 60 * 1000;
let info = null;
let chat = null;
let client = null;
let selectedFiles = [];
let pendingDraft = null;
let pendingRequestMissingRunId = null;
let busy = false;
let activeRunClient = null;
let drafts = null;
let ready = false;
let chatCreationKey = null;
let pendingCreation = null;
let creationStorageProblem = null;
let savedCreationReceipts = [];

window.addEventListener('localApp:chat-creation-pending', (event) => {
  if (!pendingCreation || event?.detail?.requestId !== pendingCreation.requestId) return;
  setStatus('Chat creation is still waiting; the request may already have arrived. Keep its original ID. Use Open LingXi in the host recovery prompt if needed, then reopen this App. No retry or navigation was started automatically.');
});

function showRecoveryWarning() {
  $('recovery-warning').textContent = 'Offline recovery is unavailable in this browser. Keep this page open while the host is paused.';
  $('recovery-warning').hidden = false;
}
window.addEventListener('localApp:recovery-unavailable', showRecoveryWarning);

function showPendingRequestMissing() {
  if (typeof pendingRequestMissingRunId !== 'string') return;
  if (pendingDraft && pendingDraft.runId !== pendingRequestMissingRunId) return;
  setStatus(`This browser context cannot access the saved input for run ${pendingRequestMissingRunId}. Return to the original tab or reopen this App in a browser context that shares its storage. Nothing was resent. Review the input before explicitly starting a new request.`);
}
window.addEventListener('localApp:agent-handoff-error', (event) => {
  const detail = event?.detail;
  if (detail?.code !== 'pending_request_missing' || typeof detail.runId !== 'string' || !detail.runId) return;
  pendingRequestMissingRunId = detail.runId;
  showPendingRequestMissing();
});

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
async function readChatData(read) {
  try { return await read(); }
  catch (failure) {
    $('chat-list-warning').textContent = 'Agent or Chat data is unavailable or could not be verified. Existing data and saved identities were left untouched. Review the storage status in LingXi; this App will not automatically delete, reset or rebuild it.';
    $('chat-list-warning').hidden = false;
    throw failure;
  }
}
function createRunId() {
  if (typeof globalThis.crypto?.randomUUID === 'function') return globalThis.crypto.randomUUID();
  const bytes = globalThis.crypto.getRandomValues(new Uint8Array(16));
  return [...bytes].map((value) => value.toString(16).padStart(2, '0')).join('');
}

async function clearExpiredDraft(allowBusy = false) {
  if (!drafts || !pendingDraft || (busy && !allowBusy)) return false;
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

function mobileAgentStorageUnavailable() {
  return info?.platform === 'mobile-browser' && !globalThis.indexedDB;
}

function hasOriginalMessage(draft) {
  const message = draft?.message;
  return typeof draft?.runId === 'string' && typeof draft.messageId === 'string'
    && Array.isArray(draft.skills) && message?.id === draft.messageId && message.role === 'user'
    && Array.isArray(message.content) && message.content.length > 0
    && message.content.every((part) => part && (part.type === 'text'
      ? typeof part.text === 'string'
      : (part.type === 'image' || part.type === 'document') && typeof part.id === 'string'
        && part.source?.type === 'data' && typeof part.source.value === 'string'));
}

function updateControls() {
  const hasChat = Boolean(chat && client);
  $('send').disabled = !app || !hasChat || busy || mobileAgentStorageUnavailable()
    || (!pendingDraft && !($('prompt').value.trim() || selectedFiles.length));
  $('send').textContent = pendingDraft ? (hasOriginalMessage(pendingDraft) ? 'Retry saved turn' : 'Check saved run') : 'Send';
  $('attach').disabled = busy || Boolean(pendingDraft);
  $('prompt').disabled = busy || Boolean(pendingDraft);
  $('use-skill').disabled = busy || Boolean(pendingDraft);
  $('chat-list').disabled = busy || Boolean(pendingDraft);
  $('check-chats').disabled = !ready || busy;
  $('creation-receipts').disabled = !ready || busy || Boolean(pendingDraft);
  $('restore-creation').disabled = !ready || busy || Boolean(pendingDraft) || !$('creation-receipts').value;
  $('new-chat').disabled = !ready || !app || busy || Boolean(pendingDraft) || Boolean(pendingCreation) || Boolean(creationStorageProblem);
  $('check-creation').hidden = !pendingCreation;
  $('retry-creation').hidden = !pendingCreation;
  $('check-creation').disabled = $('retry-creation').disabled = !ready || busy || Boolean(pendingDraft);
  $('forget-creation').hidden = !pendingCreation && !creationStorageProblem;
  $('forget-creation').disabled = busy;
  $('creation-draft').hidden = !pendingCreation && !creationStorageProblem;
  $('creation-draft').textContent = creationStorageProblem || (pendingCreation
    ? `Chat creation ${pendingCreation.requestId} is saved. Check its status, or explicitly retry the same saved request. A missing result does not mean creation failed.` : '');
  $('cancel').hidden = !activeRunClient;
  $('clear-draft').hidden = !pendingDraft || busy;
  $('archive-chat').disabled = busy || !chat || Boolean(pendingDraft);
  $('delete-chat').disabled = busy || !chat || Boolean(pendingDraft);
  $('draft').hidden = !pendingDraft;
  if (pendingDraft) $('draft').textContent = hasOriginalMessage(pendingDraft)
    ? `A turn is saved with run ID ${pendingDraft.runId}. Retry checks the host first and may resend the unchanged input. Refresh will not send it automatically.`
    : `The input for run ${pendingDraft.runId} is incomplete. Only a read-only host check is available.`;
}

function draftFileNames(draft) {
  if (Array.isArray(draft.message?.content)) {
    return draft.message.content.filter((part) => part?.type === 'image' || part?.type === 'document')
      .map((part) => part.metadata?.['com.lingxi.localApp']?.filename || 'Attachment');
  }
  return [];
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
    const page = await readChatData(() => chatHandle.history(options));
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

async function getSavedTask(draft) {
  for (let attempt = 0; attempt < 3; attempt++) {
    try {
      return await app.agent.tasks.get(draft.runId);
    } catch (failure) {
      if (failure?.code !== 'task_not_found') throw failure;
      if (attempt < 2) {
        setStatus('The saved run is not visible yet. Checking the same run again without sending a model request…');
        await new Promise((resolve) => setTimeout(resolve, 250 * (attempt + 1)));
      }
    }
  }
  const failure = new Error(`Saved run ${draft.runId} is not visible yet. The original input and run ID remain saved; retry this read-only check. No model failure was confirmed and no new request was sent.`);
  failure.code = 'task_not_found';
  throw failure;
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
async function loadChatList(preferredId, confirmedChat = null) {
  const chats = [];
  const unavailableIds = new Set();
  let anyUncertainChats = false;
  let allPagesExplicitlyClear = true;
  let cursor;
  do {
    const options = {limit: 50, includeArchived: false};
    if (cursor) options.cursor = cursor;
    const page = await readChatData(() => app.agent.chats.list(options));
    if (!page || !Array.isArray(page.chats)) throw new Error('The host returned invalid AG-UI chat summaries.');
    if ((page.hasUncertainChats !== undefined && typeof page.hasUncertainChats !== 'boolean')
        || (page.unavailableChats !== undefined && (!Array.isArray(page.unavailableChats)
          || page.unavailableChats.some((item) => !item || typeof item.chatId !== 'string' || !item.chatId
            || item.code !== 'chat_storage_uncertain')))) {
      throw new Error('The host returned invalid unavailable Chat status.');
    }
    // IDs belong to this page; the boolean describes uncertainty anywhere in this App.
    for (const item of page.unavailableChats || []) unavailableIds.add(item.chatId);
    anyUncertainChats ||= page.hasUncertainChats === true;
    allPagesExplicitlyClear &&= page.hasUncertainChats === false;
    chats.push(...page.chats);
    if (page.nextCursor !== undefined && (typeof page.nextCursor !== 'string' || !page.nextCursor || page.nextCursor === cursor)) {
      throw new Error('The host returned an invalid AG-UI chat cursor.');
    }
    cursor = page.nextCursor;
  } while (cursor);
  // Creation status can confirm an ID before the list reflects it.
  if (confirmedChat && !chats.some((item) => chatIdentity(item) === confirmedChat.id)) {
    chats.push({chatId: confirmedChat.id, title: pendingCreation?.title});
  }
  const select = $('chat-list');
  select.replaceChildren(new Option('Choose a saved chat', ''));
  for (const summary of chats) {
    const id = chatIdentity(summary);
    select.add(new Option(summary.title || id, id));
  }
  const savedId = preferredId || localStorage.getItem(`attachment-review:revision-${info.revision}:chatId`);
  // A missing or unavailable saved ID is not permission to switch to another Chat.
  const selected = savedId ? chats.find((item) => chatIdentity(item) === savedId) : chats[0];
  if (selected) {
    select.value = chatIdentity(selected);
    await openChat(chatIdentity(selected));
  } else {
    select.value = '';
    chat = null;
    client = null;
    renderMessages([]);
  }
  const warnings = [];
  if (anyUncertainChats || unavailableIds.size) {
    warnings.push('Some Chats in this App need their status checked. Their saved data may still exist.');
  } else if (!allPagesExplicitlyClear) {
    warnings.push('The host did not confirm whether any Chats in this App need recovery. A missing warning does not establish that all saved data is healthy.');
  }
  if (unavailableIds.size) warnings.push(`Unavailable IDs on the pages read: ${[...unavailableIds].join(', ')}.`);
  if (savedId && !selected) warnings.push(`Saved Chat ${savedId} is not currently available in this list. Its ID was kept; no other Chat was selected.`);
  if (warnings.length) warnings.push('Check saved Chats again or review their status in LingXi. Nothing was replayed and no replacement Chat was created.');
  $('chat-list-warning').textContent = warnings.join(' ');
  $('chat-list-warning').hidden = warnings.length === 0;
  updateControls();
}

async function checkSavedChats() {
  if (!ready || busy) return;
  busy = true;
  updateControls();
  try { await loadChatList(pendingDraft?.chatId || chat?.id); }
  catch (failure) { setStatus(`Saved Chat status is unavailable: ${failure.message}. No Chat was created and no Agent run was started.`); }
  finally { busy = false; updateControls(); }
}

async function openChat(chatId) {
  chat = await readChatData(() => app.agent.chats.open(chatId));
  if (chat.id !== chatId) throw new Error('The host returned a different Chat identity.');
  const messages = await fetchHistory(chat);
  client = app.agent.createClient({threadId: chat.id, initialMessages: messages});
  localStorage.setItem(`attachment-review:revision-${info.revision}:chatId`, chat.id);
  renderMessages(messages);
  setStatus('Chat history restored. No model request was started.');
  updateControls();
}

async function recoverSavedRun(draft, history, savedTask) {
  const task = savedTask || await getSavedTask(draft);
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
    setStatus(`The saved AG-UI run failed: ${failure.message || 'The host reported RUN_ERROR.'}${failure.code ? ` (${failure.code})` : ''} The draft and run ID remain available; no new model request was started.`);
    return false;
  }
  if (recovery.getOutcome() !== 'success') {
    const outcome = recovery.getOutcome();
    const reason = outcome === 'cancelled' ? 'The saved run was cancelled.'
      : outcome === 'interrupt' ? 'The saved run paused for an interrupt.'
        : 'The host did not report a successful terminal event.';
    setStatus(`${reason} The draft and run ID remain available; no new model request was started.`);
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

function originalCreationOptions(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)
      || typeof value.requestId !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(value.requestId)
      || Object.keys(value).some((key) => key !== 'requestId' && key !== 'title')
      || (Object.hasOwn(value, 'title') && typeof value.title !== 'string')) {
    throw new Error('The saved Chat creation ID or original input is missing or invalid. Review saved Chats before forgetting this record. Nothing was submitted or assigned a replacement ID.');
  }
  return Object.hasOwn(value, 'title') ? {requestId: value.requestId, title: value.title} : {requestId: value.requestId};
}

function restoreChatCreation() {
  try {
    if (!chatCreationKey) throw new Error('The host has not supplied this App identity and revision.');
    const saved = localStorage.getItem(chatCreationKey);
    pendingCreation = saved === null ? null : originalCreationOptions(JSON.parse(saved));
  } catch (failure) {
    creationStorageProblem = `Chat creation cannot be safely restored: ${failure.message}`;
  }
}

async function loadCreationReceipts() {
  const panel = $('creation-receipts-panel');
  const select = $('creation-receipts');
  select.replaceChildren(new Option('Choose an exact saved creation ID', ''));
  select.value = '';
  savedCreationReceipts = [];
  try {
    // The SDK may reconcile its receipts on reload. This local read does not restore our UI or a previous Promise.
    const records = await app.agent.chats.pendingCreations();
    if (!Array.isArray(records)) throw new Error('The host returned invalid creation receipts.');
    savedCreationReceipts = records.map((record) => {
      if (!record || !['pending', 'accepted', 'deleted', 'rejected'].includes(record.state)) {
        throw new Error('A saved creation receipt has an invalid state.');
      }
      return {options: originalCreationOptions({requestId: record.requestId,
        ...(Object.hasOwn(record, 'title') ? {title: record.title} : {})}), state: record.state};
    });
    for (const record of savedCreationReceipts) {
      select.add(new Option(`${record.options.requestId} (${record.state})`, record.options.requestId));
    }
    panel.hidden = savedCreationReceipts.length === 0;
    $('creation-receipts-note').textContent = 'These receipts belong to this App and revision. Choose an exact ID to check it read-only. No Chat is selected or created from this list automatically.';
  } catch (failure) {
    panel.hidden = false;
    $('creation-receipts-note').textContent = `Saved creation receipts are unavailable: ${failure.message}. Existing data was kept; no new request was sent.`;
  }
  updateControls();
}

async function restoreSelectedCreation() {
  if (!ready || busy || pendingDraft || creationStorageProblem) return;
  const saved = savedCreationReceipts.find((record) => record.options.requestId === $('creation-receipts').value);
  if (!saved) { setStatus('Choose the exact saved creation ID before checking it.'); return; }
  busy = true;
  updateControls();
  try {
    restoreChatCreation();
    if (creationStorageProblem) throw new Error(creationStorageProblem);
    const options = originalCreationOptions(saved.options);
    if (pendingCreation && JSON.stringify(originalCreationOptions(pendingCreation)) !== JSON.stringify(options)) {
      throw new Error('Another original creation request is saved. Check or review it before choosing a different ID.');
    }
    localStorage.setItem(chatCreationKey, JSON.stringify(options));
    pendingCreation = options;
    await checkCreationStatus(); // An explicit selection restores our display; it never creates the Chat.
  } catch (failure) { setStatus(`${failure.message}. No creation was submitted and no replacement ID was generated.`); }
  finally { busy = false; updateControls(); }
}

async function acceptCreatedChat(created) {
  if (created?.id !== pendingCreation?.requestId) throw new Error('The host returned a different Chat creation identity. The original request remains saved.');
  const original = JSON.stringify(originalCreationOptions(pendingCreation));
  await loadChatList(created.id, created);
  // Do not clear an intent saved by another page while the history read was pending.
  if (localStorage.getItem(chatCreationKey) === original) localStorage.removeItem(chatCreationKey);
  pendingCreation = null;
  setStatus('The saved Chat creation is confirmed. Its history was opened without starting an Agent run.');
}

async function checkCreationStatus() {
  if (!pendingCreation) throw new Error(creationStorageProblem || 'No saved Chat creation ID is available. Review saved Chats before explicitly creating a new one.');
  const created = await readChatData(() => app.agent.chats.creationStatus(originalCreationOptions(pendingCreation)));
  if (created) await acceptCreatedChat(created);
  else setStatus('The saved Chat is not visible yet. The original request may still be in flight. Nothing was resubmitted; check again or explicitly retry the same request.');
  return created;
}

async function checkChatCreation() {
  if (!ready || busy || pendingDraft) return;
  busy = true;
  updateControls();
  try { await checkCreationStatus(); }
  catch (failure) { setStatus(`${failure.message}${failure.code ? ` (${failure.code})` : ''} The creation outcome remains unconfirmed. Nothing was submitted.`); }
  finally { busy = false; updateControls(); }
}

async function createChat({retry = false} = {}) {
  if (!ready || busy || pendingDraft) return;
  busy = true;
  updateControls();
  try {
    // Re-read before a user action so a saved intent from another tab is not knowingly overwritten.
    restoreChatCreation();
    if (creationStorageProblem) throw new Error(creationStorageProblem);
    if (retry) {
      if (!pendingCreation) throw new Error('No saved Chat creation ID is available. A retry cannot assign a replacement ID.');
      if (await checkCreationStatus()) return;
    } else {
      if (pendingCreation) throw new Error('Check the saved Chat creation or explicitly retry it before creating another Chat.');
      const options = originalCreationOptions({requestId: globalThis.crypto.randomUUID(), title: 'Attachment review'});
      // Saving must succeed before the first POST. Preserve title omission, whitespace and all IDs on retry.
      localStorage.setItem(chatCreationKey, JSON.stringify(options));
      pendingCreation = options;
    }
    const created = await app.agent.chats.create(originalCreationOptions(pendingCreation));
    await acceptCreatedChat(created);
  } catch (failure) {
    setStatus(`${failure.message}${failure.code ? ` (${failure.code})` : ''} No replacement request was sent.`);
    // A failed POST may have committed. Only inspect the original request; never retry it here.
    if (pendingCreation) {
      try { await checkCreationStatus(); }
      catch (checkFailure) { setStatus(`${failure.message}${failure.code ? ` (${failure.code})` : ''} Status is unavailable: ${checkFailure.message}. The exact creation request remains saved; nothing was resubmitted.`); }
    }
  } finally { busy = false; updateControls(); }
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
  if (!client || busy || mobileAgentStorageUnavailable()) return;
  // Lock before any asynchronous work so repeated clicks cannot create two run IDs.
  busy = true;
  updateControls();
  try {
    if (await clearExpiredDraft(true)) return;
    const retrying = Boolean(pendingDraft);
    const draft = pendingDraft || {
      chatId: chat.id,
      runId: createRunId(),
      messageId: createRunId(),
      prompt: $('prompt').value.trim() || 'Review the attached files and list concrete questions.',
      files: selectedFiles,
      skills: $('use-skill').checked ? ['review'] : [],
    };
    if (draft.chatId !== chat.id) throw new Error('Open the saved Chat that owns this turn before retrying.');
    if (!retrying) {
      if (draft.files.length > 8 || draft.files.some((file) => file.size > 25 * 1024 * 1024)
          || draft.files.reduce((size, file) => size + file.size, 0) > 50 * 1024 * 1024) {
        throw new Error('Choose up to eight files, 25 MiB each and 50 MiB total.');
      }
      if (draft.files.some((file) => typeof File !== 'function' || !(file instanceof File))) {
        throw new Error('Attachments must be browser File objects.');
      }
      // agent.file generates a new ContentPart ID each time. Store the original
      // message, not just File objects, so a retry preserves the exact input.
      draft.message = await messageForDraft(draft);
      delete draft.files;
      if (drafts) await drafts.put(draft);
    }
    pendingDraft = draft;
    updateControls();
    setStatus('Preparing official AG-UI user message…');

    const history = await readHistory();
    const alreadyInHistory = history.some((message) => message.id === draft.messageId);
    if (retrying || alreadyInHistory) {
      let task;
      try { task = await getSavedTask(draft); }
      catch (failure) {
        // A history entry proves that this turn existed, even if its task is
        // temporarily not visible. A lookup miss is otherwise still uncertain.
        if (failure?.code !== 'task_not_found' || alreadyInHistory) throw failure;
      }
      if (task) {
        await recoverSavedRun(draft, history, task);
        return;
      }
      if (!hasOriginalMessage(draft)) {
        throw new Error('The saved draft does not contain the complete original input. Check again, or forget it and explicitly create a new turn after reviewing any possible effects.');
      }
      setStatus('The saved run is not visible yet. Retrying the unchanged input with the same run ID by your request…');
    }

    const message = draft.message;
    // The Chat's Harness session already owns canonical history. Keep the full
    // transcript for display, but send only this new user message to the run.
    client.setMessages([message]);
    const run = subscriberForRun(client, history);
    setStatus('Sending the new turn in this persistent thread…');
    activeRunClient = client;
    updateControls();
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
    const available = drafts ? 'saved in this browser' : 'available while this page stays open';
    const detail = hasOriginalMessage(pendingDraft) ? ` The exact input and run ID remain ${available}.`
      : pendingDraft ? ` The draft and run ID remain ${available}; the exact input is unavailable or incomplete.` : '';
    setStatus(`${failure.message}${detail}${pendingDraft ? ' No new model request was started automatically.' : ''}`);
  } finally {
    activeRunClient = null;
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
  if (typeof info.id === 'string' && info.id && info.revision !== undefined) {
    chatCreationKey = `attachment-review:${info.id}:revision-${info.revision}:chat-creation`;
  }
  if (info.recovery?.status === 'unavailable') showRecoveryWarning();
  drafts = globalThis.indexedDB ? new DraftStore(info.revision) : null;
  pendingDraft = drafts ? await drafts.get() : null;
  restoreChatCreation();
  await loadCreationReceipts();
  await loadChatList(pendingDraft?.chatId || pendingCreation?.requestId);
  if (!pendingDraft && drafts?.expired) setStatus('The saved retry draft expired after 24 hours and was deleted. It was not sent again automatically.');
  else if (!drafts) setStatus(mobileAgentStorageUnavailable()
    ? 'IndexedDB is unavailable. This mobile host cannot submit Agent runs without it. Enable browser storage or reopen in a browser context that allows it.'
    : 'IndexedDB draft storage is unavailable. A saved turn cannot be restored after refresh; Agent availability depends on this host.');
  if (pendingDraft) {
    $('prompt').value = pendingDraft.prompt;
    $('file-names').textContent = draftFileNames(pendingDraft).join(', ') || 'No files selected';
    setStatus(hasOriginalMessage(pendingDraft)
      ? 'A saved turn is ready. Choose Retry to check the host and reuse its run ID and original input.'
      : 'The saved input is incomplete. Check the original run read-only before deciding what to do next.');
  }
  showPendingRequestMissing();
  ready = true;
  if (creationStorageProblem) setStatus(creationStorageProblem);
  else if (pendingCreation && !pendingDraft) await checkChatCreation();
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
$('check-chats').addEventListener('click', () => void checkSavedChats());
$('check-creation').addEventListener('click', () => void checkChatCreation());
$('creation-receipts').addEventListener('change', updateControls);
$('restore-creation').addEventListener('click', () => void restoreSelectedCreation());
$('retry-creation').addEventListener('click', () => void createChat({retry: true}));
$('forget-creation').addEventListener('click', () => {
  if (busy || !confirm('Forget this saved creation request? A Chat may already exist or still be created. Review saved Chats first. Forgetting does not cancel creation or create a replacement.')) return;
  if (!chatCreationKey) { setStatus('The host App identity is unavailable; no creation record was changed.'); return; }
  try {
    localStorage.removeItem(chatCreationKey);
    pendingCreation = null;
    creationStorageProblem = null;
    setStatus('The local creation record was forgotten. Existing or in-flight host Chats are unchanged. A new Chat requires a separate explicit action.');
  } catch (failure) { setStatus(failure.message); }
  updateControls();
});
$('chat-list').addEventListener('change', () => {
  if ($('chat-list').value) void openChat($('chat-list').value).catch((failure) => setStatus(failure.message));
});
$('cancel').addEventListener('click', () => {
  if (!activeRunClient) return;
  activeRunClient.abortRun();
  setStatus('Cancellation requested; waiting for the host terminal outcome.');
});
$('clear-draft').addEventListener('click', async () => {
  if (!pendingDraft || busy || !confirm('Forget this saved turn? Any admitted run remains in Chat history and will not be cancelled. Any SDK handoff data is separate.')) return;
  if (drafts) await drafts.delete();
  pendingDraft = null;
  selectedFiles = [];
  $('file-names').textContent = 'No files selected';
  $('prompt').value = '';
  setStatus('Saved retry draft removed. Any admitted run and SDK handoff data are separate.');
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
