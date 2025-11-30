-- CreateTable
CREATE TABLE "SandboxSession" (
    "id" TEXT NOT NULL,
    "sourceGroupId" TEXT NOT NULL,
    "sandboxGroupId" TEXT NOT NULL,
    "appId" TEXT,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "expiresAt" TIMESTAMP(3),

    CONSTRAINT "SandboxSession_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE UNIQUE INDEX "SandboxSession_sandboxGroupId_key" ON "SandboxSession"("sandboxGroupId");

-- AddForeignKey
ALTER TABLE "SandboxSession" ADD CONSTRAINT "SandboxSession_sourceGroupId_fkey" FOREIGN KEY ("sourceGroupId") REFERENCES "Group"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "SandboxSession" ADD CONSTRAINT "SandboxSession_sandboxGroupId_fkey" FOREIGN KEY ("sandboxGroupId") REFERENCES "Group"("id") ON DELETE RESTRICT ON UPDATE CASCADE;
