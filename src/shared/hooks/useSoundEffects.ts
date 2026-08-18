import { useEffect } from "react";
import { listen } from "@tauri-apps/api/event";
import { playSoundEffect } from "../lib/soundEffects";

interface UseSoundEffectsOptions {
  soundEnabled: boolean;
  soundVolume: number;
  pasteSoundEnabled: boolean;
}

export const useSoundEffects = ({
  soundEnabled,
  soundVolume,
  pasteSoundEnabled
}: UseSoundEffectsOptions) => {
  useEffect(() => {
    const AudioContext =
      window.AudioContext ||
      (window as Window & { webkitAudioContext?: typeof window.AudioContext }).webkitAudioContext;
    if (!AudioContext) return;
    const ctx = new AudioContext();

    const unlisten = listen<string>("play-sound", (event) => {
      if (!soundEnabled) return;

      const type = event.payload;
      if (type === "paste" && !pasteSoundEnabled) return;

      const play = () => {
        try {
          if (type === "copy" || type === "paste") {
            playSoundEffect(ctx, type, soundVolume);
          }
        } catch (e) {
          console.error("Sound play error", e);
        }
      };

      if (ctx.state === "suspended") {
        ctx.resume().then(play).catch((err) => {
          console.error("Failed to resume audio ctx", err);
          play();
        });
      } else {
        play();
      }
    });

    return () => {
      unlisten.then((f) => f());
      ctx.close();
    };
  }, [soundEnabled, soundVolume, pasteSoundEnabled]);
};
