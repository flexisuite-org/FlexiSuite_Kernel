export type EventName =
  | 'group.created'
  | 'user.created'
  | 'entity.created'
  | 'entity.updated'
  | 'entity.deleted'
  | 'auth.mfa_challenged';

export interface EventPayloadMap {
  'group.created': { groupId: string; actorId: string };
  'user.created': { userId: string; email: string; actorId: string };
  'entity.created': { entityId: string; definitionId: string; groupId: string; actorId: string };
  'entity.updated': { entityId: string; definitionId: string; groupId: string; actorId: string };
  'entity.deleted': { entityId: string; definitionId: string; groupId: string; actorId: string };
  'auth.mfa_challenged': { userId: string; challengeId: string };
}

export interface EventMessage<K extends EventName = EventName> {
  id: string;
  name: K;
  payload: EventPayloadMap[K];
  occurredAt: Date;
  correlationId?: string;
}
