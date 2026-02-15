import { AsyncLocalStorage } from 'async_hooks';

export interface RequestContextValue {
  groupId?: string | null;
  userId?: string | null;
  mode?: 'draft' | 'stable';
}

export const requestContext = new AsyncLocalStorage<RequestContextValue>();

export function setRequestContext(value: RequestContextValue) {
  // enterWith keeps the store for the current async call chain
  requestContext.enterWith(value);
}

export function getRequestContext(): RequestContextValue | undefined {
  return requestContext.getStore();
}
