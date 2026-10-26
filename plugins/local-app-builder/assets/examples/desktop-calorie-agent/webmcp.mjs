import { calorieRange, checkedResult } from './nutrition.mjs';

const emptyInputSchema = {
  type: 'object',
  properties: {},
  additionalProperties: false,
};

const mealListInputSchema = {
  type: 'object',
  properties: {
    limit: { type: 'integer', minimum: 1, maximum: 20 },
  },
  additionalProperties: false,
};

const portionInputSchema = {
  type: 'object',
  required: ['itemIndex', 'grams'],
  properties: {
    itemIndex: { type: 'integer', minimum: 0, maximum: 19 },
    grams: { type: 'number', minimum: 1, maximum: 10000 },
    name: { type: 'string', minLength: 1, maxLength: 120 },
  },
  additionalProperties: false,
};

function own(object, key) {
  return Object.prototype.hasOwnProperty.call(object, key);
}

function inputObject(value, allowedKeys, requiredKeys = []) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) {
    throw new TypeError('Tool input must be a JSON object.');
  }
  const prototype = Object.getPrototypeOf(value);
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError('Tool input must be a plain JSON object.');
  }
  for (const key of Object.keys(value)) {
    if (!allowedKeys.includes(key)) throw new TypeError(`Unknown tool input: ${key}.`);
  }
  for (const key of requiredKeys) {
    if (!own(value, key)) throw new TypeError(`Missing tool input: ${key}.`);
  }
  return value;
}

function jsonResponse(value) {
  let encoded;
  try { encoded = JSON.stringify(value); }
  catch { throw new TypeError('Tool result must be JSON serializable.'); }
  if (typeof encoded !== 'string') throw new TypeError('Tool result must be JSON serializable.');
  return JSON.parse(encoded);
}

function execute(handler) {
  return (input, { signal } = {}) => {
    signal?.throwIfAborted();
    const value = handler(input);
    signal?.throwIfAborted();
    return jsonResponse(value);
  };
}

/** Summarize only the visible estimate and edits; never include the source photo or bytes. */
export function summarizeMealDraft({ result, edits = [], canSave = false, busy = false } = {}) {
  if (!result) {
    return {
      available: false,
      status: busy ? 'recognizing' : 'no_estimate',
      canSave: false,
      hasUnsavedEdits: false,
      items: [],
      calorieRange: null,
      assumptions: [],
      questions: [],
    };
  }

  const checked = checkedResult(result);
  if (!Array.isArray(edits) || edits.length !== checked.items.length) {
    throw new Error('The current meal draft is incomplete.');
  }

  const items = checked.items.map((estimate, index) => {
    const edit = edits[index];
    const name = typeof edit?.name === 'string' ? edit.name.slice(0, 120) : '';
    const gramsValue = Number(edit?.grams);
    const gramsValid = Number.isFinite(gramsValue) && gramsValue >= 1 && gramsValue <= 10000;
    return {
      index,
      name,
      grams: gramsValid ? gramsValue : null,
      estimatedName: estimate.name,
      estimatedGrams: estimate.grams,
      kcalPer100gMin: estimate.kcalPer100gMin,
      kcalPer100gMax: estimate.kcalPer100gMax,
      basis: estimate.basis,
    };
  });
  const valid = items.length > 0 && items.every((item) => (
    item.name.trim().length > 0 && item.name.trim().length <= 120 && item.grams !== null
  ));
  const range = valid
    ? calorieRange(items.map((item) => ({
      grams: item.grams,
      kcalPer100gMin: item.kcalPer100gMin,
      kcalPer100gMax: item.kcalPer100gMax,
    })))
    : null;

  return {
    available: true,
    status: 'reviewing',
    valid,
    canSave: Boolean(canSave) && !busy && valid,
    hasUnsavedEdits: items.some((item) => (
      item.name.trim() !== item.estimatedName || item.grams !== item.estimatedGrams
    )),
    items,
    calorieRange: range,
    assumptions: checked.assumptions.slice(0, 12).map((text) => text.slice(0, 500)),
    questions: checked.questions.slice(0, 8).map((text) => text.slice(0, 500)),
  };
}

function validatePortionInput(value) {
  const input = inputObject(value, ['itemIndex', 'grams', 'name'], ['itemIndex', 'grams']);
  if (!Number.isSafeInteger(input.itemIndex) || input.itemIndex < 0 || input.itemIndex > 19) {
    throw new TypeError('itemIndex must be an integer from 0 to 19.');
  }
  if (typeof input.grams !== 'number' || !Number.isFinite(input.grams) || input.grams < 1 || input.grams > 10000) {
    throw new TypeError('grams must be a number from 1 to 10,000.');
  }
  if (own(input, 'name') && (
    typeof input.name !== 'string' || input.name.trim().length < 1 || input.name.trim().length > 120
  )) {
    throw new TypeError('name must contain 1 to 120 non-space characters.');
  }
  return {
    itemIndex: input.itemIndex,
    grams: input.grams,
    ...(own(input, 'name') ? { name: input.name.trim() } : {}),
  };
}

/** Return proposed DOM edits without saving or mutating the original estimate. */
export function applyUnsavedMealPortion(draft, value) {
  const input = validatePortionInput(value);
  if (draft?.busy) throw new Error('Meal portions cannot be edited during recognition.');
  if (!draft?.result || !draft.canSave) throw new Error('There is no completed, unsaved estimate to edit.');
  const checked = checkedResult(draft.result);
  if (!Array.isArray(draft.edits) || draft.edits.length !== checked.items.length) {
    throw new Error('The current meal draft is incomplete.');
  }
  if (input.itemIndex >= checked.items.length) throw new RangeError('itemIndex is outside the current meal draft.');

  const edits = draft.edits.map((edit) => ({ name: edit.name, grams: edit.grams }));
  const current = edits[input.itemIndex];
  edits[input.itemIndex] = {
    name: input.name ?? current.name,
    grams: String(input.grams),
  };
  const summary = summarizeMealDraft({ ...draft, edits });
  if (!summary.valid) throw new Error('Update the draft with a valid food name and portion.');
  return { edits, summary, item: summary.items[input.itemIndex] };
}

function summarizeSavedRecord(record) {
  if (!record || typeof record !== 'object' || Array.isArray(record)) {
    throw new Error('A saved meal record is malformed.');
  }
  if (typeof record.id !== 'string' || record.id.length < 1 || record.id.length > 128) {
    throw new Error('A saved meal record has an invalid identifier.');
  }
  if (typeof record.savedAt !== 'string' || !Number.isFinite(Date.parse(record.savedAt))) {
    throw new Error('A saved meal record has an invalid date.');
  }
  if (record.reviewedByUser !== true || !Array.isArray(record.items) || record.items.length < 1 || record.items.length > 20) {
    throw new Error('A saved meal record is not a reviewed meal.');
  }

  const checked = checkedResult({
    items: record.items.map((item) => ({
      name: item?.name,
      grams: item?.grams,
      kcalPer100gMin: item?.kcalPer100gMin,
      kcalPer100gMax: item?.kcalPer100gMax,
      basis: item?.basis,
      sources: item?.sources,
    })),
    assumptions: [],
    questions: [],
  });
  const computedRange = calorieRange(checked.items);
  if (!record.range || record.range.min !== computedRange.min || record.range.max !== computedRange.max) {
    throw new Error('A saved meal record has an inconsistent calorie range.');
  }

  return {
    id: record.id,
    savedAt: new Date(record.savedAt).toISOString(),
    calorieRange: computedRange,
    items: checked.items.map((item) => ({
      name: item.name,
      grams: item.grams,
      kcalPer100gMin: item.kcalPer100gMin,
      kcalPer100gMax: item.kcalPer100gMax,
      basis: item.basis,
    })),
  };
}

/** Bound reads to recent records and return a narrow, JSON-only projection. */
export function summarizeSavedMealRecords(records, limit = 10) {
  if (!Array.isArray(records)) throw new TypeError('Saved meal history must be a list.');
  if (!Number.isSafeInteger(limit) || limit < 1 || limit > 20) {
    throw new TypeError('limit must be an integer from 1 to 20.');
  }
  const summaries = records.slice(0, limit).map(summarizeSavedRecord);
  return { count: summaries.length, limit, records: summaries };
}

/** Create the app's business tools using callbacks that stay inside this App's data boundary. */
export function createMealWebMcpTools({ getDraftSummary, updateDraftPortion, listSavedRecords }) {
  if (typeof getDraftSummary !== 'function' || typeof updateDraftPortion !== 'function' || typeof listSavedRecords !== 'function') {
    throw new TypeError('Meal WebMCP requires draft, edit, and history callbacks.');
  }

  return [
    {
      name: 'meal_get_draft_summary',
      title: 'Read current meal estimate',
      description: 'Read the current recognition draft and user edits from this app. Returns food names, portion sizes, estimated calorie ranges, assumptions, and questions. Does not read or return the photo.',
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      inputSchema: emptyInputSchema,
      execute: execute((value) => {
        inputObject(value, []);
        return getDraftSummary();
      }),
    },
    {
      name: 'meal_list_saved_records',
      title: 'List recent saved meals',
      description: 'Read up to 20 recent, user-saved meal summaries from this app local storage. Returns no photos, arbitrary storage keys, or host data.',
      annotations: { readOnlyHint: true, untrustedContentHint: true },
      inputSchema: mealListInputSchema,
      execute: execute((value) => {
        const input = inputObject(value, ['limit']);
        const limit = own(input, 'limit') ? input.limit : 10;
        if (!Number.isSafeInteger(limit) || limit < 1 || limit > 20) {
          throw new TypeError('limit must be an integer from 1 to 20.');
        }
        return listSavedRecords(limit);
      }),
    },
    {
      name: 'meal_update_unsaved_portion',
      title: 'Edit an unsaved meal portion',
      description: 'Edit the name and/or gram portion of one item in the current completed, unsaved recognition draft. This does not start recognition, alter saved records, or save the meal; the user must still click Save.',
      annotations: { consequentialHint: true, untrustedContentHint: true },
      inputSchema: portionInputSchema,
      execute: execute((value) => updateDraftPortion(validatePortionInput(value))),
    },
  ];
}

function isModelContext(value) {
  try {
    return Boolean(value) && typeof value.registerTool === 'function';
  } catch {
    return false;
  }
}

/**
 * Register only with the browser's official document.modelContext API.
 * Registrations use one AbortSignal and survive BFCache pagehide/pageShow pairs.
 */
export function registerMealWebMcp({
  documentRef = globalThis.document,
  windowRef = globalThis.window,
  getDraftSummary,
  updateDraftPortion,
  listSavedRecords,
  onStatus,
} = {}) {
  const report = (status) => {
    try { onStatus?.(status); }
    catch { /* Availability reporting must not interrupt the ordinary App. */ }
  };
  let modelContext;
  try { modelContext = documentRef?.modelContext; }
  catch { modelContext = null; }
  if (!isModelContext(modelContext)) {
    report({state: 'unavailable', reason: 'api_unavailable'});
    return { supported: false, ready: Promise.resolve(false), dispose() {} };
  }
  if (!windowRef
    || typeof windowRef.addEventListener !== 'function'
    || typeof windowRef.removeEventListener !== 'function'
    || typeof AbortController !== 'function') {
    report({state: 'unavailable', reason: 'lifecycle_unavailable'});
    return { supported: false, ready: Promise.resolve(false), dispose() {} };
  }

  const tools = createMealWebMcpTools({ getDraftSummary, updateDraftPortion, listSavedRecords });
  let controller = null;
  let ready = Promise.resolve(false);
  let disposed = false;

  const register = () => {
    if (disposed || controller) return ready;
    const nextController = new AbortController();
    controller = nextController;
    report({state: 'pending'});
    ready = (async () => {
      for (const tool of tools) {
        if (nextController.signal.aborted) return false;
        await modelContext.registerTool(tool, { signal: nextController.signal });
      }
      const registered = !nextController.signal.aborted;
      if (registered && !disposed) report({state: 'registered'});
      return registered;
    })().catch(() => {
      if (controller === nextController) {
        const failed = !nextController.signal.aborted;
        nextController.abort();
        controller = null;
        if (failed && !disposed) report({state: 'unavailable', reason: 'registration_failed'});
      }
      return false;
    });
    return ready;
  };

  const onPageHide = (event) => {
    // A BFCache entry is still the same document; keep its API registration.
    if (event?.persisted) return;
    controller?.abort();
    controller = null;
    ready = Promise.resolve(false);
  };
  const onPageShow = (event) => {
    if (event?.persisted && !controller) void register();
  };

  windowRef.addEventListener('pagehide', onPageHide);
  windowRef.addEventListener('pageshow', onPageShow);
  void register();

  return {
    supported: true,
    get ready() { return ready; },
    dispose() {
      if (disposed) return;
      disposed = true;
      windowRef.removeEventListener('pagehide', onPageHide);
      windowRef.removeEventListener('pageshow', onPageShow);
      controller?.abort();
      controller = null;
    },
  };
}
