/** Multi-tab-per-connection tab-key generator. Same shape as SSH mode:
 *  `${connectionId}#${Date.now()}-${counter}`. */

let counter = 0;

export function newExplorerTabKey(connectionId: string): string {
  counter += 1;
  return `${connectionId}#${Date.now()}-${counter}`;
}

export function connectionIdFromTabKey(tabKey: string): string {
  const i = tabKey.indexOf('#');
  return i >= 0 ? tabKey.slice(0, i) : tabKey;
}
