import { createContext, useContext, useEffect, useMemo, useState } from "react";
import { getDeviceContext } from "@/lib/device-context";
import { getPlatformAdapter, platformStyle } from "@/lib/platform-adapter";

const PlatformContext = createContext(null);

function readSnapshot() {
  const device = getDeviceContext();
  return { device, adapter: getPlatformAdapter(device) };
}

/// `getDeviceContext` returns a plain object, a snapshot of the moment it was
/// called. Without this comparison-and-resubscribe the provider would freeze the
/// very first read: an iPad rotation, a Split View resize, or a dark-mode toggle
/// would never reach the app.
function sameDevice(a, b) {
  return (
    a.os === b.os &&
    a.formFactor === b.formFactor &&
    a.colorScheme === b.colorScheme &&
    a.reducedMotion === b.reducedMotion &&
    a.inputMode === b.inputMode &&
    a.viewport.width === b.viewport.width &&
    a.viewport.height === b.viewport.height &&
    a.safeArea.top === b.safeArea.top &&
    a.safeArea.right === b.safeArea.right &&
    a.safeArea.bottom === b.safeArea.bottom &&
    a.safeArea.left === b.safeArea.left
  );
}

const MEDIA_QUERIES = [
  "(prefers-color-scheme: dark)",
  "(prefers-reduced-motion: reduce)",
  "(pointer: fine)",
  "(any-pointer: coarse)",
];

export function PlatformProvider({ children }) {
  const [snapshot, setSnapshot] = useState(readSnapshot);

  // Every source that can change a device fact is watched: window resize covers
  // rotation and iPad multitasking, visualViewport covers the software keyboard
  // and pinch-zoom, pageshow and visibilitychange cover a page restored from the
  // back-forward cache or a tab coming back, and the media queries cover
  // appearance, motion and input mode.
  useEffect(() => {
    const resync = () => {
      setSnapshot((current) => {
        const next = readSnapshot();
        return sameDevice(current.device, next.device) ? current : next;
      });
    };

    window.addEventListener("resize", resync);
    window.addEventListener("orientationchange", resync);
    window.addEventListener("pageshow", resync);
    document.addEventListener("visibilitychange", resync);
    const viewport = window.visualViewport ?? null;
    viewport?.addEventListener("resize", resync);
    viewport?.addEventListener("scroll", resync);

    const lists = MEDIA_QUERIES.map((query) => window.matchMedia?.(query)).filter(
      Boolean,
    );
    for (const list of lists) list.addEventListener("change", resync);

    // The first paint can land before layout has settled the safe-area insets,
    // so take one catch-up read instead of trusting the mount-time snapshot.
    resync();

    return () => {
      window.removeEventListener("resize", resync);
      window.removeEventListener("orientationchange", resync);
      window.removeEventListener("pageshow", resync);
      document.removeEventListener("visibilitychange", resync);
      viewport?.removeEventListener("resize", resync);
      viewport?.removeEventListener("scroll", resync);
      for (const list of lists) list.removeEventListener("change", resync);
    };
  }, []);

  const style = useMemo(() => platformStyle(snapshot.adapter), [snapshot.adapter]);

  // Publish the platform facts on <html>, where no generated code can drop
  // them.
  //
  // These used to live on the root element of `app/screens/home-screen.jsx` —
  // the one file every app rewrites first. An app that replaced that screen
  // lost `data-platform`, `data-input-mode` and the platform custom properties
  // along with it, and nothing failed: the styles simply stopped applying.
  useEffect(() => {
    const root = document.documentElement;
    const { adapter } = snapshot;

    root.dataset.platform = adapter.key;
    root.dataset.inputMode = adapter.context.inputMode;
    root.dataset.reducedMotion = String(adapter.context.reducedMotion === true);
    root.classList.toggle("ion-palette-dark", adapter.context.colorScheme === "dark");

    for (const [property, propertyValue] of Object.entries(style)) {
      root.style.setProperty(property, propertyValue);
    }
  }, [snapshot, style]);

  const value = useMemo(
    () => ({ ...snapshot, platformStyle: style }),
    [snapshot, style],
  );

  return (
    <PlatformContext.Provider value={value}>
      {children}
    </PlatformContext.Provider>
  );
}

export function usePlatform() {
  const value = useContext(PlatformContext);
  if (!value) throw new Error("usePlatform must be used inside PlatformProvider");
  return value;
}
