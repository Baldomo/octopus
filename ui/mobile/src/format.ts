/** A server key's first and last four hex digits. */
export const shortKey = (hex: string) => `${hex.slice(0, 4)}…${hex.slice(-4)}`;

/** A socket address's host, without its port or an IPv6 address's brackets. */
export const host = (address: string) => address.replace(/:\d+$/, "").replace(/^\[(.*)\]$/, "$1");
