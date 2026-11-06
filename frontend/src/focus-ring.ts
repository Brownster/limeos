/** The part of a DOM node that wrapping needs; keeps the logic testable without a browser. */
export interface Ordered {
  compareDocumentPosition(other: never): number;
}
const PRECEDING = 2;
const FOLLOWING = 4;
/**
 * Where Tab or Shift+Tab must send focus to keep it inside a modal dialog, or
 * null when the browser's own movement stays inside. Besides the first and last
 * control this covers focus resting on a non-control (a heading focused after a
 * step change): the browser would otherwise leave the dialog from there.
 */
export function wrapTarget<T extends Ordered>(
  controls: T[],
  current: unknown,
  shift: boolean,
): T | null {
  const first = controls[0];
  const last = controls[controls.length - 1];
  if (!first || !last) return null;
  const index = controls.indexOf(current as T);
  if (index !== -1) {
    if (shift && current === first) return last;
    if (!shift && current === last) return first;
    return null;
  }
  const node = current as Ordered;
  if (!node || typeof node.compareDocumentPosition !== "function") return null;
  const beside = (control: T, bit: number) =>
    (node.compareDocumentPosition(control as never) & bit) !== 0;
  return shift
    ? ([...controls].reverse().find((c) => beside(c, PRECEDING)) ?? last)
    : (controls.find((c) => beside(c, FOLLOWING)) ?? first);
}
