// Which sessions sit in columns beside the tab's own. Plain, tested without a browser.

export const NO_COLUMNS: readonly string[] = [];

/** The most columns beside the main one: past that each is too narrow to read. */
export const MAX_COLUMNS = 4;

/**
 * The columns with `session` added at the end: not twice, not the one the main column shows, and
 * the oldest giving way once there are `MAX_COLUMNS`.
 */
export function addColumn(
  ids: readonly string[],
  session: string,
  shown: string | undefined,
): readonly string[] {
  if (session === shown || ids.includes(session)) return ids;
  return [...ids, session].slice(-MAX_COLUMNS);
}

/** The address of a column's frame: the same page, opened on `session` as a column. */
export function columnSrc(pathname: string, session: string): string {
  return `${pathname}?session=${encodeURIComponent(session)}&column`;
}
