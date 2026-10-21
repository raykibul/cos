import { FolderSearch, SquareTerminal } from "lucide-react";
import type { CosStore } from "../store/store";
import type { MenuItem } from "../ui/Menu";

/** Reveal in Finder and Open in Terminal, through the shell. */
export function folderItems(cos: CosStore, path: string): MenuItem[] {
  return [
    {
      label: "Reveal in Finder",
      icon: FolderSearch,
      onSelect: () => void cos.bridge.revealPath(path).catch(() => undefined),
    },
    {
      label: "Open in Terminal",
      icon: SquareTerminal,
      onSelect: () => void cos.bridge.openInTerminal(path).catch(() => undefined),
    },
  ];
}
