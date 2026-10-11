import { getDeviceContext, normalizeDeviceContext } from "./device-context";

/// Template-managed. Derives the platform facts that something actually consumes.
///
/// The previous version of this file published `stateLayer: "ripple"`,
/// `navigation: "tabs"` and `supportsSidebar` — none of which had a single
/// consumer anywhere in the template. Ripples and platform chrome are now real
/// because `ionicMode` feeds `setupIonicReact({ mode })`, not because a table
/// says the word.

const DEFAULT_ADAPTER_KEY = "desktop:desktop";

/// `ios` renders Apple's design language, `md` renders Material. Deciding it
/// here, from the same device context as everything else, keeps the Ionic mode
/// and the rest of the platform styling in agreement; it also gets iPadOS right,
/// whose desktop-class user agent would otherwise read as a Mac.
function ionicModeFor(os) {
  return os === "ios" ? "ios" : "md";
}

const PLATFORM_ADAPTERS = {
  "ios:iphone": { controlDensity: 44, fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif" },
  "ios:tablet": { controlDensity: 44, fontFamily: "-apple-system, BlinkMacSystemFont, sans-serif" },
  "android:phone": { controlDensity: 48, fontFamily: "Roboto, sans-serif" },
  "android:tablet": { controlDensity: 48, fontFamily: "Roboto, sans-serif" },
  "desktop:desktop": { controlDensity: 40, fontFamily: "Inter, system-ui, sans-serif" },
};

export function getPlatformAdapter(context = getDeviceContext()) {
  const safeContext = normalizeDeviceContext(context);
  const key = `${safeContext.os}:${safeContext.formFactor}`;
  const resolved = key === "ios:ipad" ? "ios:tablet" : key;
  return {
    ...(PLATFORM_ADAPTERS[resolved] ?? PLATFORM_ADAPTERS[DEFAULT_ADAPTER_KEY]),
    key,
    ionicMode: ionicModeFor(safeContext.os),
    context: safeContext,
  };
}

/// The custom properties `styles/foundation.css` reads. Applied to
/// `document.documentElement` by the provider — NOT to a screen component, which
/// the app rewrites freely. Safe-area insets are not among them: CSS reads those
/// straight from `env(safe-area-inset-*)`.
export function platformStyle(adapter) {
  const fallback = PLATFORM_ADAPTERS[DEFAULT_ADAPTER_KEY];
  const safe = adapter && typeof adapter === "object" ? adapter : fallback;
  return {
    "--platform-control-min": `${safe.controlDensity ?? fallback.controlDensity}px`,
    "--platform-font": safe.fontFamily ?? fallback.fontFamily,
  };
}
