import { isTauri } from "@tauri-apps/api/core";

export type UpdateProgress = { received: number; total: number | null };

export interface PendingUpdate {
  version: string;
  body?: string;
  downloadAndInstall(onProgress: (progress: UpdateProgress) => void): Promise<void>;
}

export type UpdateCheck =
  | { kind: "available"; update: PendingUpdate }
  | { kind: "current" }
  | { kind: "unavailable" };

/**
 * Keeps the desktop-only updater out of the normal web build. The served UI
 * remains useful for development, but an installed desktop app is the only
 * place an update can be checked or installed.
 */
export async function checkForUpdate(): Promise<UpdateCheck> {
  if (!isTauri()) return { kind: "unavailable" };

  const { check } = await import("@tauri-apps/plugin-updater");
  const update = await check();
  if (!update) return { kind: "current" };

  return {
    kind: "available",
    update: {
      version: update.version,
      body: update.body,
      async downloadAndInstall(onProgress) {
        let received = 0;
        let total: number | null = null;
        await update.downloadAndInstall((event) => {
          if (event.event === "Started") {
            total = event.data.contentLength ?? null;
          } else if (event.event === "Progress") {
            received += event.data.chunkLength;
            onProgress({ received, total });
          }
        });
      },
    },
  };
}

/** Installs the already signature-verified update, then starts the new app. */
export async function installUpdate(update: PendingUpdate, onProgress: (progress: UpdateProgress) => void) {
  await update.downloadAndInstall(onProgress);
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
