import { Prisma } from '@prisma/client';

export function isDraftWriteNotAllowed(err: unknown): err is Error {
  return err instanceof Error && err.message === 'write_not_allowed_in_draft';
}

export function mapPrismaError(err: unknown) {
  if (isDraftWriteNotAllowed(err)) {
    return { status: 403, body: { error: 'write_not_allowed_in_draft' } };
  }
  if (err instanceof Prisma.PrismaClientKnownRequestError) {
    return { status: 400, body: { error: err.code, meta: err.meta } };
  }
  return null;
}
