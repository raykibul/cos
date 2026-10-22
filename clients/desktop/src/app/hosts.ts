import { THIS_MAC } from "../bridge/types";
import { useApp } from "./context";

/**
 * Switches host from anywhere: the window goes back to the waiting page, and
 * the store starts over on the new host.
 */
export function useSwitchHost(): (id: string) => void {
  const { cos, ui } = useApp();
  return (id: string) => {
    if (id === (cos.get().host?.id ?? THIS_MAC)) return;
    ui.close();
    ui.navigate({ kind: "waiting" });
    void ui.perform("Couldn't switch Mac", () => cos.switchHost(id));
  };
}
