export type Result<T> = { ok: true; value: T } | { ok: false; error: unknown };

/**
 * A refetch that never runs twice at once. Requests made while one is in
 * flight fold into a single follow-up, so a burst of events costs at most
 * one in-flight request plus one more. An answer that a newer request
 * overtook is dropped, since it may be stale. With a delay, requests in a
 * burst fold into one before anything is fetched.
 */
export class CoalescedRefresh<T> {
  private running = false;
  private again = false;
  private disposed = false;

  constructor(
    private readonly fetch: () => Promise<T>,
    private readonly apply: (result: Result<T>) => void,
    private readonly delayMs = 0,
  ) {}

  /** Whether a fetch is in flight or waiting for its delay. */
  get busy(): boolean {
    return this.running;
  }

  request(): void {
    if (this.disposed) return;
    if (this.running) {
      this.again = true;
      return;
    }
    this.running = true;
    void this.loop();
  }

  /** Stops applying answers; requests after this do nothing. */
  dispose(): void {
    this.disposed = true;
  }

  private async loop(): Promise<void> {
    if (this.delayMs > 0) await new Promise((resolve) => setTimeout(resolve, this.delayMs));
    do {
      this.again = false;
      let result: Result<T>;
      try {
        result = { ok: true, value: await this.fetch() };
      } catch (error) {
        result = { ok: false, error };
      }
      if (this.disposed) break;
      if (!this.again) this.apply(result);
    } while (this.again && !this.disposed);
    this.running = false;
  }
}
