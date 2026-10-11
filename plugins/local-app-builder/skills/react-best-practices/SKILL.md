---
name: react-best-practices
description: Apply React 19 client-side rules for local apps, focusing on component boundaries, state and effect discipline, cleanup, error handling, and render performance.
---

# React best practices

This skill is only for the client-side React layer inside the pinned local-app
runtime.

Use it to keep the app:

- split into clear component, hook, and adapter boundaries;
- explicit about loading, empty, error, success, and permission states;
- disciplined about effects, cleanup, and subscription lifecycles;
- performant without pushing per-frame simulation state into React or Zustand.

Read [references/router.md](references/router.md) first. Exclude Next.js, SSR,
Server Components, route loaders owned by other frameworks, and platform design
rules that belong in `$frontend-design`.
