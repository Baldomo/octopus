export const MIDDLE = 0.5;
export const CATCH_PX = 6;
export const RELEASE_PX = 12;

/** Whether the divider holds on the middle with the pointer `off` px from it, given whether it held a move ago. */
export const holds = (off: number, held: boolean) => Math.abs(off) <= (held ? RELEASE_PX : CATCH_PX);
