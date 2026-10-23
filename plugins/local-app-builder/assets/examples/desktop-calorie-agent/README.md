# 餐盘笔记 — Desktop Agent example

This dependency-free static Local App demonstrates the real `window.localApp.agent` API. Copy this directory into a project and publish the complete directory with `LocalApp({ "bundle_path": "/absolute/copied/project" })`, then start it. In the trusted Apps UI, enable this App's Agent and select a configured model that supports images and structured output. Allow camera access to test capture, or use Choose photo. No provider secret or extra backend is required.

The photo is resized to at most 1536 pixels on its longest edge and sent as JPEG after the user presses Recognize. The app requests a schema with food candidates, estimated grams, calorie density ranges, assumptions and questions. It grants no business tools, so every result is explicitly a **model estimate**, not a nutrition database measurement. Users can correct names and gram weights before pressing Save. Changing food identity requires a new estimate; changing its displayed name does not invent a new calorie density. Saved app-local records retain original estimates and user changes, but not photo bytes.

`agent-client.mjs` saves the exact pending request in sessionStorage before submission and removes it after the host returns a run ID. If an IPC response is lost, an explicit Retry uses that same request ID/payload; refresh does not resubmit it. A rejected or unconfirmed request can be explicitly discarded using the acknowledgment and discard button, then the user can choose a new photo. Discarding its recovery record does not claim to cancel a possibly accepted host task. Cancel itself never submits an unconfirmed request. After acceptance, refresh only reconnects to the existing task. Photos are therefore temporarily retained in the current app tab until acceptance or explicit discard, never added to saved meal history. An unavailable storage quota prevents starting an unrecoverable request. Terminal failure, interruption and cancellation are displayed; they are never automatically rerun. The UI uses only text nodes for model output.

## Files

- `index.html`, `styles.css`, `app.mjs`: camera/file input, busy/cancel states, result editing and app-local records.
- `agent-client.mjs`: idempotent submission and sequence-aware reconnect.
- `nutrition.mjs`: Draft 7 schema and business validation. Nutrition semantics do not belong in the generic Agent host.
- `lingxi-app.json`: Desktop static entry with only `camera` and `agent`, and no tool grants.

The example needs the Desktop runtime. A plain browser can show its UI but cannot call an Agent. Mobile continues using its existing document contract until its host implements and advertises an Agent bridge.

## Checks

From the plugin repository:

```sh
node --test scripts/tests/test-desktop-calorie-agent.mjs
./scripts/check-all.sh
```

Unit tests cover calorie range semantics, provenance rejection, edited/saved portions, lost-response retry, refresh recovery, event reordering, cancellation and storage failure. These do not prove live model behavior. For end-to-end acceptance, publish/start the registered App, choose a photo, confirm it before sending, verify the real validated output, edit a portion, save and reload. Also test host model absence, denied camera/Agent, a nonfood image, cancellation and reload during a running task. Retain the signed host package and provider evidence with the implementation acceptance report.
