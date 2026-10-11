import { useRef } from "react";
import { IonApp, setupIonicReact } from "@ionic/react";
import { IonReactHashRouter } from "@ionic/react-router";
import { ErrorBoundary } from "@/app/error-boundary";
import { PlatformProvider, usePlatform } from "@/lib/platform-provider";

/// Ionic reads `mode` when a component is CONSTRUCTED, not when it renders, so
/// `setupIonicReact` has to run before the first `ion-*` element exists. The
/// device context is read synchronously from the browser, so the first render
/// already carries the right mode.
///
/// The hash router is deliberate: `dist/index.html` is opened straight from
/// disk, and a `file://` page has no server to answer deep links.
function IonicHost({ children }) {
  const { adapter } = usePlatform();
  const configured = useRef(false);

  if (!configured.current) {
    setupIonicReact({ mode: adapter.ionicMode });
    configured.current = true;
  }

  return (
    <IonApp>
      <IonReactHashRouter>{children}</IonReactHashRouter>
    </IonApp>
  );
}

export function AppProviders({ children }) {
  return (
    <PlatformProvider>
      <ErrorBoundary>
        <IonicHost>{children}</IonicHost>
      </ErrorBoundary>
    </PlatformProvider>
  );
}
