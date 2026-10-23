import assert from 'node:assert/strict';
import test from 'node:test';
import { readFile } from 'node:fs/promises';
import { MealAgentClient } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/agent-client.mjs';
import { checkedResult, calorieRange, mealRecord, recognitionPrompt } from '../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/nutrition.mjs';

const food = { name: 'Rice', grams: 150, kcalPer100gMin: 120, kcalPer100gMax: 150, basis: 'model_estimate', sources: [] };
const result = () => ({ items: [{ ...food }], assumptions: ['Cooking oil is unknown'], questions: ['How much rice?'] });
function memoryStorage() {
  const values = new Map();
  return { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value), removeItem: (key) => values.delete(key) };
}
const state = (sequence, status = 'running') => ({ runId: 'run-1', state: status, sequence, updatedAt: 5, result: { output: result() } });

test('edits scale estimates without silently replacing model assumptions or provenance', () => {
  const checked = checkedResult(result());
  assert.deepEqual(calorieRange(checked.items), { min: 180, max: 225 });
  const record = mealRecord(checked, [{ name: 'Cooked rice', grams: '200' }], 'run-1', new Date('2026-10-23T12:00:00Z'));
  assert.deepEqual(record.range, { min: 240, max: 300 });
  assert.equal(record.items[0].estimatedGrams, 150);
  assert.equal(record.items[0].estimatedName, 'Rice');
  assert.equal(record.items[0].basis, 'model_estimate');
  assert.equal(record.reviewedByUser, true);
  assert.equal(checked.items[0].grams, 150);
  assert.equal('image' in record, false);
});

test('inverted ranges, invalid edited portions, and unearned database provenance are rejected', () => {
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, kcalPer100gMin: 200 }] }));
  for (const grams of ['', 0, -1, Infinity, 'unknown', 10001]) {
    assert.throws(() => mealRecord(result(), [{ name: 'Rice', grams }], 'run-1'));
  }
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, basis: 'nutrition_source', sources: ['invented database'] }] }));
  assert.throws(() => checkedResult({ ...result(), items: [{ ...food, sources: ['invented database'] }] }));
  assert.match(recognitionPrompt(), /no tools/);
  assert.match(recognitionPrompt(), /do not invent/);
});

test('empty food result is explainable but cannot be saved as a meal', () => {
  const empty = checkedResult({ items: [], assumptions: [], questions: ['Please photograph food.'] });
  assert.throws(() => mealRecord(empty, [], 'run-1'));
});

test('a lost run response retries the same request after page reload', async () => {
  const storage = memoryStorage();
  const inputs = [];
  const request = { requestId: 'request-1', prompt: 'meal', images: [{ media_type: 'image/jpeg', base64: 'encoded' }] };
  const agent = {
    async run(input) { inputs.push(input); if (inputs.length === 1) throw new Error('IPC disconnected'); return { runId: 'run-1' }; },
    subscribe() { return () => {}; }, async get() { return state(1); },
  };
  const first = new MealAgentClient(agent, storage, () => {});
  await assert.rejects(first.submit(request));
  const reloaded = new MealAgentClient(agent, storage, () => {});
  assert.equal(await reloaded.resume(), null);
  assert.equal(inputs.length, 1, 'refresh never silently retries an unconfirmed request');
  await reloaded.submit({ requestId: 'must-not-replace', prompt: 'new photo must not silently replace the original' });
  assert.deepEqual(inputs, [request, request]);
  assert.equal(reloaded.pending, null, 'temporary image payload is removed after host acceptance');
  const third = new MealAgentClient(agent, storage, () => {});
  await third.resume();
  assert.equal(inputs.length, 2, 'accepted runs reconnect through get/subscribe, never run again');
});

test('a rejected request can be explicitly discarded before a different photo is submitted', async () => {
  const storage = memoryStorage();
  const inputs = [];
  const firstPhoto = { requestId: 'photo-1', prompt: 'meal', images: [{ media_type: 'image/jpeg', base64: 'first' }] };
  const nextPhoto = { requestId: 'photo-2', prompt: 'meal', images: [{ media_type: 'image/jpeg', base64: 'second' }] };
  let granted = false;
  const agent = {
    async run(input) { inputs.push(input); if (!granted) throw new Error('Agent access has not been granted for this application.'); return { runId: 'run-1' }; },
    subscribe() { return () => {}; }, async get() { return state(1); },
  };
  const client = new MealAgentClient(agent, storage, () => {});
  await assert.rejects(client.submit(firstPhoto));
  granted = true;
  await client.resume();
  assert.equal(inputs.length, 1, 'granting permission then refreshing must not run the rejected photo');
  await assert.rejects(client.cancel(), /尚未确认受理/);
  assert.equal(inputs.length, 1, 'cancel must not create an unaccepted run');
  assert.throws(() => client.discardPending(), /确认放弃/);
  assert.equal(client.discardPending(true), true);
  assert.equal(client.pending, null);
  assert.equal(inputs.length, 1, 'explicit discard itself makes no model call');
  await client.submit(nextPhoto);
  assert.deepEqual(inputs, [firstPhoto, nextPhoto]);
});

test('replay/get race and duplicate events cannot overwrite a terminal state', async () => {
  const received = [];
  let listener;
  let unsubscribed = 0;
  const agent = {
    subscribe(_id, callback) { listener = callback; callback({ type: 'state', runId: 'run-1', sequence: 4, run: state(4, 'succeeded') }); return () => { unsubscribed++; }; },
    async get() { return state(2); },
    async cancel(runId) { assert.equal(runId, 'run-1'); },
  };
  const client = new MealAgentClient(agent, memoryStorage(), (event) => received.push(event));
  await client.watch('run-1');
  listener({ type: 'state', runId: 'other-app-run', sequence: 5, run: state(5) });
  listener({ type: 'state', runId: 'run-1', sequence: 4, run: state(4, 'succeeded') });
  assert.deepEqual(received.map((event) => event.run.state), ['succeeded']);
  await client.cancel();
  client.dispose();
  assert.equal(unsubscribed, 1);
});

test('storage failure prevents starting a task that cannot recover its request identity', async () => {
  let ran = false;
  const storage = { getItem() { return null; }, setItem() { throw new Error('QuotaExceededError'); } };
  const client = new MealAgentClient({ async run() { ran = true; } }, storage, () => {});
  await assert.rejects(client.submit({ requestId: 'request-1', prompt: 'meal' }), /QuotaExceededError/);
  assert.equal(ran, false);
});

test('desktop example declares only camera and agent and keeps provider access in the host', async () => {
  const base = new URL('../../plugins/local-app-builder/assets/examples/desktop-calorie-agent/', import.meta.url);
  const manifest = JSON.parse(await readFile(new URL('lingxi-app.json', base), 'utf8'));
  assert.deepEqual(manifest.capabilities, ['agent', 'camera']);
  assert.deepEqual(manifest.agent.tools, []);
  const source = await readFile(new URL('app.mjs', base), 'utf8');
  assert.match(source, /window\.localApp\?\.agent/);
  assert.match(source, /getUserMedia/);
  assert.match(source, /createImageBitmap/);
  assert.doesNotMatch(source, /fetch\(|api[_-]?key|window\.lingxi|innerHTML/i);
});
