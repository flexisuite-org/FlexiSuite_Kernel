-- Add bundle integrity/signature
ALTER TABLE "ComponentPackage"
  ADD COLUMN "bundleIntegrity" TEXT,
  ADD COLUMN "bundleSignature" TEXT;

-- PlaygroundLog table
CREATE TABLE "PlaygroundLog" (
  "id" TEXT PRIMARY KEY DEFAULT gen_random_uuid(),
  "groupId" TEXT NOT NULL,
  "userId" TEXT,
  "payload" JSONB NOT NULL,
  "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
  CONSTRAINT "PlaygroundLog_groupId_fkey" FOREIGN KEY ("groupId") REFERENCES "Group"("id") ON DELETE RESTRICT ON UPDATE CASCADE,
  CONSTRAINT "PlaygroundLog_userId_fkey" FOREIGN KEY ("userId") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE
);

-- RLS for playground log
ALTER TABLE "PlaygroundLog" ENABLE ROW LEVEL SECURITY;
CREATE POLICY "playgroundlog_tenant" ON "PlaygroundLog"
  USING ("groupId" = current_setting('flexi.current_group'))
  WITH CHECK ("groupId" = current_setting('flexi.current_group'));
