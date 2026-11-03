/** A server key's first and last four hex digits. */
export const shortKey = (hex: string) => `${hex.slice(0, 4)}…${hex.slice(-4)}`;
