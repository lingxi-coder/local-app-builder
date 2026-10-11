# App shell and routing

- Keep routed screens inside `IonRouterOutlet` and wrap routed views in
  `IonPage`.
- Keep the checked-in hash router and do not swap in a parallel navigation
  stack: `dist/index.html` opens from disk, where a history-based router has no
  server to answer deep links.
- Import `useIonRouter` and every other Ionic hook from `@ionic/react`, never
  from `@ionic/react-router`.
- Use one intentional navigation model per target presentation instead of
  mixing arbitrary tabs, stacks, and sheets.
- Let overlays, sheets, and modals stay Ionic surfaces so focus and dismissal
  behavior remain platform-correct.
- Keep safe-area handling on the `--ion-safe-area-*` variables that
  `styles/foundation.css` feeds from `env(safe-area-inset-*)`.
- When a canvas or Three scene is present, keep the scene under one surface and
  keep HUD, pause, and settings in separate overlay UI.
- Import Ionic hooks and components from `@ionic/react`, not from
  `@ionic/react-router`.

## Sources

Reviewed: 2026-08-27

- Ionic React navigation: https://ionicframework.com/docs/react/navigation
- IonPage navigation guidance: https://ionicframework.com/docs/react/navigation#ionpage
