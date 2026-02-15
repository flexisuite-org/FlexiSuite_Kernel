import Ajv, { ValidateFunction } from 'ajv';

const ajv = new Ajv({ allErrors: true, useDefaults: true, removeAdditional: 'failing' });

const cache = new Map<string, ValidateFunction>();

export function getValidator(schemaId: string, schema: object): ValidateFunction {
  if (cache.has(schemaId)) return cache.get(schemaId)!;
  const validate = ajv.compile(schema);
  cache.set(schemaId, validate);
  return validate;
}
