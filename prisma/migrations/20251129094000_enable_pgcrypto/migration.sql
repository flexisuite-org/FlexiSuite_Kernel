-- Ensure pgcrypto is available for gen_random_uuid used by cuid()
CREATE EXTENSION IF NOT EXISTS "pgcrypto";
