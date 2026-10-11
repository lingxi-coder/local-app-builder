import { useRef } from "react";
import { IonApp, setupIonicReact } from "@ionic/react";
import { ErrorBoundary } from "@/app/error-boundary";
import { PlatformProvider, usePlatform } from "@/lib/platform-provider";

/// Same contract as the DOM scaffold — see the comment there for why
/// `setupIonicReact` runs once, before the first `ion-*` element exists.
///
/// The difference is that there is NO router. A drawn app is one surface plus
/// overlays; a router outlet would add a page stack with nothing to push onto
/// it, and its transitions would fight the frame loop for the compositor.
function IonicHost({ children }) {
  const { adapter } = usePlatform();
  const configured = useRef(false);

  if (!configured.current) {
    setupIonicReact({ mode: adapter.ionicMode });
    configured.current = true;
  }

  return <IonApp>{children}</IonApp>;
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
