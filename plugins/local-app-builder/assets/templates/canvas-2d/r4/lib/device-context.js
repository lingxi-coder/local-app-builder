/// Template-managed. The device facts the app adapts to, read from the browser.
///
/// There is no native host: every value comes from a standard web API, so the
/// same build behaves the same wherever a browser opens it. Values are read on
/// every call; the provider calls again whenever one of them can have changed.

export const FALLBACK_DEVICE_CONTEXT = {
  os: "unknown",
  formFactor: "unknown",
  viewport: { width: 0, height: 0 },
  safeArea: { top: 0, right: 0, bottom: 0, left: 0 },
  colorScheme: "light",
  reducedMotion: false,
  inputMode: "touch",
};

function nonNegativeNumber(value) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0
    ? value
    : 0;
}

export function normalizeDeviceContext(value) {
  if (!value || typeof value !== "object") return FALLBACK_DEVICE_CONTEXT;
  return {
    os: typeof value.os === "string" ? value.os : "unknown",
    formFactor:
      typeof value.formFactor === "string" ? value.formFactor : "unknown",
    viewport: {
      width: nonNegativeNumber(value.viewport?.width),
      height: nonNegativeNumber(value.viewport?.height),
    },
    safeArea: {
      top: nonNegativeNumber(value.safeArea?.top),
      right: nonNegativeNumber(value.safeArea?.right),
      bottom: nonNegativeNumber(value.safeArea?.bottom),
      left: nonNegativeNumber(value.safeArea?.left),
    },
    colorScheme: ["dark", "light", "unknown"].includes(value.colorScheme)
      ? value.colorScheme
      : "unknown",
    reducedMotion: value.reducedMotion === true,
    inputMode: ["pointer", "touch", "hybrid", "unknown"].includes(
      value.inputMode,
    )
      ? value.inputMode
      : "unknown",
  };
}

function matches(query) {
  try {
    return window.matchMedia?.(query)?.matches === true;
  } catch {
    return false;
  }
}

/// iPadOS sends a desktop "Macintosh" user agent, so a Mac that reports touch
/// points is an iPad.
function detectOs(userAgent, maxTouchPoints) {
  if (/iPhone|iPad|iPod/.test(userAgent)) return "ios";
  if (/Macintosh/.test(userAgent) && maxTouchPoints > 1) return "ios";
  if (/Android/.test(userAgent)) return "android";
  return "desktop";
}

/// 600 CSS pixels on the short side is where Android's own layouts switch from
/// phone to tablet.
function detectFormFactor(os, userAgent, screenWidth, screenHeight) {
  if (os === "ios") return /iPhone|iPod/.test(userAgent) ? "iphone" : "ipad";
  if (os === "android") {
    return Math.min(screenWidth, screenHeight) >= 600 ? "tablet" : "phone";
  }
  return "desktop";
}

function detectInputMode() {
  const fine = matches("(pointer: fine)");
  const coarse = matches("(any-pointer: coarse)");
  if (fine && coarse) return "hybrid";
  return fine ? "pointer" : "touch";
}

let safeAreaProbe = null;

/// `env(safe-area-inset-*)` can only be read back through layout, so an
/// invisible fixed element carries the four insets as its padding. CSS should
/// keep using `env()` directly (see `styles/foundation.css`); these numbers are
/// for script that positions things on a canvas.
function readSafeArea() {
  if (
    typeof document === "undefined" ||
    !document.body ||
    typeof getComputedStyle !== "function"
  ) {
    return FALLBACK_DEVICE_CONTEXT.safeArea;
  }
  if (!safeAreaProbe || !safeAreaProbe.isConnected) {
    safeAreaProbe = document.createElement("div");
    safeAreaProbe.setAttribute("aria-hidden", "true");
    safeAreaProbe.style.cssText =
      "position:fixed;top:0;left:0;width:0;height:0;visibility:hidden;pointer-events:none;" +
      "padding:env(safe-area-inset-top,0px) env(safe-area-inset-right,0px) " +
      "env(safe-area-inset-bottom,0px) env(safe-area-inset-left,0px)";
    document.body.appendChild(safeAreaProbe);
  }
  const style = getComputedStyle(safeAreaProbe);
  return {
    top: parseFloat(style.paddingTop) || 0,
    right: parseFloat(style.paddingRight) || 0,
    bottom: parseFloat(style.paddingBottom) || 0,
    left: parseFloat(style.paddingLeft) || 0,
  };
}

export function getDeviceContext() {
  if (typeof window === "undefined") return FALLBACK_DEVICE_CONTEXT;
  const nav = typeof navigator === "undefined" ? {} : navigator;
  const userAgent = String(nav.userAgent ?? "");
  const os = detectOs(userAgent, nonNegativeNumber(nav.maxTouchPoints));
  const viewport = window.visualViewport ?? null;
  return normalizeDeviceContext({
    os,
    formFactor: detectFormFactor(
      os,
      userAgent,
      nonNegativeNumber(window.screen?.width),
      nonNegativeNumber(window.screen?.height),
    ),
    viewport: {
      width: viewport?.width ?? window.innerWidth,
      height: viewport?.height ?? window.innerHeight,
    },
    safeArea: readSafeArea(),
    colorScheme: matches("(prefers-color-scheme: dark)") ? "dark" : "light",
    reducedMotion: matches("(prefers-reduced-motion: reduce)"),
    inputMode: detectInputMode(),
  });
}
