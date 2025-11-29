-- CreateEnum
CREATE TYPE "ComponentStatus" AS ENUM ('DRAFT', 'APPROVED', 'REVOKED');

-- CreateEnum
CREATE TYPE "DependencyKind" AS ENUM ('RUNTIME', 'PEER', 'OPTIONAL');

-- CreateEnum
CREATE TYPE "ReleaseChannel" AS ENUM ('STABLE', 'DRAFT');

-- CreateTable
CREATE TABLE "ComponentPolicy" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "memoryMb" INTEGER NOT NULL DEFAULT 128,
    "timeoutMs" INTEGER NOT NULL DEFAULT 500,
    "allowNetwork" BOOLEAN NOT NULL DEFAULT false,
    "allowedModules" TEXT[] DEFAULT ARRAY[]::TEXT[],

    CONSTRAINT "ComponentPolicy_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "ComponentPackage" (
    "id" TEXT NOT NULL,
    "name" TEXT NOT NULL,
    "version" TEXT NOT NULL,
    "status" "ComponentStatus" NOT NULL DEFAULT 'DRAFT',
    "integrityHash" TEXT NOT NULL,
    "manifest" JSONB NOT NULL,
    "policyId" TEXT NOT NULL,
    "ownerGroupId" TEXT NOT NULL,
    "createdById" TEXT,
    "approvedAt" TIMESTAMP(3),
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "ComponentPackage_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "ComponentDependency" (
    "id" TEXT NOT NULL,
    "packageId" TEXT NOT NULL,
    "depName" TEXT NOT NULL,
    "depVersion" TEXT NOT NULL,
    "integrity" TEXT,
    "kind" "DependencyKind" NOT NULL DEFAULT 'RUNTIME',

    CONSTRAINT "ComponentDependency_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "ComponentInstall" (
    "id" TEXT NOT NULL,
    "packageId" TEXT NOT NULL,
    "groupId" TEXT NOT NULL,
    "channel" "ReleaseChannel" NOT NULL DEFAULT 'STABLE',
    "lockData" JSONB NOT NULL,
    "installedBy" TEXT,
    "installedAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,
    "rollbackOf" TEXT,

    CONSTRAINT "ComponentInstall_pkey" PRIMARY KEY ("id")
);

-- CreateTable
CREATE TABLE "RolloutRule" (
    "id" TEXT NOT NULL,
    "installId" TEXT NOT NULL,
    "percentage" INTEGER NOT NULL DEFAULT 100,
    "allowlist" JSONB,
    "blocklist" JSONB,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP,

    CONSTRAINT "RolloutRule_pkey" PRIMARY KEY ("id")
);

-- CreateIndex
CREATE INDEX "ComponentPackage_ownerGroupId_idx" ON "ComponentPackage"("ownerGroupId");

-- CreateIndex
CREATE UNIQUE INDEX "ComponentPackage_name_version_key" ON "ComponentPackage"("name", "version");

-- CreateIndex
CREATE INDEX "ComponentInstall_groupId_idx" ON "ComponentInstall"("groupId");

-- CreateIndex
CREATE UNIQUE INDEX "ComponentInstall_packageId_groupId_channel_key" ON "ComponentInstall"("packageId", "groupId", "channel");

-- CreateIndex
CREATE UNIQUE INDEX "RolloutRule_installId_key" ON "RolloutRule"("installId");

-- AddForeignKey
ALTER TABLE "ComponentPackage" ADD CONSTRAINT "ComponentPackage_policyId_fkey" FOREIGN KEY ("policyId") REFERENCES "ComponentPolicy"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentPackage" ADD CONSTRAINT "ComponentPackage_ownerGroupId_fkey" FOREIGN KEY ("ownerGroupId") REFERENCES "Group"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentPackage" ADD CONSTRAINT "ComponentPackage_createdById_fkey" FOREIGN KEY ("createdById") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentDependency" ADD CONSTRAINT "ComponentDependency_packageId_fkey" FOREIGN KEY ("packageId") REFERENCES "ComponentPackage"("id") ON DELETE CASCADE ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentInstall" ADD CONSTRAINT "ComponentInstall_packageId_fkey" FOREIGN KEY ("packageId") REFERENCES "ComponentPackage"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentInstall" ADD CONSTRAINT "ComponentInstall_groupId_fkey" FOREIGN KEY ("groupId") REFERENCES "Group"("id") ON DELETE RESTRICT ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "ComponentInstall" ADD CONSTRAINT "ComponentInstall_installedBy_fkey" FOREIGN KEY ("installedBy") REFERENCES "User"("id") ON DELETE SET NULL ON UPDATE CASCADE;

-- AddForeignKey
ALTER TABLE "RolloutRule" ADD CONSTRAINT "RolloutRule_installId_fkey" FOREIGN KEY ("installId") REFERENCES "ComponentInstall"("id") ON DELETE CASCADE ON UPDATE CASCADE;
