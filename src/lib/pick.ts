// Native file dialogs. Outside Tauri (the browser mock) they return canned values so the pages that
// use them can still be exercised.

import { open, save } from "@tauri-apps/plugin-dialog";
import { inTauri } from "./api";

const IMAGES = { name: "Pictures", extensions: ["png", "jpg", "jpeg", "webp", "gif", "bmp"] };

function asList(selection: string | string[] | null): string[] {
  return selection === null ? [] : Array.isArray(selection) ? selection : [selection];
}

export async function pickImages(multiple = true): Promise<string[]> {
  if (!inTauri) return ["C:\mock\picture.png"];
  return asList(await open({ multiple, filters: [IMAGES] }));
}

export async function pickFolder(): Promise<string | null> {
  if (!inTauri) return "C:\mock\sprites";
  const selection = await open({ directory: true });
  return typeof selection === "string" ? selection : null;
}

export async function pickCard(): Promise<string | null> {
  if (!inTauri) return "C:\mock\card.json";
  const selection = await open({
    multiple: false,
    filters: [{ name: "SillyTavern character card", extensions: ["json", "png"] }],
  });
  return typeof selection === "string" ? selection : null;
}

export async function pickSaveJson(defaultName: string): Promise<string | null> {
  if (!inTauri) return `C:\mock\${defaultName}.json`;
  return save({ defaultPath: `${defaultName}.json`, filters: [{ name: "Character card (JSON)", extensions: ["json"] }] });
}
