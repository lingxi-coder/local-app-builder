// Desktop supplies its preload SDK; Mobile loads the authenticated browser SDK.
if (!globalThis.localApp && location.hostname === '127.0.0.1') {
  try { await import('/__local_app_sdk.js'); }
  catch { /* The UI below explains how to reopen this app from LingXi. */ }
}

import { MealAgentClient, MealDraftStore, terminalStates, revisionStorage } from './agent-client.mjs';
import { nutritionSchema, recognitionPrompt, checkedResult, calorieRange, mealRecord } from './nutrition.mjs';

const $ = (id) => document.getElementById(id);
$('host-open').hidden = !window.localApp?.host?.open;
$('host-open').addEventListener('click', () => window.localApp?.host?.open());
let agent = window.localApp?.agent;
let taskStorage;
let draftStore;
let hostError = '';
if (agent) {
  try {
    const info = await window.localApp.info();
    $('ios-host-note').hidden = info.hostPlatform !== 'ios';
    // The external browser can open a fresh page after the native handoff.
    taskStorage = revisionStorage(info.platform === 'mobile-browser' ? localStorage : sessionStorage, info.revision);
    draftStore = new MealDraftStore(info.revision);
  }
  catch (failure) { agent = null; hostError = `无法读取应用运行版本：${failure.message}。请重新打开应用。`; }
}
const historyKey = 'meal-notes.records.v1';
let image = null;
let imageUrl = null;
let stream = null;
let result = null;
let busy = false;
let completedRunId = null;
let resultRunId = null;
let cameraGeneration = 0;

function error(message) {
  $('error').textContent = message;
  $('error').hidden = !message;
}

function setBusy(value) {
  busy = value;
  $('analyze').disabled = busy || !agent || (!image && !client?.pending);
  $('analyze').textContent = client?.pending ? '恢复同一请求' : '识别这一餐';
  $('cancel').hidden = !busy || Boolean(client?.pending) || !client?.runId;
  $('pending-review').hidden = busy || !client?.pending;
  $('camera-open').disabled = busy || Boolean(client?.pending);
  $('photo-file').disabled = busy || Boolean(client?.pending);
  $('photo-choose').disabled = busy || Boolean(client?.pending);
  $('notes').disabled = busy || Boolean(client?.pending);
}

function stopCamera() {
  cameraGeneration += 1;
  stream?.getTracks().forEach((track) => track.stop());
  stream = null;
  $('camera').srcObject = null;
  $('camera').hidden = true;
  $('camera-capture').hidden = true;
  $('camera-close').hidden = true;
  $('photo').hidden = !image;
  $('photo-placeholder').hidden = Boolean(image);
}

async function photoFile(source) {
  let drawable = source;
  let bitmap = null;
  try {
    if (source instanceof File) {
      if (source.size > 50 * 1024 * 1024) throw new Error('请选择 50 MiB 以内的照片。');
      bitmap = await createImageBitmap(source);
      drawable = bitmap;
    }
    const width = drawable.videoWidth || drawable.width;
    const height = drawable.videoHeight || drawable.height;
    if (!width || !height) throw new Error('照片无法读取，请换一张照片。');
    if (width * height > 64000000 || Math.max(width, height) > 32768) throw new Error('照片尺寸过大。');
    const scale = Math.min(1, 1536 / Math.max(width, height));
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(width * scale));
    canvas.height = Math.max(1, Math.round(height * scale));
    const context = canvas.getContext('2d');
    if (!context) throw new Error('浏览器无法处理这张照片。');
    context.fillStyle = '#fff';
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(drawable, 0, 0, canvas.width, canvas.height);
    const blob = await new Promise((resolve, reject) => canvas.toBlob(value => value ? resolve(value) : reject(new Error('照片压缩失败。')), 'image/jpeg', .86));
    return new File([blob], 'meal-photo.jpg', {type: 'image/jpeg', lastModified: Date.now()});
  } finally {
    bitmap?.close();
  }
}

function showPhoto(file, replacing = true) {
  if (!file || !(file instanceof File)) throw new Error('请选择一张照片。');
  if (replacing) {
    client?.forgetCompleted();
    result = null;
    completedRunId = null;
    resultRunId = null;
    $('result').hidden = true;
  }
  image = file;
  if (imageUrl) URL.revokeObjectURL(imageUrl);
  imageUrl = URL.createObjectURL(file);
  $('photo').src = imageUrl;
  stopCamera();
  $('status').textContent = '照片已准备好，确认后点击识别。';
  error('');
  setBusy(false);
}

async function setPhoto(source) { showPhoto(await photoFile(source)); }
$('camera-open').addEventListener('click', async () => {
  try {
    if (!navigator.mediaDevices?.getUserMedia) throw new Error('此浏览器不支持相机预览。');
    stopCamera();
    const generation = cameraGeneration;
    const requestedStream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: 'environment' }, audio: false });
    if (generation !== cameraGeneration || busy) { requestedStream.getTracks().forEach((track) => track.stop()); return; }
    stream = requestedStream;
    $('camera').srcObject = stream;
    await $('camera').play();
    $('camera').hidden = false;
    $('photo').hidden = true;
    $('photo-placeholder').hidden = true;
    $('camera-capture').hidden = false;
    $('camera-close').hidden = false;
    error('');
  } catch (failure) {
    stopCamera();
    error(`无法打开相机：${failure.message}。也可以选择照片。`);
  }
});
$('camera-close').addEventListener('click', stopCamera);
$('photo-choose').addEventListener('click', async () => {
  try {
    if (typeof window.localApp?.files?.pick === 'function') {
      const picked = await window.localApp.files.pick({multiple: false, accept: 'image/jpeg,image/png,image/webp'});
      if (picked[0]) await setPhoto(picked[0]);
    } else $('photo-file').click();
  } catch (failure) { error(failure.message); }
});
$('camera-capture').addEventListener('click', async () => {
  try { await setPhoto($('camera')); } catch (failure) { error(failure.message); }
});
$('photo-file').addEventListener('change', async (event) => {
  const file = event.target.files?.[0];
  if (!file) return;
  try {
    if (!['image/jpeg', 'image/png', 'image/webp'].includes(file.type)) {
      throw new Error('请选择 JPEG、PNG 或 WebP 照片。');
    }
    await setPhoto(file);
  } catch (failure) { error(failure.message); }
  event.target.value = '';
});

function edits() {
  return [...$('food-items').querySelectorAll('.food')].map((row) => ({
    name: row.querySelector('[data-field="name"]').value,
    grams: row.querySelector('[data-field="grams"]').value,
  }));
}

function updateTotal() {
  try {
    const values = edits();
    const range = calorieRange(result.items.map((item, index) => ({ ...item, grams: values[index].grams })));
    $('total').textContent = `${range.min}–${range.max}`;
    $('save').disabled = result.items.length === 0;
  } catch {
    $('total').textContent = '请检查份量';
    $('save').disabled = true;
  }
}

function listText(id, heading, entries) {
  $(id).replaceChildren();
  if (!entries.length) return;
  const title = document.createElement('h3');
  title.textContent = heading;
  const list = document.createElement('ul');
  for (const entry of entries) {
    const item = document.createElement('li');
    item.textContent = entry;
    list.append(item);
  }
  $(id).append(title, list);
}

function renderResult(value) {
  result = checkedResult(value);
  $('food-items').replaceChildren();
  result.items.forEach((item, index) => {
    const row = document.createElement('div');
    row.className = 'food';
    for (const [field, labelText] of [['name', '食物'], ['grams', '份量（克）']]) {
      const label = document.createElement('label');
      label.textContent = labelText;
      const input = document.createElement('input');
      input.dataset.field = field;
      input.id = `food-${index}-${field}`;
      input.value = item[field];
      if (field === 'grams') { input.type = 'number'; input.min = '1'; input.max = '10000'; input.step = 'any'; }
      else input.maxLength = 120;
      input.addEventListener('input', updateTotal);
      label.append(input);
      row.append(label);
    }
    const density = document.createElement('p');
    density.className = 'fine';
    density.textContent = `模型估算：${item.kcalPer100gMin}–${item.kcalPer100gMax} kcal / 100 克`;
    row.append(density);
    $('food-items').append(row);
  });
  listText('assumptions', '估算基于这些假设', result.assumptions);
  listText('questions', '这些信息可以让估算更准确', result.questions);
  $('result').hidden = false;
  updateTotal();
}

const phases = { queued: '请求已提交…', running: '正在识别餐食…', waiting_for_permission: '请在 LingXi 宿主窗口中处理工具授权。', waiting_for_answer: '请在 LingXi 宿主窗口中回答待确认问题。' };
const client = agent ? new MealAgentClient(agent, taskStorage, (event) => {
  if (event.type === 'draft-expired') {
    setBusy(false);
    $('status').textContent = '照片重试草稿已超过 24 小时并已删除；没有请求被自动重新提交。请重新选择照片。';
    return;
  }
  if (event.type === 'connection') {
    error(event.error.message);
    $('status').textContent = '连接已暂停；请返回灵犀恢复应用。不会自动重新提交识别请求。';
    return;
  }
  const task = event.task;
  setBusy(!terminalStates.has(task.state));
  if (task.state === 'succeeded') {
    completedRunId = task.runId;
    if (resultRunId !== task.runId) {
      try { renderResult(task.result?.output); resultRunId = task.runId; }
      catch (failure) { error(failure.message); $('status').textContent = '结果未通过应用校验，请重试。'; return; }
    }
    $('status').textContent = '识别完成。请确认食物与份量，再保存记录。';
  } else if (terminalStates.has(task.state)) {
    $('status').textContent = task.state === 'cancelled' ? '识别已取消。' : task.state === 'interrupted' ? '任务已中断，请手动重试。' : '识别失败。';
    error(task.error ? `${task.error.message} (${task.error.code})` : '');
  } else $('status').textContent = phases[task.phase] || phases[task.state] || '正在处理…';
}, draftStore) : null;

$('analyze').addEventListener('click', async () => {
  if (busy || !client || (!image && !client.pending)) return;
  error('');
  stopCamera();
  $('result').hidden = true;
  result = null;
  completedRunId = null;
  resultRunId = null;
  setBusy(true);
  try {
    const request = client.pending ?? { requestId: crypto.randomUUID(), prompt: recognitionPrompt($('notes').value),
      attachments: [image], outputSchema: nutritionSchema, tools: [], maxTurns: 4, timeoutMs: 120000 };
    await client.submit(request);
  } catch (failure) {
    error(failure.code === 'draft_expired'
      ? `${failure.message}不会自动重试或恢复这条旧请求。`
      : failure.definitive
      ? `${failure.message}。宿主未接受请求，已清除保存的请求和照片。调整后再重新提交。`
      : `${failure.message}。可在确认原请求状态后重试同一请求；请先在 LingXi 的 Apps 中检查本应用的 Agent 授权和模型。`);
    setBusy(false);
  }
});
$('cancel').addEventListener('click', async () => {
  try { await client.cancel(); } catch (failure) { error(`取消未确认：${failure.message}`); }
});

function readHistory() {
  const saved = JSON.parse(localStorage.getItem(historyKey) || '[]');
  if (!Array.isArray(saved)) throw new Error('已保存记录格式无效。请保留应用数据并联系维护者。');
  return saved;
}

function renderHistory() {
  const records = readHistory();
  $('history').replaceChildren();
  if (!records.length) { $('history').textContent = '还没有记录。确认第一餐后，它会出现在这里。'; return; }
  for (const record of records) {
    const article = document.createElement('article');
    article.className = 'record';
    const heading = document.createElement('strong');
    heading.textContent = `${record.range.min}–${record.range.max} kcal · 模型估算`;
    const detail = document.createElement('p');
    detail.textContent = record.items.map((item) => `${item.name} ${item.grams} 克`).join('、');
    const date = document.createElement('p');
    date.className = 'fine';
    date.textContent = new Date(record.savedAt).toLocaleString();
    article.append(heading, detail, date);
    $('history').append(article);
  }
}
$('save').addEventListener('click', () => {
  try {
    const record = mealRecord(result, edits(), completedRunId);
    const records = readHistory();
    localStorage.setItem(historyKey, JSON.stringify([record, ...records.filter((item) => item.id !== record.id)]));
    renderHistory();
    $('status').textContent = '已保存。照片未保存在餐食记录中。';
    error('');
  } catch (failure) { error(`未保存：${failure.message}`); }
});
window.addEventListener('pagehide', () => { stopCamera(); client?.dispose(); });
const checkDraftExpiry = () => {
  if (document.visibilityState === 'visible') void client?.checkDraftExpiry().catch((failure) => error(`无法检查照片草稿过期状态：${failure.message}`));
};
document.addEventListener('visibilitychange', checkDraftExpiry);
window.addEventListener('pageshow', checkDraftExpiry);
setInterval(checkDraftExpiry, 60 * 60 * 1000);
try { renderHistory(); } catch (failure) { error(failure.message); }
if (!client) {
  $('status').textContent = hostError || '请从支持 Agent 的灵犀应用库中打开，并在应用详情中选择模型与授权。普通浏览器无法调用宿主 Agent。';
} else {
  setBusy(true);
  client.resume().then((runId) => {
    if (client.drafts.expired) $('status').textContent = '保存的照片重试草稿已超过 24 小时并已删除；没有请求被自动重新提交。';
    if (!runId) {
      setBusy(false);
      if (client.pending) {
        const draftPhoto = client.pending.attachments?.[0];
        if (draftPhoto instanceof File) showPhoto(draftPhoto, false);
        $('status').textContent = '保留了一条未确认请求。点击“恢复同一请求”后才会再次提交。';
      }
    }
  }).catch((failure) => { error(`恢复任务失败：${failure.message}`); setBusy(false); });
}
