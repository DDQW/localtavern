import { convertFileSrc } from "@tauri-apps/api/core";
import { inTauri } from "./api";

const REMOTE = /^(https?:|data:|blob:|asset:|http:\/\/asset\.localhost)/i;

/** True for a filesystem path (`C:\pics\a.png`, `\\server\share\a.png`, `/home/me/a.png`). */
export function isLocalPath(value: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(value) || value.startsWith("\\\\") || value.startsWith("/");
}

export const VIDEO_EXTENSIONS = /\.(mp4|webm|mov|m4v|ogv)$/i;
export const AUDIO_EXTENSIONS = /\.(mp3|wav|ogg|oga|m4a|flac)$/i;

/** A URL the webview can load for a stored file path (the app's asset protocol inside Tauri). */
export function fileUrl(path: string | null | undefined): string {
  if (!path) return "";
  if (REMOTE.test(path)) return path;
  if (!inTauri) return path;
  try {
    return convertFileSrc(path);
  } catch {
    return path;
  }
}
