import type { NewAgent } from "../api/client";

export interface ComposerValue {
  prompt: string;
  model: string;
  permissionMode: string;
  /** Files uploaded to the workspace, by name. */
  attachments?: string[];
}

/** What the composer sends, without empty fields. */
export function toNewAgent(value: ComposerValue, title?: string): NewAgent {
  const request: NewAgent = { permissionMode: value.permissionMode };
  if (value.prompt.trim()) request.prompt = value.prompt.trim();
  if (value.model) request.model = value.model;
  if (title?.trim()) request.title = title.trim();
  if (value.attachments?.length) request.attachments = value.attachments;
  return request;
}
