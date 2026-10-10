# Water tracker

A one-screen app to log glasses of water and see today's total.

```authoring-spec
{
  "name": "Water Tracker",
  "brief": "Log glasses of water and see today's total.",
  "template_id": "react-dom-r4",
  "spec": {
    "product": {
      "goal": "Track daily water intake",
      "tasks": ["log a glass", "show today's total"],
      "external_integrations": []
    },
    "targets": [{ "id": "phone", "os": "ios", "form_factor": "phone" }],
    "ui": {
      "structure": ["header", "log button", "today's total"],
      "theme": { "mode": "system", "accent": "teal" },
      "style": { "direction": "calm", "density": "comfortable" },
      "references": []
    },
    "design": {
      "presentations": [
        { "target_id": "phone", "presentation": "single screen", "navigation": "none" }
      ],
      "tokens": { "radius": "16px" },
      "states": {
        "loading": "skeleton total",
        "empty": "prompt to log the first glass",
        "error": "retry banner",
        "success": "total ticks up",
        "permission": "notifications not yet granted"
      },
      "inputs": {
        "pointer_touch": ["tap to log"],
        "keyboard_mouse": ["space to log"],
        "back": "system back leaves the app",
        "reduced_motion": "no counter animation"
      }
    },
    "acceptance_checks": [
      {
        "id": "log-a-glass",
        "target_ids": ["phone"],
        "required": true,
        "preconditions": [],
        "steps": ["tap the log button"],
        "expected": "today's total increases by one",
        "evidence": ["inspect"]
      }
    ]
  }
}
```
