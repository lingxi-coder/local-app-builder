export const terminalStates = new Set(['succeeded', 'failed', 'cancelled', 'interrupted']);
const pendingKey = 'meal-notes.agent.pending';
const runKey = 'meal-notes.agent.run';
const draftTtlMs = 24 * 60 * 60 * 1000;

export function revisionStorage(storage, revision) {
  if (!Number.isSafeInteger(revision) || revision < 1) throw new Error('应用运行版本不可用，请重新打开应用。');
  const key = (name) => `revision-${revision}:${name}`;
  return {
    getItem: (name) => storage.getItem(key(name)),
    setItem: (name, value) => storage.setItem(key(name), value),
    removeItem: (name) => storage.removeItem(key(name)),
  };
}

export class MealDraftStore {
  constructor(revision) {
    if (!Number.isSafeInteger(revision) || revision < 1) throw new Error('应用运行版本不可用，请重新打开应用。');
    this.key = `meal-notes:revision-${revision}`;
    this.expired = false;
  }

  open() {
    if (!globalThis.indexedDB) return Promise.reject(new Error('此浏览器不支持可恢复的照片存储。'));
    return new Promise((resolve, reject) => {
      const opening = indexedDB.open('meal-notes-agent-drafts', 1);
      opening.onupgradeneeded = () => {
        if (!opening.result.objectStoreNames.contains('drafts')) opening.result.createObjectStore('drafts', { keyPath: 'key' });
      };
      opening.onsuccess = () => resolve(opening.result);
      opening.onerror = () => reject(opening.error || new Error('无法打开照片恢复存储。'));
      opening.onblocked = () => reject(new Error('照片恢复存储正被另一个标签页使用。'));
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
      request.onerror = () => reject(request.error || new Error('无法读取已保存的照片。'));
      tx.oncomplete = () => db.close();
      tx.onerror = () => { db.close(); reject(tx.error || new Error('无法读取已保存的照片。')); };
    });
  }

  async put(request) {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      tx.objectStore('drafts').put({key: this.key, request, createdAt: Date.now()});
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error('无法保存照片以便恢复。')); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error('无法保存照片以便恢复。')); };
    });
  }

  async delete() {
    const db = await this.open();
    return new Promise((resolve, reject) => {
      const tx = db.transaction('drafts', 'readwrite');
      tx.objectStore('drafts').delete(this.key);
      tx.oncomplete = () => { db.close(); resolve(); };
      tx.onerror = () => { db.close(); reject(tx.error || new Error('无法删除照片恢复记录。')); };
      tx.onabort = () => { db.close(); reject(tx.error || new Error('无法删除照片恢复记录。')); };
    });
  }
}

/** Durable task observation plus a revision-scoped, explicit file draft. */
export class MealAgentClient {
  constructor(agent, storage, onEvent, drafts = new MealDraftStore(1)) {
    this.agent = agent;
    this.storage = storage;
    this.drafts = drafts;
    this.onEvent = onEvent;
    this.runId = null;
    this.requestId = null;
    this.state = null;
    this.lastSequence = -1;
    this.unsubscribe = null;
    this.submitting = false;
    this.pending = null;
  }

  async submit(input) {
    if (this.submitting) throw new Error('请求正在提交，请稍候。');
    this.submitting = true;
    try {
      const saved = await this.drafts.get();
      if (this.drafts.expired) {
        this.pending = null;
        this.requestId = null;
        this.storage.removeItem(pendingKey);
        const error = new Error('照片重试草稿已超过 24 小时并已删除；请重新选择照片后再提交。');
        error.code = 'draft_expired';
        throw error;
      }
      const request = saved ?? {...input, requestId: input.requestId || crypto.randomUUID()};
      if (saved && input.requestId !== saved.requestId) {
        throw new Error('仍有一条照片请求待恢复，请先恢复原请求。');
      }
      this.pending = request;
      this.requestId = request.requestId;
      this.storage.setItem(pendingKey, request.requestId);
      await this.drafts.put(request);
      // SDK stores a second short-lived IndexedDB copy for the iOS native
      // wake-and-return handoff. It submits only after the matching ready URL.
      let runId;
      try { ({runId} = await this.agent.tasks.start(request)); }
      catch (error) {
        if (error.definitive) {
          await this.drafts.delete();
          this.storage.removeItem(pendingKey);
          this.pending = null;
        }
        throw error;
      }
      this.storage.setItem(runKey, runId);
      this.storage.removeItem(pendingKey);
      await this.drafts.delete();
      this.pending = null;
      await this.watch(runId);
      return runId;
    } finally {
      this.submitting = false;
    }
  }

  async resume() {
    const savedRunId = this.storage.getItem(runKey);
    if (savedRunId) {
      const draft = await this.drafts.get();
      if (draft) {
        const admitted = await this.agent.tasks.get(savedRunId);
        if (admitted.requestId === draft.requestId) await this.drafts.delete();
      }
      await this.watch(savedRunId);
      return savedRunId;
    }
    const draft = await this.drafts.get();
    if (!draft) return null;
    this.pending = draft;
    this.requestId = draft.requestId;
    this.storage.setItem(pendingKey, draft.requestId);
    // The SDK may have admitted this request after returning from iOS. Find
    // it by its request ID; refresh itself never submits the draft again.
    for (let attempt = 0; attempt < 6; attempt++) {
      const tasks = await this.agent.tasks.list({limit: 100});
      const task = tasks.find((candidate) => candidate.requestId === draft.requestId);
      if (task) {
        this.storage.setItem(runKey, task.runId);
        this.storage.removeItem(pendingKey);
        await this.drafts.delete();
        this.pending = null;
        await this.watch(task.runId);
        return task.runId;
      }
      await new Promise((resolve) => setTimeout(resolve, 250));
    }
    return null;
  }

  async checkDraftExpiry() {
    if (this.submitting || !this.pending) return false;
    await this.drafts.get();
    if (!this.drafts.expired) return false;
    this.pending = null;
    this.requestId = null;
    this.storage.removeItem(pendingKey);
    this.onEvent({type: 'draft-expired'});
    return true;
  }

  async watch(runId) {
    this.dispose();
    this.runId = runId;
    this.state = null;
    this.lastSequence = -1;
    const accept = (event) => {
      if (event.runId !== this.runId) return;
      if (event.type === 'connection') { this.onEvent(event); return; }
      if (event.sequence <= this.lastSequence) return;
      this.lastSequence = event.sequence;
      if (event.type === 'state') this.state = event.task.state;
      this.onEvent(event);
    };
    this.unsubscribe = this.agent.tasks.subscribe(runId, accept);
    const task = await this.agent.tasks.get(runId);
    accept({type: 'state', runId, sequence: task.sequence, time: task.updatedAt, task});
  }

  async cancel() {
    if (this.runId) await this.agent.tasks.cancel(this.runId);
  }

  forgetCompleted() {
    if (this.submitting || this.pending || (this.runId && !terminalStates.has(this.state))) {
      throw new Error('请先恢复或等待当前请求结束。');
    }
    this.storage.removeItem(runKey);
    this.storage.removeItem(pendingKey);
    this.dispose();
    this.runId = null;
    this.requestId = null;
    this.state = null;
    this.lastSequence = -1;
  }

  dispose() {
    this.unsubscribe?.();
    this.unsubscribe = null;
  }
}
