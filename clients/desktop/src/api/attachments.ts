// Files attached to a message ("Attachments" in docs/agents-api.md): what an
// upload returns, and the files a transcript message names. Decoded
// leniently, like the rest.

import { asObject, list, num, str } from "./decode";

/** Where an upload goes: the folder an agent works in. */
export type AttachTarget =
  | { kind: "staff"; projectId: number }
  | { kind: "agent"; agentId: number }
  | { kind: "workspace"; workspaceId: number };

/** A file the daemon saved; `name` is what a message refers to. */
export interface Attachment {
  name: string;
  path: string;
  size: number;
  contentType: string;
}

/** An uploaded file on a message being sent, with a local preview for images. */
export type SentAttachment = Attachment & { previewUrl?: string | null };

/** A file a message names, as the transcript reads it back. */
export interface AttachedFile {
  name: string;
  path: string;
  contentType: string;
  /** As the message gives it: `245 KB`. */
  sizeLabel: string;
}

/** The largest file the daemon takes. */
export const MAX_ATTACHMENT_BYTES = 32 * 1024 * 1024;
/** How many files one message may carry. */
export const MAX_ATTACHMENTS = 20;

export function uploadPath(target: AttachTarget, name: string): string {
  const base =
    target.kind === "staff"
      ? `/v1/projects/${target.projectId}/staff/attachments`
      : target.kind === "agent"
        ? `/v1/agents/${target.agentId}/attachments`
        : `/v1/workspaces/${target.workspaceId}/attachments`;
  return `${base}?name=${encodeURIComponent(name)}`;
}

export function decodeAttachment(value: unknown): Attachment {
  const o = asObject(value, "attachment");
  const name = str(o.name, "file");
  return {
    name,
    path: str(o.path),
    size: num(o.size, 0),
    contentType: str(o.content_type, "application/octet-stream"),
  };
}

function decodeAttached(value: unknown): AttachedFile {
  const o = asObject(value, "attached file");
  const path = str(o.path);
  return {
    name: str(o.name) || path.split("/").pop() || "file",
    path,
    contentType: str(o.content_type, "application/octet-stream"),
    sizeLabel: str(o.size_label),
  };
}

export function decodeAttachedFiles(value: unknown): AttachedFile[] {
  return Array.isArray(value) ? list(value, decodeAttached) : [];
}

export function isImage(contentType: string): boolean {
  return /^image\/(png|jpeg|gif|webp|svg\+xml)$/.test(contentType);
}

/** `812 B`, `245 KB`, `3.3 MB`, as the daemon writes it. */
export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}
