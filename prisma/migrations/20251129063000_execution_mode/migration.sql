-- Add execution mode to component policy
CREATE TYPE "ExecutionMode" AS ENUM ('API', 'SANDBOX');

ALTER TABLE "ComponentPolicy"
  ADD COLUMN "executionMode" "ExecutionMode" NOT NULL DEFAULT 'API';
