/** What a failure says, whatever was thrown. */
export const said = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Runs `op` on each item in turn, saying why each failed one did; resolves to how many succeeded. */
export async function each<T>(items: T[], op: (item: T) => Promise<unknown>, problem: (text: string) => void) {
  let done = 0;
  for (const item of items) {
    try {
      await op(item);
      done++;
    } catch (error) {
      problem(said(error));
    }
  }
  return done;
}
