// The fake daemon's attachments ("Attachments" in docs/agents-api.md):
// uploads into an agent's folder, a workspace or a chief's worktree, kept in
// memory under `<folder>/.cos/attachments/<name>`, and the files themselves
// for the chat to show. Names are made safe and unique the way the daemon
// makes them.

import type { DaemonResponse } from "../types";
import type { Raw } from "./fixtures";

export const MAX_BYTES = 32 * 1024 * 1024;

export interface FakeFile {
  name: string;
  path: string;
  size: number;
  content_type: string;
  /** The bytes, base64. */
  data: string;
}

const types: Record<string, string> = {
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  heic: "image/heic",
  svg: "image/svg+xml",
  pdf: "application/pdf",
  txt: "text/plain",
  log: "text/plain",
  md: "text/markdown",
  csv: "text/csv",
  json: "application/json",
  zip: "application/zip",
};

export function contentTypeOf(name: string): string {
  const dot = name.lastIndexOf(".");
  const extension = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  return types[extension] ?? "application/octet-stream";
}

/** As the daemon cleans an upload's name. */
export function safeName(name: string, contentType: string): string {
  const base = name.split(/[/\\]/).pop() ?? "";
  let clean = base
    .trim()
    .replace(/[^\p{L}\p{N}._-]/gu, "-")
    .replace(/-+/g, "-")
    .replace(/^[.-]+|[.-]+$/g, "");
  if (!clean) clean = "file";
  if (!/\.[^.]{1,9}$/.test(clean)) {
    const extension = Object.entries(types).find(
      ([, kind]) => kind === contentType.split(";")[0]?.trim(),
    )?.[0];
    if (extension) clean += `.${extension}`;
  }
  return clean.slice(0, 100);
}

export function sizeLabel(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function base64Length(data: string): number {
  const clean = data.replace(/\s/g, "");
  const padding = clean.endsWith("==") ? 2 : clean.endsWith("=") ? 1 : 0;
  return Math.floor((clean.length * 3) / 4) - padding;
}

export class AttachmentError {
  constructor(
    readonly status: number,
    readonly message: string,
  ) {}
}

/** Every file uploaded to the fake daemon, by folder. */
export class FakeFiles {
  private readonly folders = new Map<string, FakeFile[]>();

  /** Saves an upload in `folder`, renaming it if the name is taken. */
  save(folder: string, name: string, contentType: string, data: string): FakeFile {
    const size = base64Length(data);
    if (size > MAX_BYTES) throw new AttachmentError(413, "a file can be at most 32 MB");
    const files = this.folders.get(folder) ?? [];
    const clean = safeName(name, contentType);
    const dot = clean.lastIndexOf(".");
    const [stem, extension] = dot > 0 ? [clean.slice(0, dot), clean.slice(dot)] : [clean, ""];
    let candidate = clean;
    for (let n = 2; files.some((f) => f.name === candidate); n++)
      candidate = `${stem}-${n}${extension}`;
    const file: FakeFile = {
      name: candidate,
      path: `${folder}/.cos/attachments/${candidate}`,
      size,
      content_type: contentTypeOf(candidate),
      data,
    };
    files.push(file);
    this.folders.set(folder, files);
    return file;
  }

  find(folder: string, name: string): FakeFile {
    const file = this.folders.get(folder)?.find((f) => f.name === name || f.path === name);
    if (!file) throw new AttachmentError(404, `there is no attachment ${name}`);
    return file;
  }

  /** A message's text with the block naming its files, as the daemon types it. */
  static block(files: FakeFile[]): Raw[] {
    return files.map((f) => ({
      name: f.name,
      path: f.path,
      content_type: f.content_type,
      size_label: sizeLabel(f.size),
    }));
  }
}

/** The upload's answer, as the daemon gives it. */
export function created(file: FakeFile): DaemonResponse {
  const { data: _data, ...json } = file;
  return { status: 201, body: json };
}

/** A screenshot you attached in cos's chief's first message (an SVG, so the fake needs no image files). */
export const loginShot = {
  folder: "/Users/me/.cos/worktrees/cos-1/chief",
  name: "login-expired.svg",
  svg: `<svg xmlns="http://www.w3.org/2000/svg" width="480" height="300" viewBox="0 0 480 300">
<rect width="480" height="300" rx="14" fill="#f4f1ea"/>
<rect x="0" y="0" width="480" height="34" rx="14" fill="#e6e1d6"/>
<circle cx="20" cy="17" r="5" fill="#e0685e"/><circle cx="38" cy="17" r="5" fill="#e8b84d"/><circle cx="56" cy="17" r="5" fill="#6cbf5a"/>
<rect x="140" y="70" width="200" height="170" rx="12" fill="#fff" stroke="#d9d3c6"/>
<text x="240" y="105" font-family="Helvetica" font-size="16" font-weight="700" text-anchor="middle" fill="#2b2a27">Sign in</text>
<rect x="164" y="122" width="152" height="24" rx="6" fill="#f4f1ea"/>
<rect x="164" y="154" width="152" height="24" rx="6" fill="#f4f1ea"/>
<rect x="164" y="190" width="152" height="28" rx="7" fill="#d97757"/>
<rect x="110" y="250" width="260" height="30" rx="8" fill="#fbe3df"/>
<text x="240" y="270" font-family="Helvetica" font-size="12" text-anchor="middle" fill="#b23b2e">Session expired. Please sign in again.</text>
</svg>`,
};

export function seedFiles(files: FakeFiles): FakeFile[] {
  const svg = loginShot.svg;
  const data = btoa(unescape(encodeURIComponent(svg)));
  return [
    files.save(loginShot.folder, loginShot.name, "image/svg+xml", data),
    files.save(
      loginShot.folder,
      "auth.log",
      "text/plain",
      btoa("ERROR session expired after 300s\n".repeat(40)),
    ),
  ];
}
