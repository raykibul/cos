// The files on a composer ("Attachments" in docs/agents-api.md): picked,
// pasted or dropped, and uploaded at once so Send is quick. The chips and
// buttons that show them are in attachments.tsx.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  type AttachTarget,
  type Attachment,
  MAX_ATTACHMENTS,
  type SentAttachment,
  isImage,
} from "../api/attachments";
import { useApp } from "../app/context";

/** A file on a composer, before the message goes. */
export interface DraftFile {
  id: number;
  name: string;
  size: number;
  contentType: string;
  /** A local preview for images; revoked when the file leaves. */
  previewUrl: string | null;
  state: "uploading" | "ready" | "failed";
  /** What the daemon saved, once it's up. */
  attachment: Attachment | null;
  error: string | null;
}

export interface DraftFiles {
  files: DraftFile[];
  /** Picked, pasted or dropped files; each starts uploading. */
  add: (files: Iterable<File>) => void;
  remove: (id: number) => void;
  /** After sending: the files leave the composer. */
  clear: () => void;
  /** Something is still going up. */
  uploading: boolean;
  /** What a message can carry now, with the local previews. */
  ready: SentAttachment[];
}

let nextId = 1;

/**
 * The files on one composer. They upload to `target` as soon as they're
 * added; a target of null (no chief or workspace yet) refuses them.
 */
export function useDraftFiles(target: AttachTarget | null): DraftFiles {
  const { cos } = useApp();
  const [files, setFiles] = useState<DraftFile[]>([]);
  const live = useRef(new Set<number>());
  const targetKey = target ? JSON.stringify(target) : null;

  const update = useCallback((id: number, change: Partial<DraftFile>) => {
    if (!live.current.has(id)) return;
    setFiles((list) => list.map((f) => (f.id === id ? { ...f, ...change } : f)));
  }, []);

  // How many files the composer holds, for the cap; kept outside render.
  const count = useRef(0);
  useEffect(() => {
    count.current = files.length;
  }, [files.length]);

  const add = useCallback(
    (picked: Iterable<File>) => {
      if (!target) return;
      const room = Math.max(0, MAX_ATTACHMENTS - count.current);
      const added = [...picked].slice(0, room).map((file): DraftFile => {
        const id = nextId++;
        const contentType = file.type || "application/octet-stream";
        live.current.add(id);
        void cos.uploadAttachment(target, file).then(
          (attachment) => update(id, { state: "ready", attachment, name: attachment.name }),
          (error: unknown) =>
            update(id, {
              state: "failed",
              error: error instanceof Error ? error.message : String(error),
            }),
        );
        return {
          id,
          name: file.name || "Pasted image",
          size: file.size,
          contentType,
          previewUrl: isImage(contentType) ? objectUrl(file) : null,
          state: "uploading",
          attachment: null,
          error: null,
        };
      });
      count.current += added.length;
      setFiles((list) => [...list, ...added]);
    },
    // The target is compared by value.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [cos, targetKey, update],
  );

  const forget = (file: DraftFile) => {
    live.current.delete(file.id);
    if (file.previewUrl) URL.revokeObjectURL(file.previewUrl);
  };

  const remove = useCallback((id: number) => {
    setFiles((list) => {
      const file = list.find((f) => f.id === id);
      if (file) forget(file);
      return list.filter((f) => f.id !== id);
    });
  }, []);

  const clear = useCallback(() => {
    setFiles((list) => {
      // Previews stay alive: the sent message shows them until the
      // transcript has the files.
      for (const file of list) live.current.delete(file.id);
      return [];
    });
  }, []);

  // A new chief or workspace: what was picked for the old one goes.
  useEffect(
    () => () => {
      setFiles((list) => {
        list.forEach(forget);
        return [];
      });
    },
    [targetKey],
  );

  return {
    files,
    add,
    remove,
    clear,
    uploading: files.some((f) => f.state === "uploading"),
    ready: files.flatMap((f) =>
      f.state === "ready" && f.attachment ? [{ ...f.attachment, previewUrl: f.previewUrl }] : [],
    ),
  };
}

function objectUrl(file: File): string | null {
  try {
    return URL.createObjectURL(file);
  } catch {
    return null;
  }
}

/**
 * Drag and drop and paste for a composer: spread `handlers` on its box and
 * show the overlay while `dragging`.
 */
export function useFileDrop(onFiles: (files: File[]) => void, enabled = true) {
  const [dragging, setDragging] = useState(false);
  const depth = useRef(0);
  const hasFiles = (event: React.DragEvent) => [...event.dataTransfer.types].includes("Files");
  const handlers = {
    onDragEnter: (event: React.DragEvent) => {
      if (!enabled || !hasFiles(event)) return;
      event.preventDefault();
      depth.current += 1;
      setDragging(true);
    },
    onDragOver: (event: React.DragEvent) => {
      if (!enabled || !hasFiles(event)) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = "copy";
    },
    onDragLeave: () => {
      depth.current = Math.max(0, depth.current - 1);
      if (depth.current === 0) setDragging(false);
    },
    onDrop: (event: React.DragEvent) => {
      depth.current = 0;
      setDragging(false);
      if (!enabled) return;
      const files = [...event.dataTransfer.files];
      if (files.length === 0) return;
      event.preventDefault();
      onFiles(files);
    },
    onPaste: (event: React.ClipboardEvent) => {
      if (!enabled) return;
      const files = [...event.clipboardData.files];
      if (files.length === 0) return;
      event.preventDefault();
      onFiles(files.map((file, i) => (file.name ? file : named(file, i))));
    },
  };
  return { dragging, handlers };
}

/** A pasted screenshot has no name; give it one with its type's extension. */
function named(file: File, index: number): File {
  const extension = file.type.split("/")[1]?.replace("jpeg", "jpg") ?? "bin";
  return new File([file], `pasted-${index + 1}.${extension}`, { type: file.type });
}
