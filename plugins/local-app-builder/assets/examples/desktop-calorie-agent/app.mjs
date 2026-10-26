// Desktop supplies its preload SDK; Mobile loads the authenticated external-browser SDK.
if (!globalThis.localApp) {
  try { await import('/__local_app_sdk.js'); }
  catch { /* The UI below explains how to reopen this app from LingXi. */ }
}

import { nutritionSchema, recognitionPrompt, checkedResult, calorieRange, mealRecord } from './nutrition.mjs';
import { applyUnsavedMealPortion, registerMealWebMcp, summarizeMealDraft, summarizeSavedMealRecords } from './webmcp.mjs';

const $ = (id) => document.getElementById(id);
const agent = window.localApp?.agent;
$('host-open').hidden = typeof window.localApp?.host?.open !== 'function';
$('host-open').addEventListener('click', () => window.localApp?.host?.open());

function clearOldAgentCaches() {
  for (const getStorage of [() => globalThis.localStorage, () => globalThis.sessionStorage]) {
    try {
      const storage = getStorage();
      for (let index = storage.length - 1; index >= 0; index--) {
        const key = storage.key(index);
        if (key && /^(?:revision-\d+:)?meal-notes\.agent\.(pending|run)$/.test(key)) storage.removeItem(key);
      }
    } catch {}
  }
  if (globalThis.indexedDB) {
    const request = indexedDB.deleteDatabase('meal-notes-agent-drafts');
    request.onerror = () => console.warn('Could not remove the retired meal Agent draft database.');
    request.onblocked = () => console.warn('The retired meal Agent draft database is still open in another tab.');
  }
}
clearOldAgentCaches();

let hostError = '';
if (agent) {
  try {
    const info = await window.localApp.info();
    $('ios-host-note').hidden = info.hostPlatform !== 'ios';
  } catch (failure) { hostError = `无法读取应用运行信息：${failure.message}。请重新打开应用。`; }
}

const historyKey = 'meal-notes.records.v1';
let image = null;
let imageUrl = null;
let stream = null;
let result = null;
let busy = false;
let runClient = null;
let completedRunId = null;
let cameraGeneration = 0;

function error(message) {
  $('error').textContent = message;
  $('error').hidden = !message;
}

function setBusy(value) {
  busy = value;
  $('analyze').disabled = busy || !agent || !image;
  $('analyze').textContent = '识别这一餐';
  $('cancel').hidden = !busy;
  $('camera-open').disabled = busy;
  $('photo-file').disabled = busy;
  $('photo-choose').disabled = busy;
  $('notes').disabled = busy;
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

function showPhoto(file) {
  if (!file || !(file instanceof File)) throw new Error('请选择一张照片。');
  result = null;
  completedRunId = null;
  $('result').hidden = true;
  image = file;
  if (imageUrl) URL.revokeObjectURL(imageUrl);
  imageUrl = URL.createObjectURL(file);
  $('photo').src = imageUrl;
  stopCamera();
  $('status').textContent = '照片已准备好。点击识别后会启动一个新的 Agent 线程。';
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

function currentMealDraftSummary() {
  return summarizeMealDraft({
    result,
    edits: result ? edits() : [],
    canSave: Boolean(completedRunId),
    busy,
  });
}

function updateUnsavedDraftPortion(input) {
  if (busy || !result || !completedRunId) throw new Error('There is no completed, unsaved estimate to edit.');
  const rows = [...$('food-items').querySelectorAll('.food')];
  const row = rows[input.itemIndex];
  const nameField = row?.querySelector('[data-field="name"]');
  const gramsField = row?.querySelector('[data-field="grams"]');
  if (!nameField || !gramsField) throw new RangeError('itemIndex is outside the current meal draft.');

  const change = applyUnsavedMealPortion({ result, edits: edits(), canSave: true, busy }, input);
  nameField.value = change.edits[input.itemIndex].name;
  gramsField.value = change.edits[input.itemIndex].grams;
  const inputEvent = new Event('input', { bubbles: true });
  nameField.dispatchEvent(inputEvent);
  gramsField.dispatchEvent(new Event('input', { bubbles: true }));
  return { updated: true, item: change.item, calorieRange: change.summary.calorieRange, canSave: change.summary.canSave };
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

$('analyze').addEventListener('click', async () => {
  if (busy || !agent || !image) return;
  error('');
  stopCamera();
  $('result').hidden = true;
  result = null;
  completedRunId = null;
  runClient = agent.createClient({threadId: crypto.randomUUID()});
  const runId = crypto.randomUUID();
  setBusy(true);
  $('status').textContent = '正在准备 AG-UI 1.0 消息…';

  let runError = null;
  let outcome;
  try {
    const imagePart = await agent.file(image);
    runClient.addMessage({
      id: crypto.randomUUID(),
      role: 'user',
      content: [{type: 'text', text: recognitionPrompt($('notes').value)}, imagePart],
    });
    $('status').textContent = '正在识别餐食…';
    const response = await runClient.runAgent({
      runId,
      forwardedProps: {
        'com.lingxi.localApp': {
          version: 1,
          outputSchema: nutritionSchema,
          tools: [],
          maxTurns: 4,
          timeoutMs: 120000,
        },
      },
    }, {
      onEvent({event}) {
        if (event.type === 'STEP_STARTED') $('status').textContent = '正在分析餐食…';
      },
      onRunErrorEvent({event}) { runError = event; },
      onRunFinishedEvent({outcome: terminalOutcome}) { outcome = terminalOutcome; },
    });

    if (runError) throw new Error(`${runError.message}${runError.code ? ` (${runError.code})` : ''}`);
    if (outcome === 'cancelled') {
      $('status').textContent = '宿主确认识别已取消。';
      return;
    }
    if (outcome === 'interrupt') {
      $('status').textContent = '识别已暂停。请返回 LingXi 处理授权或待确认问题。';
      return;
    }
    if (outcome !== 'success') throw new Error('宿主没有报告成功的 AG-UI 终止事件。');
    renderResult(response.result);
    completedRunId = runId;
    $('status').textContent = '识别完成。请确认食物与份量，再保存记录。';
  } catch (failure) {
    error(`${failure.message}。本次结果没有保存。请检查授权或网络后由你决定是否重新识别。`);
    $('status').textContent = '识别未完成；没有自动重新提交。';
  } finally {
    runClient = null;
    setBusy(false);
  }
});

$('cancel').addEventListener('click', () => {
  if (!runClient) return;
  runClient.abortRun();
  $('status').textContent = '已请求取消；等待宿主报告最终状态。';
});

function readHistory() {
  const saved = JSON.parse(localStorage.getItem(historyKey) || '[]');
  if (!Array.isArray(saved)) throw new Error('已保存记录格式无效。请保留应用数据并联系维护者。');
  return saved;
}

function listSavedMealSummaries(limit) {
  return summarizeSavedMealRecords(readHistory(), limit);
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
    if (!completedRunId) throw new Error('没有可保存的已完成估算。');
    const record = mealRecord(result, edits(), completedRunId);
    const records = readHistory();
    localStorage.setItem(historyKey, JSON.stringify([record, ...records.filter((item) => item.id !== record.id)]));
    renderHistory();
    $('status').textContent = '已保存。照片未保存在餐食记录中。';
    error('');
  } catch (failure) { error(`未保存：${failure.message}`); }
});

window.addEventListener('pagehide', stopCamera);
try { renderHistory(); } catch (failure) { error(failure.message); }
if (!agent) {
  $('status').textContent = hostError || '请从支持 AG-UI 1.0 的灵犀应用库中打开，并在应用详情中选择模型与授权。普通浏览器无法调用宿主 Agent。';
} else {
  setBusy(false);
}

// Browser agents can discover these app-scoped tools only when the host
// exposes the official document.modelContext API. Unsupported browsers keep
// the ordinary UI; registering tools never starts or saves an Agent run.
registerMealWebMcp({
  documentRef: document,
  windowRef: window,
  getDraftSummary: currentMealDraftSummary,
  updateDraftPortion: updateUnsavedDraftPortion,
  listSavedRecords: listSavedMealSummaries,
});
