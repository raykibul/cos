import type { NewAgent } from "../api/client";

export interface ComposerValue {
  prompt: string;
  model: string;
  permissionMode: string;
}

/** What the composer sends, without empty fields. */
export function toNewAgent(value: ComposerValue, title?: string): NewAgent {
  const request: NewAgent = { permissionMode: value.permissionMode };
  if (value.prompt.trim()) request.prompt = value.prompt.trim();
  if (value.model) request.model = value.model;
  if (title?.trim()) request.title = title.trim();
  return request;
}
