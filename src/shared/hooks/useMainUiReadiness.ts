import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isTauriRuntime } from "../lib/tauriRuntime";

type MainUiLifecycleMode = "default" | "hidden" | "destroyed";
type MainUiWakeIntent = "main" | "search" | "tray" | "test";
type MainUiReadyPhase = "react_mounted" | "hydrated" | "search_ready" | "search_results_settled";

export type MainUiBootstrap = {
  enabled: boolean;
  mode: MainUiLifecycleMode;
  generation: number;
  request_id: number | null;
  intent: MainUiWakeIntent | null;
};

type MainUiReadyReport = {
  requestId: number;
  generation: number;
  phase: MainUiReadyPhase;
  detail?: string;
};

export const selectNewestMainUiBootstrap = (
  current: MainUiBootstrap | null,
  incoming: MainUiBootstrap
): MainUiBootstrap => {
  if (current?.request_id != null) {
    if (incoming.request_id == null || incoming.request_id < current.request_id) {
      return current;
    }
  }
  return incoming;
};

export const shouldCarryStableMainUiPhases = (
  current: MainUiBootstrap | null,
  incoming: MainUiBootstrap
) => current?.generation === incoming.generation;

const isMainWindow = () => {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("window") === null;
};

export const useMainUiReadiness = () => {
  const [bootstrap, setBootstrap] = useState<MainUiBootstrap | null>(null);
  const observedPhasesRef = useRef<Map<MainUiReadyPhase, string | undefined>>(new Map());
  const reportedPhasesRef = useRef<Set<MainUiReadyPhase>>(new Set());
  const correlationRef = useRef<string | null>(null);
  const readyToReport = Boolean(
    bootstrap?.enabled &&
    bootstrap.mode !== "default" &&
    bootstrap.request_id != null
  );

  const acceptBootstrap = useCallback((incoming: MainUiBootstrap) => {
    setBootstrap((current) => {
      const selected = selectNewestMainUiBootstrap(current, incoming);
      if (
        selected !== current &&
        (selected.generation !== current?.generation || selected.request_id !== current?.request_id)
      ) {
        const correlation = `${selected.generation}:${selected.request_id ?? "none"}`;
        if (correlationRef.current !== correlation) {
          const previousPhases = new Map(observedPhasesRef.current);
          correlationRef.current = correlation;
          observedPhasesRef.current.clear();
          reportedPhasesRef.current.clear();
          if (shouldCarryStableMainUiPhases(current, selected)) {
            for (const phase of ["react_mounted", "hydrated"] as const) {
              if (previousPhases.has(phase)) {
                observedPhasesRef.current.set(phase, previousPhases.get(phase));
              }
            }
          }
        }
      }
      return selected;
    });
  }, []);

  useEffect(() => {
    if (!isMainWindow() || !isTauriRuntime()) return;

    let cancelled = false;
    invoke<MainUiBootstrap>("get_main_ui_lifecycle_bootstrap")
      .then((nextBootstrap) => {
        if (cancelled) return;
        acceptBootstrap(nextBootstrap);
      })
      .catch((error) => {
        console.error("Failed to get main UI lifecycle bootstrap:", error);
      });

    return () => {
      cancelled = true;
    };
  }, [acceptBootstrap]);

  useEffect(() => {
    if (!isMainWindow() || !isTauriRuntime()) return;

    const unlisten = listen<MainUiBootstrap>("main-ui-lifecycle-wake", (event) => {
      acceptBootstrap(event.payload);
    });
    return () => {
      unlisten.then((off) => off());
    };
  }, [acceptBootstrap]);

  const sendReadyPhase = useCallback(
    (phase: MainUiReadyPhase, detail?: string) => {
      if (!readyToReport || bootstrap?.request_id == null) return;
      if (reportedPhasesRef.current.has(phase)) return;

      reportedPhasesRef.current.add(phase);
      const report: MainUiReadyReport = {
        requestId: bootstrap.request_id,
        generation: bootstrap.generation,
        phase,
        detail
      };

      invoke("report_main_ui_ready", { report }).catch((error) => {
        reportedPhasesRef.current.delete(phase);
        console.error(`Failed to report main UI ${phase}:`, error);
      });
    },
    [bootstrap, readyToReport]
  );

  useEffect(() => {
    if (!readyToReport) return;
    observedPhasesRef.current.forEach((detail, phase) => sendReadyPhase(phase, detail));
  }, [readyToReport, sendReadyPhase]);

  const reportReadyPhase = useCallback(
    (phase: MainUiReadyPhase, detail?: string) => {
      observedPhasesRef.current.set(phase, detail);
      sendReadyPhase(phase, detail);
    },
    [sendReadyPhase]
  );

  return {
    bootstrap,
    readinessEnabled: readyToReport,
    readinessRequestId: bootstrap?.request_id ?? null,
    shouldPrepareSearchIntent:
      readyToReport && (bootstrap?.intent === "search" || bootstrap?.intent === "test"),
    reportReadyPhase
  };
};
